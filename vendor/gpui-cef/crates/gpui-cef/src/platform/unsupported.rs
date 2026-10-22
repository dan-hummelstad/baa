//! Stub for platforms other than macOS and Windows. It builds, but no webview
//! is shown.
//!
//! Linux should work on the CEF side, so pointing the `#[path]` in [`super`] at
//! the cef backend would probably be enough. That is untested, hence this stub.

use gpui::{div, App, Context, FocusHandle, IntoElement, ParentElement, Render, Styled, Window};

use crate::{RuntimeOptions, WebviewOptions};

/// Holds nothing. Present to match the other backends' API.
#[derive(Clone)]
pub struct Runtime;

impl Runtime {
    pub fn start(&self, _cx: &mut App) -> crate::Result<()> {
        Ok(())
    }

    /// Cookies need a browser store, which this platform does not have.
    pub fn set_cookies(&self, _cookies: &[crate::CookieSpec]) -> crate::Result<usize> {
        Err(crate::Error::Unsupported("cookies"))
    }

    pub fn shutdown(self) {}
}

/// Storage profiles are not wired up on this backend; every webview shares one.
#[derive(Clone, Debug)]
pub struct Profile;

impl Profile {
    // ponytail: options ignored. Ceiling: no per-Space isolation on this backend.
    pub fn new(_options: crate::ProfileOptions) -> crate::Result<Self> {
        Ok(Profile)
    }
}

pub fn init(_options: RuntimeOptions) -> crate::Result<Runtime> {
    Ok(Runtime)
}

impl gpui::EventEmitter<crate::WebviewEvent> for Webview {}

/// A webview that draws nothing, with the same API as the real backends.
pub struct Webview {
    options: WebviewOptions,
    focus_handle: FocusHandle,
    this: gpui::WeakEntity<Webview>,
}

impl Webview {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>, options: WebviewOptions) -> Self {
        Self {
            options,
            focus_handle: cx.focus_handle(),
            this: cx.weak_entity(),
        }
    }

    pub fn load_url(&self, _url: &str) {}
    pub fn reload(&self) {}
    pub fn go_back(&self) {}
    pub fn go_forward(&self) {}
    pub fn stop(&self) {}
    pub fn eval(&self, _script: &str) {}
    /// The current page title. Always empty.
    pub fn title(&self) -> String {
        String::new()
    }
    pub fn url(&self) -> String {
        self.options.url.clone()
    }
    /// This webview's own entity, for code that only holds `&Webview`.
    pub fn weak_entity(&self) -> gpui::WeakEntity<Self> {
        self.this.clone()
    }

    pub fn set_zoom(&self, _factor: f64) {}
    pub fn find(&self, _text: &str, _forward: bool) {}
    pub fn show_dev_tools(&self) {}
    // ponytail: no back/forward list on this backend yet.
    pub fn history(&self) -> (Vec<crate::HistoryEntry>, usize) {
        (Vec::new(), 0)
    }

    pub fn go_to_offset(&self, offset: i32) {
        self.eval(&format!("history.go({offset})"));
    }

    /// No-op: a native child window has square corners.
    pub fn set_corner_radius(&self, _radius: f32) {}

    /// No-op: child windows stack by creation.
    pub fn set_floating(&self, _floating: bool) {}

    pub fn set_visible(&self, _visible: bool) {}
    /// Whether a page is loading. Always false.
    pub fn is_loading(&self) -> bool {
        false
    }
    /// Whether there is history to go back to. Always false.
    pub fn can_go_back(&self) -> bool {
        false
    }
    /// Whether there is history to go forward to. Always false.
    pub fn can_go_forward(&self) -> bool {
        false
    }
}

impl gpui::Focusable for Webview {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Webview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(format!(
            "webview unsupported on this platform ({})",
            self.options.url
        ))
    }
}
