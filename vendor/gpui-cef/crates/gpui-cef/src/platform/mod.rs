//! Backend selection. `init` / `Runtime` / `Webview` are swapped per platform.

#[cfg(target_os = "macos")]
#[path = "cef/mod.rs"]
mod imp;

#[cfg(target_os = "windows")]
#[path = "wry/mod.rs"]
mod imp;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[path = "unsupported.rs"]
mod imp;

pub use imp::{fetch, init, Profile, Runtime, Webview};
#[cfg(target_os = "macos")]
pub use imp::open_chrome_window;
