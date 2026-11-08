//! Dev and release plumbing. `cargo bundle` is an alias for `bundle`.
//!
//! - `bundle [--release] [--engine DIR] [--no-sign] [--run [args…]]`: builds `baa` and puts the
//!   binary into a copy of the engine's `baa.app` (BaaCore.framework and helpers).
//! - `sign <file>`: Developer ID `codesign --timestamp` of a file (the DMG). The app itself is
//!   signed by `chromium/packaging/sign_chrome.py` (release.yml). Identity from `MACOS_SIGN_IDENTITY`.
//! - `notarize <file.dmg>`: notarytool + staple. Needs `APPLE_ID`, `APPLE_TEAM_ID`,
//!   `APPLE_APP_PASSWORD` (an app-specific password).

use anyhow::{bail, Context, Result};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args[..] {
        ["bundle", ref rest @ ..] => bundle(parse_bundle(rest)?),
        ["sign", path] => mac::sign(Path::new(path)),
        ["notarize", path] => mac::notarize(Path::new(path)),
        _ => bail!("usage: xtask bundle [--release] [--engine DIR] [--no-sign] [--run args…] | sign <file> | notarize <file>"),
    }
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

fn target_dir() -> PathBuf {
    env::var_os("CARGO_TARGET_DIR").map_or_else(|| workspace_root().join("target"), PathBuf::from)
}

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().with_context(|| format!("spawning {cmd:?}"))?;
    if !status.success() {
        bail!("{cmd:?} failed: {status}");
    }
    Ok(())
}

const FRAMEWORK: &str = "Contents/Frameworks/BaaCore.framework";
const SHARED_ENGINE: &str = "/Users/Shared/hb/helium-macos/build/src/out/Default";

struct Bundle {
    release: bool,
    engine: Option<PathBuf>,
    sign: bool,
    run: Option<Vec<String>>,
}

fn parse_bundle(args: &[&str]) -> Result<Bundle> {
    let mut b = Bundle { release: false, engine: None, sign: true, run: None };
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        match arg {
            "--release" => b.release = true,
            "--no-sign" => b.sign = false,
            "--engine" => b.engine = Some(it.next().context("--engine needs a directory")?.into()),
            // Everything after --run goes to baa (Chromium switches, URLs).
            "--run" => {
                b.run = Some(it.map(|a| a.to_string()).collect());
                break;
            }
            other => bail!("unknown bundle argument {other}"),
        }
    }
    Ok(b)
}

/// The engine directory (it holds `baa.app`): `--engine`, `$BAA_ENGINE`, the shared
/// local Chromium tree, `target/engine`.
fn find_engine(arg: Option<PathBuf>) -> Result<PathBuf> {
    let shared = PathBuf::from(SHARED_ENGINE);
    let candidates = [arg, env::var_os("BAA_ENGINE").map(PathBuf::from), Some(shared), Some(workspace_root().join("target/engine"))];
    for dir in candidates.into_iter().flatten() {
        if dir.join("baa.app").is_dir() {
            return Ok(dir);
        }
    }
    bail!(
        "no engine (a directory with baa.app) found. Get one:\n  \
         Rust-only work: gh run download <run> -n baa-engine-<tree> -D target/engine, then\n    \
         zstd -dc target/engine/engine.tar.zst | tar -xf - -C target/engine\n  \
         C++ work: unpack chromium-tree at /Users/Shared/hb and run chromium/local-build.sh\n\
         (README \"Build\"); then pass --engine DIR or set BAA_ENGINE"
    )
}

fn modified(path: &Path) -> Option<std::time::SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn bundle(b: Bundle) -> Result<()> {
    let engine = find_engine(b.engine)?;
    let mut build = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    build.current_dir(workspace_root()).args(["build", "-p", "baa"]);
    if b.release {
        build.arg("--release");
    }
    run(&mut build)?;

    // A component engine finds libchrome_dll.dylib through an rpath to its out
    // directory, so its bundle has to live there. An official engine is relocatable.
    let component = fs::read_to_string(engine.join("args.gn")).is_ok_and(|a| a.replace(' ', "").contains("is_component_build=true"));
    let source = engine.join("baa.app");
    let app = if component { engine.join("baa Dev.app") } else { target_dir().join("bundle/baa.app") };
    // Compare the framework binary: a directory's mtime moves for unrelated reasons.
    let core = |app: &Path| app.join(FRAMEWORK).join("BaaCore");
    let stale = !core(&app).exists() || modified(&core(&source)) > modified(&core(&app));
    // An official bundle is always fresh: old helpers and signatures must not stay.
    if stale || !component {
        if app.exists() {
            fs::remove_dir_all(&app)?;
        }
        fs::create_dir_all(app.parent().unwrap())?;
        run(Command::new("ditto").arg(&source).arg(&app))?;
    }

    let binary = target_dir().join(if b.release { "release" } else { "debug" }).join("baa");
    // A copy over a hard link (ditto of an engine) would write through to the engine.
    let dest = app.join("Contents/MacOS/baa");
    let _ = fs::remove_file(&dest);
    fs::copy(&binary, &dest).with_context(|| format!("copying {}", binary.display()))?;
    let plist = app.join("Contents/Info.plist");
    for key in ["CFBundleShortVersionString", "CFBundleVersion"] {
        run(Command::new("plutil").args(["-replace", key, "-string", env!("CARGO_PKG_VERSION")]).arg(&plist))?;
    }
    if b.sign {
        mac::dev_sign(&app)?;
    }
    println!("{}", app.display());
    match b.run {
        Some(args) => run(Command::new(app.join("Contents/MacOS/baa")).args(args)),
        None => Ok(()),
    }
}

mod mac {
    use super::run;
    use anyhow::{bail, Context, Result};
    use std::{env, path::Path, process::Command};

    /// Developer ID, hardened runtime, secure timestamp.
    pub fn sign(path: &Path) -> Result<()> {
        let identity = env::var("MACOS_SIGN_IDENTITY").context("MACOS_SIGN_IDENTITY not set")?;
        run(Command::new("codesign")
            .args(["--force", "--timestamp", "--options", "runtime", "--sign", &identity])
            .arg(path))
    }

    /// Outer app only. A stable identity keeps the Keychain's "Always Allow" for
    /// "baa Storage Key" across rebuilds (an ad-hoc signature's requirement is the
    /// binary's hash). `BAA_SIGN_IDENTITY` overrides; "-" signs ad hoc.
    pub fn dev_sign(app: &Path) -> Result<()> {
        let identity = env::var("BAA_SIGN_IDENTITY").unwrap_or_else(|_| apple_development().unwrap_or_else(|| "-".into()));
        run(Command::new("codesign")
            .args(["--force", "--sign", &identity, "--identifier", "me.baa.browser"])
            .arg(app))
    }

    fn apple_development() -> Option<String> {
        let out = Command::new("security").args(["find-identity", "-v", "-p", "codesigning"]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let start = text.find("\"Apple Development: ")? + 1;
        let len = text[start..].find('"')?;
        Some(text[start..start + len].to_string())
    }

    pub fn notarize(path: &Path) -> Result<()> {
        let var = |k: &str| env::var(k).with_context(|| format!("{k} not set"));
        let out = Command::new("xcrun")
            .args(["notarytool", "submit"])
            .arg(path)
            .args(["--apple-id", &var("APPLE_ID")?, "--team-id", &var("APPLE_TEAM_ID")?])
            .args(["--password", &var("APPLE_APP_PASSWORD")?, "--wait", "--output-format", "json"])
            .output()?;
        let json = String::from_utf8_lossy(&out.stdout);
        println!("{json}");
        // Gate on the reported status, not the exit code: an "Invalid" verdict is still a
        // completed submission.
        if !json.replace(' ', "").contains(r#""status":"Accepted""#) {
            bail!(
                "notarization not accepted ({}); see `xcrun notarytool log <id>`",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        run(Command::new("xcrun").args(["stapler", "staple"]).arg(path))
    }
}
