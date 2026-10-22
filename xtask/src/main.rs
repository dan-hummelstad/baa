//! Packaging for the app (PLAN.md §7, §13). `cargo bundle` is an alias for `bundle`.
//!
//! - `bundle [--release]`: builds `baa` and lays it out with CEF: `target/bundle/baa.app` on
//!   macOS (framework + 5 helper apps, a bare binary cannot start), `target/bundle/baa/` on Windows.
//! - `sign <baa.app|file>`: macOS Developer ID signing, inside-out, hardened runtime.
//!   Identity from `MACOS_SIGN_IDENTITY`. Windows is signed in release.yml (Azure Trusted Signing).
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
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["bundle"] => bundle(false),
        ["bundle", "--release"] => bundle(true),
        #[cfg(target_os = "macos")]
        ["sign", path] => mac::sign(Path::new(path)),
        #[cfg(target_os = "macos")]
        ["notarize", path] => mac::notarize(Path::new(path)),
        _ => bail!("usage: xtask bundle [--release] | sign <path> | notarize <path>  (sign/notarize: macOS only)"),
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

fn bundle(release: bool) -> Result<()> {
    let mut build = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    build.current_dir(workspace_root()).args(["build", "-p", "baa", "--bins"]);
    if release {
        build.arg("--release");
    }
    run(&mut build)?;
    let bins = target_dir().join(if release { "release" } else { "debug" });
    let out = target_dir().join("bundle");
    fs::create_dir_all(&out)?;
    println!("{}", platform_bundle(&out, &bins)?.display());
    Ok(())
}

/// `<app semver>+cef<X.Y.Z>`, so an engine-only release gets a distinct version (§13).
#[cfg(target_os = "macos")]
fn app_version() -> Result<semver::Version> {
    let cef = std::str::from_utf8(&cef::sys::CEF_VERSION[..])?.trim_end_matches('\0');
    let cef = cef.split('+').next().unwrap_or(cef);
    Ok(format!("{}+cef{cef}", env!("CARGO_PKG_VERSION")).parse()?)
}

#[cfg(target_os = "macos")]
fn platform_bundle(out: &Path, bins: &Path) -> Result<PathBuf> {
    use cef::build_util::mac::{bundle, BundleInfo};
    let app = out.join("baa.app");
    // A stale bundle would keep old signatures and helpers around.
    if app.exists() {
        fs::remove_dir_all(&app)?;
    }
    let info = BundleInfo::new("baa", "me.baa.browser", "baa", "en", app_version()?);
    bundle(out, bins, "baa", "baa_helper", None, info).map_err(|e| anyhow::anyhow!("bundle failed: {e:?}"))
}

/// cef-rs's own Windows bundler expects the sandbox's dll + bootstrap.exe layout; baa is a plain
/// exe that re-launches itself as the subprocess (§7), so this copies the runtime next to it.
#[cfg(target_os = "windows")]
fn platform_bundle(out: &Path, bins: &Path) -> Result<PathBuf> {
    let dir = out.join("baa");
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    let cef = cef::sys::get_cef_dir().context("CEF distribution not found; set CEF_PATH")?;
    let runtime = |name: &str| {
        name.ends_with("_icd.json") || [".dll", ".pak", ".dat", ".bin"].iter().any(|ext| name.ends_with(ext))
    };
    for (src, dst) in [(cef.clone(), dir.clone()), (cef.join("locales"), dir.join("locales"))] {
        fs::create_dir_all(&dst)?;
        for entry in fs::read_dir(&src)? {
            let entry = entry?;
            if entry.file_type()?.is_file() && runtime(&entry.file_name().to_string_lossy()) {
                fs::copy(entry.path(), dst.join(entry.file_name()))?;
            }
        }
    }
    fs::copy(bins.join("baa.exe"), dir.join("baa.exe"))?;
    // Without the compatibility section Chromium sees Windows 8 and misdetects features.
    fs::write(dir.join("baa.exe.manifest"), WIN_MANIFEST)?;
    Ok(dir)
}

#[cfg(target_os = "windows")]
const WIN_MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
      <maxversiontested Id="10.0.18362.0"/>
    </application>
  </compatibility>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="Win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges><requestedExecutionLevel level="asInvoker"/></requestedPrivileges></security>
  </trustInfo>
</assembly>
"#;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_bundle(_: &Path, _: &Path) -> Result<PathBuf> {
    bail!("baa ships for macOS and Windows only")
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{run, workspace_root};
    use anyhow::{bail, Context, Result};
    use std::{env, fs, path::Path, process::Command};

    fn codesign(path: &Path, entitlements: Option<&Path>) -> Result<()> {
        let identity = env::var("MACOS_SIGN_IDENTITY").context("MACOS_SIGN_IDENTITY not set")?;
        let mut cmd = Command::new("codesign");
        cmd.args(["--force", "--timestamp", "--options", "runtime", "--sign", &identity]);
        if let Some(entitlements) = entitlements {
            cmd.arg("--entitlements").arg(entitlements);
        }
        run(cmd.arg(path))
    }

    fn children_with_ext(dir: &Path, ext: &str) -> Result<Vec<std::path::PathBuf>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == ext) {
                out.push(path);
            }
        }
        Ok(out)
    }

    /// Nested code must be signed before its container, or the outer seal is invalid.
    pub fn sign(path: &Path) -> Result<()> {
        if path.extension().is_none_or(|e| e != "app") {
            return codesign(path, None);
        }
        let entitlements = workspace_root().join("xtask/entitlements.plist");
        let frameworks = path.join("Contents/Frameworks");
        let cef = frameworks.join("Chromium Embedded Framework.framework");
        for dylib in children_with_ext(&cef.join("Libraries"), "dylib")? {
            codesign(&dylib, None)?;
        }
        codesign(&cef, None)?;
        for helper in children_with_ext(&frameworks, "app")? {
            codesign(&helper, Some(&entitlements))?;
        }
        codesign(path, Some(&entitlements))?;
        run(Command::new("codesign").args(["--verify", "--deep", "--strict", "--verbose=2"]).arg(path))
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
