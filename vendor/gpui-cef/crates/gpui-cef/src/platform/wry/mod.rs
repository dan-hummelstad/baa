//! Windows backend: WebView2 as a native child window, via wry.
//!
//! Unlike the CEF backend on macOS, WebView2 exposes no public off-screen
//! rendering API. So this parents a **native child window** to the gpui window
//! and keeps it aligned with whatever rect gpui laid out.
//!
//! What that costs, relative to the CEF backend:
//!
//! - The child window is always in front of everything gpui draws. Nothing can
//!   be layered on top of the webview.
//! - No clipping or rounded corners. It is a rectangle.
//! - It catches up a frame late, so during fast scrolling it can visibly lag
//!   behind the rest of the window.
//!
//! Input goes straight to WebView2, so none of the CEF backend's event
//! forwarding is needed.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gpui::{
    div, prelude::*, App, Bounds, Context, Element, ElementId, GlobalElementId, InspectorElementId,
    LayoutId, Pixels, Render, Style, Window,
};
use wry::{
    dpi::{LogicalPosition, LogicalSize},
    PageLoadEvent, Rect, WebView, WebViewBuilder,
};

use crate::{Error, RuntimeOptions, WebviewOptions};

/// The WebView2 runtime needs no process-wide setup, so [`Runtime`] holds
/// nothing. It exists to match the macOS API.
#[derive(Clone)]
pub struct Runtime;

impl Runtime {
    /// Does nothing. Present so the same call sequence works on both platforms.
    pub fn start(&self, _cx: &mut App) -> crate::Result<()> {
        Ok(())
    }

    /// Not yet: WebView2 keeps cookies in its own profile, which this backend
    /// does not reach into.
    pub fn set_cookies(&self, _cookies: &[crate::CookieSpec]) -> crate::Result<usize> {
        Err(Error::Unsupported("setting cookies with WebView2"))
    }

    /// Does nothing: WebView2 needs no process-wide teardown.
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

/// Does nothing. Present so the same call sequence works on both platforms.
pub fn init(_options: RuntimeOptions) -> crate::Result<Runtime> {
    Ok(Runtime)
}

/// A webview, as a gpui entity.
pub struct Webview {
    webview: Option<WebView>,
    focus_handle: gpui::FocusHandle,
    /// The last rect pushed to the child window; `set_bounds` is only called
    /// when it changes.
    last_bounds: Option<Bounds<Pixels>>,
    url: String,
    /// Whether a page is loading, updated from wry's page load callback.
    is_loading: Rc<Cell<bool>>,
    /// Messages that arrived while gpui was busy, emitted on the next render.
    inbox: Rc<RefCell<Vec<String>>>,
    this: gpui::WeakEntity<Webview>,
}

impl Webview {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, options: WebviewOptions) -> Self {
        let is_loading = Rc::new(Cell::new(true));
        let inbox: Rc<RefCell<Vec<String>>> = Rc::default();
        let entity = cx.weak_entity();
        let app = cx.to_async();

        let mut builder = WebViewBuilder::new();
        if let Some(script) = &options.init_script {
            builder = builder.with_initialization_script(script);
        }
        let builder = builder
            .with_url(&options.url)
            .with_transparent(options.transparent)
            // The page posts through its console, as on macOS; this hands the
            // prefixed lines to wry's IPC channel instead.
            .with_initialization_script(console_bridge_script())
            .with_ipc_handler({
                let inbox = inbox.clone();
                move |request| {
                    let message = request.body().clone();
                    // WebView2 calls back from the message loop, where gpui is
                    // normally not mid-update; when it is, the message waits for
                    // the next render.
                    let mut app = app.clone();
                    let emitted = entity.update(&mut app, |_, cx| {
                        cx.emit(crate::WebviewEvent::Message(message.clone()));
                    });
                    if emitted.is_err() {
                        inbox.borrow_mut().push(message);
                    }
                }
            })
            .with_on_page_load_handler({
                let is_loading = is_loading.clone();
                move |event, _url| {
                    is_loading.set(matches!(event, PageLoadEvent::Started));
                }
            })
            .with_bounds(Rect {
                position: LogicalPosition::new(0., 0.).into(),
                size: LogicalSize::new(
                    f32::from(window.viewport_size().width) as f64,
                    f32::from(window.viewport_size().height) as f64,
                )
                .into(),
            });

        let webview = match builder.build_as_child(&window) {
            Ok(webview) => Some(webview),
            Err(err) => {
                log::error!("{}", Error::Backend(err.to_string()));
                None
            }
        };

        Self {
            webview,
            focus_handle: cx.focus_handle(),
            last_bounds: None,
            url: options.url,
            is_loading,
            inbox,
            this: cx.weak_entity(),
        }
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
    pub fn show_dev_tools(&self) {}

    pub fn load_url(&self, url: &str) {
        if let Some(webview) = &self.webview {
            if let Err(err) = webview.load_url(url) {
                log::error!("failed to load {url}: {err}");
            }
        }
    }

    /// Reloads the current page. WebView2 has no direct API for it, so this goes
    /// through script.
    pub fn reload(&self) {
        self.eval("location.reload()");
    }

    pub fn go_back(&self) {
        self.eval("history.back()");
    }

    pub fn go_forward(&self) {
        self.eval("history.forward()");
    }

    pub fn eval(&self, script: &str) {
        if let Some(webview) = &self.webview {
            if let Err(err) = webview.evaluate_script(script) {
                log::error!("failed to evaluate script: {err}");
            }
        }
    }

    pub fn stop(&self) {
        self.eval("window.stop()");
    }

    pub fn is_loading(&self) -> bool {
        self.is_loading.get()
    }

    /// Whether there is history to go back to.
    ///
    /// WebView2's history state is not reachable through wry, so this backend
    /// **always returns `true`**. Buttons in the UI stay enabled and
    /// `history.back()` simply does nothing when there is no history. This
    /// differs from the macOS (CEF) backend.
    pub fn can_go_back(&self) -> bool {
        true
    }

    /// Whether there is history to go forward to. Always `true`, for the same
    /// reason as [`Webview::can_go_back`].
    pub fn can_go_forward(&self) -> bool {
        true
    }

    /// The current page title.
    ///
    /// Not available on this backend, so it always returns an empty string.
    /// This differs from the macOS (CEF) backend.
    pub fn title(&self) -> String {
        String::new()
    }

    pub fn url(&self) -> String {
        self.webview
            .as_ref()
            .and_then(|webview| webview.url().ok())
            .unwrap_or_else(|| self.url.clone())
    }

    /// Shows or hides the child window, e.g. when switching tabs.
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

    pub fn set_visible(&self, visible: bool) {
        if let Some(webview) = &self.webview {
            let _ = webview.set_visible(visible);
        }
    }

    /// Keeps the child window aligned with gpui's layout.
    fn sync_bounds(&mut self, bounds: Bounds<Pixels>) {
        if self.last_bounds == Some(bounds) {
            return;
        }
        self.last_bounds = Some(bounds);

        let Some(webview) = &self.webview else { return };
        let _ = webview.set_bounds(Rect {
            position: LogicalPosition::new(
                f32::from(bounds.origin.x) as f64,
                f32::from(bounds.origin.y) as f64,
            )
            .into(),
            size: LogicalSize::new(
                f32::from(bounds.size.width) as f64,
                f32::from(bounds.size.height) as f64,
            )
            .into(),
        });
    }
}

/// A script, run before each page's own, that forwards console lines carrying
/// [`crate::MESSAGE_PREFIX`] to wry's IPC channel.
fn console_bridge_script() -> String {
    format!(
        "(() => {{ const prefix = {prefix:?}; const debug = console.debug; \
         console.debug = function (...args) {{ \
           const line = args.map(String).join(' '); \
           if (line.startsWith(prefix)) {{ window.ipc.postMessage(line.slice(prefix.length)); return; }} \
           return debug.apply(this, args); \
         }}; }})();",
        prefix = crate::MESSAGE_PREFIX,
    )
}

impl gpui::EventEmitter<crate::WebviewEvent> for Webview {}

impl gpui::Focusable for Webview {
    fn focus_handle(&self, _cx: &App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Webview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for message in std::mem::take(&mut *self.inbox.borrow_mut()) {
            cx.emit(crate::WebviewEvent::Message(message));
        }
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .child(WebviewBoundsTracker {
                webview: cx.entity(),
            })
    }
}

/// An element that does nothing but report its laid-out rect back to the webview
/// entity. WebView2's child window does the drawing, so nothing is painted here.
struct WebviewBoundsTracker {
    webview: gpui::Entity<Webview>,
}

impl Element for WebviewBoundsTracker {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = gpui::relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        cx: &mut App,
    ) {
        self.webview
            .update(cx, |webview, _| webview.sync_bounds(bounds));
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }
}

impl IntoElement for WebviewBoundsTracker {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
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
    pub fn set_dark(&self, _dark: bool) -> bool {
        false
    }
    pub fn show_over(&self, _window: &gpui::Window, _bounds: gpui::Bounds<gpui::Pixels>, _corner_radius: f32) -> bool {
        false
    }
    pub fn hide(&self) {}
    pub fn eval(&self, _expression: &str, done: impl FnOnce(Option<String>) + 'static) {
        done(None)
    }
}

pub fn on_chrome_tab(_open: impl Fn(String) + 'static) {}
