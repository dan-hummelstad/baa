//! `cargo run -p xtask -- bundle` assembles the macOS `.app` (CEF needs the
//! framework + helper layout; a bare binary cannot start).

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("bundle: only needed on macOS; Windows runs from target/ directly");
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    use cef::build_util::mac::{build_bundle, BundleInfo};
    let out = std::path::PathBuf::from("target/bundle");
    std::fs::create_dir_all(&out)?;
    let info = BundleInfo::new("baa", "me.baa.browser", "baa", "en", semver::Version::new(0, 1, 0));
    let app = build_bundle(&out, "baa", info).map_err(|e| anyhow::anyhow!("bundle failed: {e:?}"))?;
    println!("{}", app.display());
    Ok(())
}
