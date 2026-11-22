#![allow(clippy::disallowed_methods, reason = "build scripts are exempt")]

//! Builds swift/categorize.swift, the bridge to Apple's on-device model (PLAN-mail.md §17), and
//! sets `cfg(baa_model)`. Without an SDK that has FoundationModels, baa-mail builds without the
//! model, unless BAA_REQUIRE_MODEL is set: release sets it, so it never ships without.
//! BAA_SWIFT_XCODE names the Xcode to take swiftc and the SDK from when the default one is older,
//! as on CI's macos-15 image.

use std::{env, path::PathBuf, process::Command};

/// baa's Google client for `oauth.rs`: from the build's environment (CI's secrets), else from
/// the repository's `.env`.
fn google_client() {
    let dotenv = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../.env");
    // Only when it exists: Cargo reruns a script whose watched file is missing on every build.
    if dotenv.exists() {
        println!("cargo:rerun-if-changed={}", dotenv.display());
    }
    let file = std::fs::read_to_string(&dotenv).unwrap_or_default();
    for key in ["BAA_GOOGLE_CLIENT_ID", "BAA_GOOGLE_CLIENT_SECRET"] {
        println!("cargo:rerun-if-env-changed={key}");
        let from_file = file.lines().rev().find_map(|l| l.trim().strip_prefix(key)?.trim_start().strip_prefix('='));
        let value = env::var(key).ok().filter(|v| !v.is_empty()).or(from_file.map(|v| v.trim().trim_matches('"').to_owned()));
        if let Some(value) = value {
            println!("cargo:rustc-env={key}={value}");
        }
    }
}

fn main() {
    google_client();
    println!("cargo:rustc-check-cfg=cfg(baa_model)");
    println!("cargo:rerun-if-changed=swift/categorize.swift");
    println!("cargo:rerun-if-env-changed=BAA_SWIFT_XCODE");
    println!("cargo:rerun-if-env-changed=BAA_REQUIRE_MODEL");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let required = env::var_os("BAA_REQUIRE_MODEL").is_some();
    let xcode = env::var("BAA_SWIFT_XCODE").ok().map(|x| {
        let x = PathBuf::from(x);
        if x.ends_with("Developer") { x } else { x.join("Contents/Developer") }
    });
    let xcrun = |args: &[&str]| {
        let mut c = Command::new("xcrun");
        if let Some(x) = &xcode {
            c.env("DEVELOPER_DIR", x);
        }
        c.args(args);
        c
    };
    let out = |args: &[&str]| {
        xcrun(args).output().ok().filter(|o| o.status.success()).map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
    };
    let sdk = out(&["--sdk", "macosx", "--show-sdk-path"]);
    let Some(sdk) = sdk.filter(|s| s.join("System/Library/Frameworks/FoundationModels.framework").exists()) else {
        if required {
            panic!("BAA_REQUIRE_MODEL: no macOS SDK with FoundationModels (set BAA_SWIFT_XCODE to an Xcode 26 or later)");
        }
        println!("cargo:warning=baa-mail builds without Apple's on-device model: the macOS SDK has no FoundationModels");
        return;
    };
    let swiftc = out(&["--find", "swiftc"]).expect("swiftc in the Xcode that has the SDK");
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok(other) => other,
        Err(_) => "arm64",
    }
    .to_owned();
    // The Rust link's own target: with any other, the linker warns, or puts Swift's
    // libraries behind an rpath that baa's binary does not have.
    let target = env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "11.0".into());
    let dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let mut swift = Command::new(&swiftc);
    if let Some(x) = &xcode {
        swift.env("DEVELOPER_DIR", x);
    }
    let status = swift
        .args(["-parse-as-library", "-emit-library", "-static", "-O", "-module-name", "BaaCategorize"])
        .arg("-target")
        .arg(format!("{arch}-apple-macos{target}"))
        .arg("-sdk")
        .arg(&sdk)
        .arg("swift/categorize.swift")
        .arg("-o")
        .arg(dir.join("libbaacategorize.a"))
        .status()
        .expect("swiftc runs");
    assert!(status.success(), "swiftc failed on swift/categorize.swift");
    let toolchain = swiftc.parent().and_then(|bin| bin.parent()).expect("swiftc is in usr/bin").join("lib/swift/macosx");
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=static=baacategorize");
    // The SDK's Swift first, so the system's libraries are linked by their /usr/lib/swift path;
    // the toolchain's directory then gives the compatibility shims only it has.
    println!("cargo:rustc-link-search=native={}", sdk.join("usr/lib/swift").display());
    println!("cargo:rustc-link-search=native={}", toolchain.display());
    println!("cargo:rustc-link-search=framework={}", sdk.join("System/Library/Frameworks").display());
    // Concurrency, before macOS 12, is linked through @rpath: baa's binary has this one too.
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    println!("cargo:rustc-cfg=baa_model");
}
