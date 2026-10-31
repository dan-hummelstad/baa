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
    pub fn set_dark(&self, _dark: bool) -> bool {
        false
    }
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
    pub fn answer_permission(&self, _id: u64, _allow: bool) {}
    pub fn answer_dialog(&self, _ok: bool, _text: &str) {}
    pub fn print(&self) {}
    pub fn set_muted(&self, _muted: bool) {}
    pub fn is_secure(&self) -> bool {
        self.url().starts_with("https:")
    }
    pub fn eval_with_gesture(&self, script: &str) {
        self.eval(script)
    }

    pub fn weak_entity(&self) -> gpui::WeakEntity<Self> {
        self.this.clone()
    }

    pub fn set_zoom(&self, _factor: f64) {}
    pub fn find(&self, _text: &str, _forward: bool) {}
    pub fn show_dev_tools(&self, _inspect_at: Option<(i32, i32)>) {}

    /// DevTools docking is macOS-only so far: an empty page.
    pub fn new_dev_tools(window: &mut Window, cx: &mut Context<Self>, _of: &gpui::Entity<Webview>, _inspect_at: Option<(i32, i32)>, _side: &str) -> Self {
        Self::new(window, cx, WebviewOptions::default())
    }
    // ponytail: no back/forward list on this backend yet.
    pub fn history(&self) -> (Vec<crate::HistoryEntry>, usize) {
        (Vec::new(), 0)
    }

    pub fn go_to_offset(&self, offset: i32) {
        self.eval(&format!("history.go({offset})"));
    }

    /// No-op: a native child window has square corners.
    pub fn set_corner_radius(&self, _radius: f32) {}
    pub fn set_square_corners(&self, _square: u8) {}

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

/// ponytail: no profile-scoped fetch on this backend yet.
pub fn fetch(_profile: Option<&Profile>, _url: &str, done: impl FnOnce(Option<String>) + 'static) {
    done(None)
}

// ponytail: downloads are not tracked on this backend.
pub fn cancel_download(_id: u32) {}

impl Profile {
    pub fn permission(&self, _url: &str, _kind: crate::Permission) -> crate::PermissionState {
        crate::PermissionState::Ask
    }
    pub fn set_permission(&self, _url: &str, _kind: crate::Permission, _state: crate::PermissionState) {}
}

/// No Chrome-style pages without CEF.
#[derive(Clone)]
pub struct ChromePage;

impl ChromePage {
    pub fn open(_profile: Option<&Profile>, _url: &str, _init_script: Option<String>) -> Option<Self> {
        None
    }
    pub fn load_url(&self, _url: &str) {}
    pub fn show_over(&self, _window: &gpui::Window, _bounds: gpui::Bounds<gpui::Pixels>, _corner_radius: f32) -> bool {
        false
    }
    pub fn hide(&self) {}
    pub fn eval(&self, _expression: &str, done: impl FnOnce(Option<String>) + 'static) {
        done(None)
    }
}

pub fn on_chrome_tab(_open: impl Fn(String) + 'static) {}

/// Native context menus are macOS-only so far: dismissed at once.
pub fn show_menu(
    window: &mut gpui::Window,
    cx: &mut gpui::App,
    _at: gpui::Point<gpui::Pixels>,
    _items: Vec<crate::MenuItem>,
    done: impl FnOnce(Option<usize>, &mut gpui::Window, &mut gpui::App) + 'static,
) {
    window.defer(cx, move |window, cx| done(None, window, cx));
}
