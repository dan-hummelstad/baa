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

const SPARKLE_FEED: &str = "https://github.com/dan-hummelstad/baa/releases/latest/download/appcast.xml";
/// The public half of the release signing key (also in chromium/helium-macos/devutils/shared.sh);
/// the private half is the SPARKLE_ED_PRIVATE_KEY secret.
const SPARKLE_PUBLIC_KEY: &str = "gW1tepVcmsDlemYLiwxqNJf0teQBBpPAYljw+bpefMI=";

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

const BUNDLE_ID: &str = "me.baa.browser";
const DEV_BUNDLE_ID: &str = "me.baa.browser.dev";
const FRAMEWORK: &str = "Contents/Frameworks/BaaCore.framework";
const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
const SHARED_ROOT: &str = "/Users/Shared/hb";
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
/// local Chromium tree if only you control it, `target/engine`.
fn find_engine(arg: Option<PathBuf>) -> Result<PathBuf> {
    let explicit = arg.or_else(|| env::var_os("BAA_ENGINE").map(PathBuf::from));
    if let Some(dir) = explicit.filter(|d| d.join("baa.app").is_dir()) {
        return Ok(dir);
    }
    // Any account can create /Users/Shared/hb first, and the bundle gets signed with
    // your identity and run: use it unasked only when it is yours alone.
    let shared = Path::new(SHARED_ENGINE);
    if shared.join("baa.app").is_dir() {
        if only_mine(Path::new(SHARED_ROOT), &shared.join("baa.app")) {
            return Ok(shared.to_path_buf());
        }
        eprintln!(
            "warning: ignoring {SHARED_ENGINE}: a directory from {SHARED_ROOT} down to baa.app is a symlink, \
             not yours, or writable by group/others. Fix it, or pass --engine / BAA_ENGINE to use it anyway."
        );
    }
    let local = workspace_root().join("target/engine");
    if local.join("baa.app").is_dir() {
        return Ok(local);
    }
    bail!(
        "no engine (a directory with baa.app) found. Get one:\n  \
         Rust-only work: gh run download <run> -n baa-engine-<tree> -D target/engine, then\n    \
         zstd -dc target/engine/engine.tar.zst | tar -xf - -C target/engine\n  \
         C++ work: unpack chromium-tree at /Users/Shared/hb and run chromium/local-build.sh\n\
         (README \"Build\"); then pass --engine DIR or set BAA_ENGINE"
    )
}

/// True when `path` lies under `top`, has no symlink in it, and every directory from `top`
/// down to `path` is owned by this user and not writable by group or others.
fn only_mine(top: &Path, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    unsafe extern "C" {
        safe fn getuid() -> u32;
    }
    let Ok(rest) = path.strip_prefix(top) else { return false };
    if fs::canonicalize(path).ok().as_deref() != Some(path) {
        return false;
    }
    let mut dir = top.to_path_buf();
    std::iter::once(None).chain(rest.components().map(Some)).all(|c| {
        dir.extend(c);
        fs::symlink_metadata(&dir).is_ok_and(|m| m.uid() == getuid() && m.mode() & 0o022 == 0)
    })
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
    // A debug baa is "baa Dev" with its own data dir (crates/baa data_dir), so it runs
    // beside an installed baa; the id keeps macOS (Dock, TCC, Keychain) from mixing them.
    let (id, name) = if b.release { (BUNDLE_ID, "baa") } else { (DEV_BUNDLE_ID, "baa Dev") };
    for (key, value) in [("CFBundleIdentifier", id), ("CFBundleName", name), ("CFBundleDisplayName", name)] {
        run(Command::new("plutil").args(["-replace", key, "-string", value]).arg(&plist))?;
    }
    // Sparkle, in release engines only (crates/baa starts it): the feed release.yml publishes, and
    // the key its updates must be signed with. Checks run daily without asking first.
    if !component {
        for (key, kind, value) in [
            ("SUFeedURL", "-string", SPARKLE_FEED),
            ("SUPublicEDKey", "-string", SPARKLE_PUBLIC_KEY),
            ("SUEnableAutomaticChecks", "-bool", "YES"),
        ] {
            run(Command::new("plutil").args(["-replace", key, kind, value]).arg(&plist))?;
        }
    }
    set_icon(&app)?;
    // LaunchServices keeps a bundle's icon by path and rereads Info.plist only when the
    // bundle's own mtime moves, which edits inside it (or a reused component bundle) never do.
    run(Command::new("touch").arg(&app))?;
    run(Command::new(LSREGISTER).arg("-f").arg(&app))?;
    // The Alerts helper draws notifications with its own icon. Only an unsigned bundle
    // gets it: dev_sign seals the outer app alone, so editing a helper would break its
    // signature; release re-signs every helper afterwards.
    if !b.sign {
        for helper in fs::read_dir(app.join(FRAMEWORK).join("Helpers"))? {
            let helper = helper?.path();
            if helper.to_string_lossy().contains("(Alerts)") {
                set_icon(&helper)?;
            }
        }
    }
    if b.sign {
        mac::dev_sign(&app, id)?;
    }
    println!("{}", app.display());
    match b.run {
        Some(args) => run(Command::new(app.join("Contents/MacOS/baa")).args(args)),
        None => Ok(()),
    }
}

/// baa's icon in place of Chromium's. CFBundleIconName points macOS at Assets.car,
/// which still holds Chromium's art, so it goes and CFBundleIconFile (app.icns) wins.
fn set_icon(app: &Path) -> Result<()> {
    let icns = app.join("Contents/Resources/app.icns");
    // Unlink first: a hard link from ditto would write through to the engine.
    let _ = fs::remove_file(&icns);
    fs::copy(workspace_root().join("assets/baa.icns"), &icns)?;
    let plist = app.join("Contents/Info.plist");
    // Absent key is fine: plutil fails and there is nothing to remove.
    let _ = Command::new("plutil").args(["-remove", "CFBundleIconName"]).arg(&plist).output();
    run(Command::new("plutil").args(["-replace", "CFBundleIconFile", "-string", "app.icns"]).arg(&plist))
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
    pub fn dev_sign(app: &Path, id: &str) -> Result<()> {
        let identity = env::var("BAA_SIGN_IDENTITY").unwrap_or_else(|_| apple_development().unwrap_or_else(|| "-".into()));
        run(Command::new("codesign")
            .args(["--force", "--sign", &identity, "--identifier", id])
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn only_mine_rejects_group_or_world_writable_and_symlinked_trees() {
        let top = env::temp_dir().join(format!("xtask-only-mine-{}", std::process::id()));
        let _ = fs::remove_dir_all(&top);
        let app = top.join("out/baa.app");
        fs::create_dir_all(&app).unwrap();
        let top = fs::canonicalize(&top).unwrap();
        let app = top.join("out/baa.app");
        let mode = |p: &Path, m: u32| fs::set_permissions(p, fs::Permissions::from_mode(m)).unwrap();
        for p in [&top, &top.join("out"), &app] {
            mode(p, 0o755);
        }
        assert!(only_mine(&top, &app));
        mode(&top.join("out"), 0o777);
        assert!(!only_mine(&top, &app));
        mode(&top.join("out"), 0o775);
        assert!(!only_mine(&top, &app));
        mode(&top.join("out"), 0o755);
        mode(&top, 0o777);
        assert!(!only_mine(&top, &app));
        mode(&top, 0o755);
        std::os::unix::fs::symlink(&app, top.join("link")).unwrap();
        assert!(!only_mine(&top, &top.join("link")));
        assert!(!only_mine(&top.join("out"), &top));
        fs::remove_dir_all(&top).unwrap();
    }
}
