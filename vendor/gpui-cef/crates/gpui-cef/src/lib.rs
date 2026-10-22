//! Embed a real browser engine as a [gpui] element.
//!
//! The backend is chosen per platform:
//!
//! | OS      | Engine                        | How it is composited |
//! |---------|-------------------------------|----------------------|
//! | macOS   | Chromium (CEF)                | A native view under gpui's layer, shown through a hole the element punches, so gpui still draws over it. Chromium renders and scrolls at the display's refresh rate. Needs the vendored `gpui_apple` (punch surfaces) and a window with `WindowBackgroundAppearance::Transparent`; gpui elements over a page must `.occlude()` to keep the mouse. |
//! | Windows | WebView2 (wry)                | A native child window parented to the gpui window and kept in sync with the laid-out rect. Being a child HWND, it is **always in front of** everything gpui draws. |
//!
//! Why not off-screen rendering on macOS: Chromium caps an off-screen page's
//! input-driven frames at 60 Hz (see `platform/cef/native.rs`).
//!
//! # Usage
//!
//! On macOS, CEF has to be brought up around gpui rather than inside it — see
//! [`init`] and [`Runtime::start`].
//!
//! ```no_run
//! fn main() {
//!     // 1. At the top of main: load CEF into the process.
//!     let runtime = gpui_cef::init(gpui_cef::RuntimeOptions::default()).unwrap();
//!
//!     // Hand the closure a *clone*. Moving the value itself in would run
//!     // cef_shutdown() the moment the closure returns.
//!     let cef = runtime.clone();
//!     gpui_platform::application().run(move |cx| {
//!         // 2. Once gpui has created NSApp, initialize CEF.
//!         cef.start(cx).unwrap();
//!         // ... cx.open_window(..., |window, cx| cx.new(|cx| Webview::new(...)))
//!     });
//!
//!     // 3. The message loop is done, so shut CEF down.
//!     runtime.shutdown();
//! }
//! ```

// The usage example above is deliberately a full `fn main`, since the ordering
// around it is the whole point.
#![allow(clippy::needless_doctest_main)]

mod options;
mod platform;

pub use options::{
    CookieSpec, FilterHook, ProfileOptions, Request, RuntimeOptions, SameSite, WebviewOptions,
};
pub use platform::{init, Profile, Runtime, Webview};
#[cfg(target_os = "macos")]
pub use platform::open_chrome_window;

/// What a [`Webview`] reports to the application, through
/// [`gpui::EventEmitter`].
#[derive(Debug, Clone, PartialEq)]
pub enum WebviewEvent {
    /// A message the page posted with the script [`post_message_script`]
    /// returns.
    ///
    /// Untrusted: any script running in the page can post one, so treat it as
    /// input from the page, not from the application.
    Message(String),
    UrlChanged(String),
    TitleChanged(String),
    /// The page's first favicon URL; `None` when it declares none.
    FaviconChanged(Option<String>),
    LoadingChanged(bool),
    HistoryChanged {
        can_go_back: bool,
        can_go_forward: bool,
    },
    /// `window.open`, `target=_blank`, or a modified/middle click on a link.
    /// The engine does not open anything itself.
    OpenRequested {
        url: String,
        from_user_gesture: bool,
    },
    /// Right click, at window coordinates. The engine shows no menu of its own.
    ContextMenu {
        x: f32,
        y: f32,
        link_url: Option<String>,
        selection: Option<String>,
    },
    /// A download started or progressed. Files go straight to the Downloads
    /// folder, without a dialog.
    Download(Download),
    /// The browser closed itself (`window.close()`).
    Closed,
}

/// A snapshot of one download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    /// Stable across updates of the same download for the life of the process.
    pub id: u32,
    pub url: String,
    /// Empty until the destination is chosen.
    pub path: String,
    pub received_bytes: i64,
    /// `-1` while unknown.
    pub total_bytes: i64,
    pub state: DownloadState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadState {
    InProgress,
    Complete,
    Canceled,
}

/// What a console line starts with when it is a message for the application.
/// The page never needs to spell it out: [`post_message_script`] does.
pub const MESSAGE_PREFIX: &str = "gpui-cef:message:";

/// JavaScript that evaluates `expression`, turns it into a string, and posts
/// it to the application as a [`WebviewEvent::Message`]. Pass it to
/// [`Webview::eval`], or embed it in a script that does.
///
/// Messages travel through the page's console, which both backends can
/// observe without code in the page's own process.
pub fn post_message_script(expression: &str) -> String {
    format!("console.debug({MESSAGE_PREFIX:?} + String({expression}))")
}

/// The message a console line carries, if it is one.
pub fn message_payload(line: &str) -> Option<&str> {
    line.strip_prefix(MESSAGE_PREFIX)
}

/// Something went wrong bringing up the browser engine.
#[derive(Debug)]
pub enum Error {
    /// The Chromium Embedded Framework could not be loaded. On macOS the
    /// executable has to live inside a bundled `.app`.
    LibraryLoad(String),
    /// `cef_initialize` failed.
    Initialize,
    /// The backend cannot do this on this platform.
    Unsupported(&'static str),
    /// The webview backend could not be created.
    Backend(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::LibraryLoad(path) => write!(
                f,
                "failed to load the Chromium Embedded Framework (looked next to {path}). \
                 the executable must live inside a bundled .app — see `just bundle`"
            ),
            Error::Initialize => write!(f, "cef_initialize() failed"),
            Error::Unsupported(what) => write!(f, "not supported on this platform: {what}"),
            Error::Backend(msg) => write!(f, "failed to create the webview backend: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_posted_message_reads_back_from_its_console_line() {
        let script = post_message_script("document.title");
        assert_eq!(
            script,
            r#"console.debug("gpui-cef:message:" + String(document.title))"#
        );
        assert_eq!(
            message_payload(r#"gpui-cef:message:{"a":1}"#),
            Some(r#"{"a":1}"#)
        );
        assert_eq!(message_payload("an ordinary log line"), None);
    }
}
