fn main() {
    // baa-mail's Swift bridge (PLAN-mail.md §17) links Swift Concurrency through @rpath when
    // built for macOS before 12; the system's copy is in /usr/lib/swift.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,/usr/lib/swift");
    }
}
