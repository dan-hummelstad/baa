//! macOS backend: Chromium (CEF) pages as native views under gpui's.
//!
//! # How it fits together
//!
//! 1. CEF is brought up in two steps, [`init`] then [`Runtime::start`], to avoid
//!    fighting gpui over `NSApp`. See [`Runtime`] for why.
//! 2. [`Webview::new`] creates a browser inside a container view placed below
//!    gpui's view; [`native`] explains the layering and input routing.
//! 3. [`PageSlot`] is the element: it moves the container to the laid-out rect,
//!    punches a hole in gpui's layer there, and forwards mouse events.
//! 4. CEF's message loop is driven by a `CFRunLoopTimer` installed on the main
//!    run loop; see [`pump`] for why it cannot live on gpui's executor.

mod chrome_window;
mod handlers;
mod native;
mod nsapp;
mod pump;
mod shared;

pub use chrome_window::open_chrome_window;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

use cef::{
    args::Args, browser_host_create_browser, execute_process, initialize,
    library_loader::LibraryLoader, request_context_create_context, BrowserHost,
    BrowserSettings, CefString, ImplBrowser, ImplBrowserHost, ImplFrame, RequestContext,
    RequestContextSettings, Settings, WindowInfo,
};
use gpui::{
    div, relative, App, Bounds, Context, DispatchPhase, Element, ElementId,
    Focusable, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollWheelEvent,
    GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, InteractiveElement, IntoElement,
    LayoutId, ParentElement, Pixels, Render, Style, Styled, Window,
};
use objc2::runtime::AnyObject;

use self::shared::Shared;
use crate::{Error, ProfileOptions, RuntimeOptions, WebviewOptions};

/// The process-wide CEF runtime, as a cloneable handle.
///
/// Starting CEF is split in two:
///
/// 1. [`init`] — at the top of `main`. Loads the framework and handles the case
///    where this process was spawned as a CEF child process.
/// 2. [`Runtime::start`] — inside the `Application::run` closure. This is what
///    actually calls `cef_initialize`.
///
/// The split exists because of `NSApp`. macOS has exactly one `NSApplication`
/// per process, and it is instantiated as **whichever class calls
/// `sharedApplication` first**. Initializing CEF first yields a plain
/// `NSApplication`, and gpui then crashes because the instance it gets back has
/// none of its own ivars. gpui has to create `NSApp` before CEF is initialized.
///
/// The handle is cloneable so that moving it into a closure does not shorten its
/// life. Handing the value itself to the `Application::run` closure would run
/// `cef_shutdown()` **the moment the closure returns** — that is, right after
/// startup — killing Chromium mid-launch with a SIGTRAP.
#[derive(Clone)]
pub struct Runtime(Rc<RuntimeInner>);

struct RuntimeInner {
    // Handle to the dlopen'd framework. Dropping it takes CEF down with it.
    _loader: LibraryLoader,
    app: RefCell<cef::App>,
    options: RuntimeOptions,
    started: Cell<bool>,
}

impl Runtime {
    /// Calls `cef_initialize`. Call it exactly once, **inside the
    /// `gpui::Application::run` closure** and before creating the first
    /// [`Webview`].
    ///
    /// `cx` is unused; it is taken so the signature pins the call site to "after
    /// gpui has created `NSApp`".
    pub fn start(&self, _cx: &mut App) -> crate::Result<()> {
        if self.0.started.replace(true) {
            return Ok(());
        }

        // Retrofit CefAppProtocol onto gpui's NSApplication subclass. Without
        // this, a CHECK inside cef_initialize takes the process down.
        if let Err(err) = nsapp::adopt_cef_app_protocol() {
            return Err(Error::Backend(err.to_string()));
        }

        let args = Args::new();
        let mut settings = Settings {
            // gpui owns the event loop — gpui is what calls [NSApp run]. Without
            // external_message_pump, CEF builds a message pump that assumes it
            // drives NSApplication's run loop itself, and Chromium then trips a
            // CHECK that takes the whole process down.
            multi_threaded_message_loop: 0,
            external_message_pump: 1,
            ..Default::default()
        };
        if let Some(log_file) = &self.0.options.log_file {
            settings.log_file = log_file.to_string_lossy().as_ref().into();
            settings.log_severity = cef::LogSeverity::VERBOSE;
        }
        // The global context's cache_path stays empty (in memory): every page
        // gets a Profile, and anything left on the global one should not persist.
        if let Some(root) = &self.0.options.root_cache_path {
            settings.root_cache_path = root.to_string_lossy().as_ref().into();
        }

        let mut app = self.0.app.borrow_mut();
        if initialize(
            Some(args.as_main_args()),
            Some(&settings),
            Some(&mut *app),
            std::ptr::null_mut(),
        ) != 1
        {
            return Err(Error::Initialize);
        }

        Ok(())
    }

    /// Puts cookies in the global, in-memory store — the one every [`Webview`]
    /// without a [`Profile`] shares — and says how many it accepted. Call it after [`Runtime::start`].
    ///
    /// The store may turn a cookie down (a domain that does not match its URL,
    /// say); that one is skipped rather than failing the rest.
    pub fn set_cookies(&self, cookies: &[crate::CookieSpec]) -> crate::Result<usize> {
        use cef::ImplCookieManager as _;

        if !self.0.started.get() {
            return Err(Error::Backend("set_cookies() called before start()".into()));
        }
        let manager = cef::cookie_manager_get_global_manager(None)
            .ok_or_else(|| Error::Backend("no cookie store".into()))?;
        let mut accepted = 0;
        for spec in cookies {
            let cookie = cef_cookie(spec);
            let url = CefString::from(spec.url.as_str());
            if manager.set_cookie(Some(&url), Some(&cookie), None) != 0 {
                accepted += 1;
            }
        }
        Ok(accepted)
    }

    /// Shuts CEF down. Call this once `gpui::Application::run` has returned.
    ///
    /// This drops one handle; `cef_shutdown()` runs when the last one goes, so
    /// keeping a clone alive past this call keeps CEF alive too.
    pub fn shutdown(self) {}
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        if self.started.get() {
            cef::shutdown();
        }
    }
}

/// Loads CEF into the process. Call this **at the top of `main`**, before
/// `gpui_platform::application()`. The actual initialization happens in
/// [`Runtime::start`].
///
/// On macOS the executable has to live inside an `.app` bundle that ships
/// `Frameworks/Chromium Embedded Framework.framework` — the layout `xtask`
/// produces.
pub fn init(options: RuntimeOptions) -> crate::Result<Runtime> {
    // Checked before anything is loaded: CEF's own loader does not report a
    // missing framework, it stops the process — and an app run with
    // `cargo run`, outside its bundle, should get an error it can carry on
    // without rather than a crash at launch.
    let exe = std::env::current_exe().map_err(|e| Error::LibraryLoad(e.to_string()))?;
    if framework_binary(&exe).is_none() {
        return Err(Error::LibraryLoad(exe.display().to_string()));
    }

    // Loading the framework and running execute_process twice is not something
    // CEF expects, and the failure mode is a confusing crash much later.
    static INITIALIZED: AtomicBool = AtomicBool::new(false);
    if INITIALIZED.swap(true, Ordering::SeqCst) {
        return Err(Error::Backend("gpui_cef::init() called twice".into()));
    }

    let loader = LibraryLoader::new(&exe, false);
    if !loader.load() {
        return Err(Error::LibraryLoad(exe.display().to_string()));
    }

    // Pin the API version. This has to happen before any handler is registered.
    let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);

    let args = Args::new();
    let mut app = handlers::GpuiCefApp::new(handlers::AppConfig {
        remote_debugging_port: options.remote_debugging_port,
        log_to_stderr: options.log_to_stderr,
        switches: options.switches.clone(),
    });

    // Returns -1 in the browser process, which is what this one is. Child
    // processes run a separate binary (`*_helper`), so they never get here.
    let code = execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if code != -1 {
        // Spawned as a child process after all — exit with whatever CEF wants.
        std::process::exit(code);
    }

    Ok(Runtime(Rc::new(RuntimeInner {
        _loader: loader,
        app: RefCell::new(app),
        options,
        started: Cell::new(false),
    })))
}

/// A storage profile: one CEF request context, so its own cookies, cache,
/// local storage and service workers. Cheap to clone; webviews created with
/// clones share storage.
#[derive(Clone)]
pub struct Profile {
    pub(crate) context: RequestContext,
}

impl Profile {
    /// Call after [`Runtime::start`].
    pub fn new(options: ProfileOptions) -> crate::Result<Self> {
        let cache_path = options
            .cache_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let settings = RequestContextSettings {
            cache_path: cache_path.as_str().into(),
            persist_session_cookies: 1,
            ..Default::default()
        };
        let mut handler = handlers::ProfileContextHandler::new(options.filter, options.prefs_off);
        let context = request_context_create_context(Some(&settings), Some(&mut handler))
            .ok_or_else(|| Error::Backend("cef_request_context_create_context() failed".into()))?;
        Ok(Self { context })
    }
}

impl std::fmt::Debug for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Profile")
    }
}

/// A webview, as a gpui entity.
///
/// Build it with
/// `cx.new(|cx| Webview::new(window, cx, WebviewOptions::url("https://example.com")))`
/// and place it like any other element.
pub struct Webview {
    shared: Rc<Shared>,
    focus_handle: gpui::FocusHandle,
    /// The view the page lives in, below gpui's.
    container: Rc<native::Container>,
    /// The page view a mouse drag started on, and whether the pointer is over
    /// the page; across frames, so a repaint mid-click loses nothing.
    pointer: Rc<Pointer>,
    /// gpui's own view, which gets keyboard focus back when the page loses it.
    gpui_view: *mut AnyObject,
    corner_radius: Rc<Cell<f32>>,
    /// The last find-in-page query, so a repeat goes to the next match.
    find_text: RefCell<String>,
    this: gpui::WeakEntity<Webview>,
    _pump: pump::Registration,
    _focus_subscriptions: [gpui::Subscription; 2],
}

impl Webview {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, options: WebviewOptions) -> Self {
        let shared = Shared::new(options.url.clone());
        let gpui_view = native::gpui_view(window);
        shared.set_gpui_view(gpui_view);
        native::install_mouse_forwarding(gpui_view);
        let container = Rc::new(native::Container::new(gpui_view, options.corner_radius));

        // Provisional size until layout runs; the element sets the real frame.
        let size = window.viewport_size();
        let window_info = WindowInfo {
            parent_view: container.as_ptr().cast(),
            bounds: cef::Rect {
                x: 0,
                y: 0,
                width: (f32::from(size.width) as i32).max(1),
                height: (f32::from(size.height) as i32).max(1),
            },
            // A native child view is Alloy style; Chrome style needs CEF's Views.
            runtime_style: cef::RuntimeStyle::ALLOY,
            // Chromium tracks visibility by hide/unhide transitions of its own
            // view, so it starts hidden and is shown only once laid out.
            hidden: 1,
            ..Default::default()
        };

        let browser_settings = BrowserSettings {
            background_color: if options.transparent {
                0
            } else {
                0xFF_FF_FF_FF
            },
            ..Default::default()
        };

        let profile = options.profile.clone();
        let mut client = handlers::GpuiCefClient::build(shared.clone());
        let mut context = profile.map(|p| p.context);
        // Async on purpose: with the Chrome bootstrap, CreateBrowserSync returns
        // null until the profile's request context has initialised, which a
        // fresh on-disk profile has not. The handle arrives in on_after_created.
        let created = browser_host_create_browser(
            Some(&window_info),
            Some(&mut client),
            Some(&options.url.as_str().into()),
            Some(&browser_settings),
            None,
            context.as_mut(),
        );
        if created == 0 {
            log::error!("cef_browser_host_create_browser() failed");
        }

        let focus_handle = cx.focus_handle();
        let (pump, focus_subscriptions) = attach(&shared, &focus_handle, window, cx);

        Self {
            shared,
            focus_handle,
            container,
            pointer: Rc::default(),
            gpui_view,
            corner_radius: Rc::new(Cell::new(options.corner_radius)),
            find_text: RefCell::default(),
            this: cx.weak_entity(),
            _pump: pump,
            _focus_subscriptions: focus_subscriptions,
        }
    }

    /// Rounds the page's corners, in logical pixels.
    pub fn set_corner_radius(&self, radius: f32) {
        self.corner_radius.set(radius);
        self.container.set_corner_radius(radius);
    }

    /// Keeps this page's view above other pages' (an overlay like Peek): native
    /// views stack in creation order, whatever order gpui paints them in.
    pub fn set_floating(&self, floating: bool) {
        self.container.set_floating(floating);
    }

    /// Moves the live page, state and all, into another gpui window.
    pub fn move_to_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let gpui_view = native::gpui_view(window);
        self.shared.set_gpui_view(gpui_view);
        self.gpui_view = gpui_view;
        self.container.move_below(gpui_view);
        native::install_mouse_forwarding(gpui_view);
        (self._pump, self._focus_subscriptions) = attach(&self.shared, &self.focus_handle, window, cx);
        cx.notify();
    }

    pub(crate) fn browser_created(&self) {
        self.container.page_created(page_view(&self.shared));
    }

    fn set_browser_focus(&self, focused: bool) {
        // Chromium makes its own view first responder; applied on creation if
        // the browser does not exist yet.
        self.shared.set_wants_focus(focused);
        if let Some(host) = self.host() {
            host.set_focus(focused as i32);
        }
        if !focused && native::has_first_responder(page_view(&self.shared)) {
            native::make_first_responder(self.gpui_view);
        }
    }

    pub fn load_url(&self, url: &str) {
        if let Some(frame) = self.shared.browser().and_then(|b| b.main_frame()) {
            frame.load_url(Some(&url.into()));
        }
    }

    pub fn reload(&self) {
        if let Some(browser) = self.shared.browser() {
            browser.reload();
        }
    }

    pub fn go_back(&self) {
        if let Some(browser) = self.shared.browser() {
            browser.go_back();
        }
    }

    pub fn go_forward(&self) {
        if let Some(browser) = self.shared.browser() {
            browser.go_forward();
        }
    }

    pub fn title(&self) -> String {
        self.shared.title()
    }

    pub fn url(&self) -> String {
        self.shared.url()
    }

    pub fn is_loading(&self) -> bool {
        self.shared.is_loading()
    }

    pub fn can_go_back(&self) -> bool {
        self.shared.can_go_back()
    }

    pub fn can_go_forward(&self) -> bool {
        self.shared.can_go_forward()
    }

    pub fn eval(&self, script: &str) {
        if let Some(frame) = self.shared.browser().and_then(|b| b.main_frame()) {
            frame.execute_java_script(Some(&script.into()), None, 0);
        }
    }

    /// The back/forward list and the index of the current entry.
    pub fn history(&self) -> (Vec<crate::HistoryEntry>, usize) {
        let entries: Rc<RefCell<(Vec<crate::HistoryEntry>, usize)>> = Rc::default();
        if let Some(host) = self.host() {
            let mut visitor = handlers::GpuiCefHistoryVisitor::new(entries.clone());
            host.navigation_entries(Some(&mut visitor), 0);
        }
        entries.take()
    }

    /// Moves `offset` entries through history: negative is back.
    pub fn go_to_offset(&self, offset: i32) {
        self.eval(&format!("history.go({offset})"));
    }

    pub fn stop(&self) {
        if let Some(browser) = self.shared.browser() {
            browser.stop_load();
        }
    }

    /// This webview's own entity, for code that only holds `&Webview`.
    pub fn weak_entity(&self) -> gpui::WeakEntity<Self> {
        self.this.clone()
    }

    /// Page zoom as a factor: 1.0 is 100%.
    pub fn set_zoom(&self, factor: f64) {
        if let Some(host) = self.host() {
            // Chromium's zoom level is log base 1.2 of the factor.
            host.set_zoom_level(factor.max(0.01).ln() / 1.2f64.ln());
        }
    }

    /// Highlights matches of `text` and moves to the next (or previous) one.
    /// An empty `text` ends the search.
    pub fn find(&self, text: &str, forward: bool) {
        let Some(host) = self.host() else { return };
        if text.is_empty() {
            host.stop_finding(1);
            self.find_text.borrow_mut().clear();
            return;
        }
        let find_next = *self.find_text.borrow() == text;
        if !find_next {
            *self.find_text.borrow_mut() = text.to_string();
        }
        host.find(Some(&text.into()), forward as i32, 0, find_next as i32);
    }

    /// Opens Chrome DevTools for this page in a separate native window.
    pub fn show_dev_tools(&self) {
        if let Some(host) = self.host() {
            host.show_dev_tools(
                Some(&WindowInfo::default()),
                None,
                Some(&BrowserSettings::default()),
                None,
            );
        }
    }

    /// Hidden browsers keep their state but stop painting and throttle timers:
    /// Chromium watches its view's visibility.
    pub fn set_visible(&self, visible: bool) {
        self.container.set_visible(visible, page_view(&self.shared));
    }

    fn host(&self) -> Option<BrowserHost> {
        self.shared.host()
    }
}

impl Drop for Webview {
    fn drop(&mut self) {
        // CEF still talks to the page's view while the browser closes, so the
        // view stays in the window, hidden, until on_before_close lets it go.
        self.container.set_visible(false, page_view(&self.shared));
        self.shared.hold_until_closed(self.container.clone());
        if let Some(host) = self.host() {
            host.close_browser(1);
        }
    }
}

impl gpui::EventEmitter<crate::WebviewEvent> for Webview {}

impl Focusable for Webview {
    fn focus_handle(&self, _cx: &App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Webview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .key_context("Webview")
            .size_full()
            .child(PageSlot {
                shared: self.shared.clone(),
                container: self.container.clone(),
                focus_handle: self.focus_handle.clone(),
                pointer: self.pointer.clone(),
                corner_radius: self.corner_radius.get(),
            })
    }
}

/// Keyboard focus follows gpui's: the page's view is first responder while the
/// webview is focused, so typing and IME go to Chromium natively. A click in
/// the page focuses Chromium first; gpui follows (see pump).
fn attach(
    shared: &Rc<Shared>,
    focus_handle: &gpui::FocusHandle,
    window: &mut Window,
    cx: &mut Context<Webview>,
) -> (pump::Registration, [gpui::Subscription; 2]) {
    let pump = pump::register(
        shared.clone(),
        cx.entity().downgrade(),
        cx.to_async(),
        window.window_handle(),
        focus_handle.clone(),
    );
    let subscriptions = [
        cx.on_focus_in(focus_handle, window, |this, _, _| this.set_browser_focus(true)),
        cx.on_focus_out(focus_handle, window, |this, _, _, _| this.set_browser_focus(false)),
    ];
    (pump, subscriptions)
}

/// Chromium's view for this page, once the browser exists.
fn page_view(shared: &Shared) -> *mut AnyObject {
    shared
        .host()
        .map(|host| host.window_handle().cast())
        .unwrap_or(std::ptr::null_mut())
}

/// Where the page goes: moves the native view to the laid-out rect and makes
/// gpui's layer transparent over it.
struct PageSlot {
    shared: Rc<Shared>,
    container: Rc<native::Container>,
    focus_handle: gpui::FocusHandle,
    pointer: Rc<Pointer>,
    corner_radius: f32,
}

#[derive(Default)]
struct Pointer {
    drag: Cell<Option<*mut AnyObject>>,
    inside: Cell<bool>,
}

impl Element for PageSlot {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

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
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _cx: &mut App,
    ) -> Hitbox {
        self.shared.set_bounds(bounds);
        let visible = bounds.intersect(&window.content_mask().bounds);
        if visible.size.width > Pixels::ZERO && visible.size.height > Pixels::ZERO {
            self.container.place(page_view(&self.shared), bounds, visible);
        } else {
            self.container.set_hidden(true);
        }
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let radius = (self.corner_radius * window.scale_factor()).round() as usize;
        if let Some(hole) = native::punch_buffer(radius) {
            window.paint_surface(bounds, hole);
        }
        // gpui sets the cursor for the hitbox under the mouse, so the page's
        // choice goes through it.
        window.set_cursor_style(self.shared.cursor(), hitbox);

        // gpui hit-tests (overlays drawn over the page keep the mouse); the page
        // gets the original NSEvent, so Chromium sees real clicks, trackpad
        // phases and momentum. A drag stays with the view it started on.
        let shared = self.shared.clone();
        let focus = self.focus_handle.clone();
        let hit = hitbox.clone();
        let pointer = self.pointer.clone();
        window.on_mouse_event(move |_: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hit.is_hovered(window) {
                pointer
                    .drag
                    .set(native::forward_current_event(page_view(&shared), None));
                window.focus(&focus, cx);
            }
        });
        let shared = self.shared.clone();
        let hit = hitbox.clone();
        let pointer = self.pointer.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            if let Some(view) = pointer.drag.get().filter(|_| event.pressed_button.is_some()) {
                native::forward_current_event(page_view(&shared), Some(view));
                return;
            }
            // Moves over the page, and the first one off it so hover ends.
            let over = hit.is_hovered(window);
            if over || pointer.inside.get() {
                pointer.inside.set(over);
                native::forward_current_event(page_view(&shared), None);
            }
        });
        let shared = self.shared.clone();
        let pointer = self.pointer.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, _| {
            if phase == DispatchPhase::Bubble {
                if let Some(view) = pointer.drag.take() {
                    native::forward_current_event(page_view(&shared), Some(view));
                }
            }
        });
        let shared = self.shared.clone();
        let hit = hitbox.clone();
        window.on_mouse_event(move |_: &ScrollWheelEvent, phase, window, _| {
            if phase == DispatchPhase::Bubble && hit.should_handle_scroll(window) {
                native::forward_current_event(page_view(&shared), None);
            }
        });
    }
}

impl IntoElement for PageSlot {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// A [`crate::CookieSpec`] in CEF's shape. CEF counts time in microseconds
/// from 1601-01-01.
fn cef_cookie(spec: &crate::CookieSpec) -> cef::Cookie {
    const EPOCH_GAP_SECONDS: i64 = 11_644_473_600;
    let expires = spec.expires.map(|unix| cef::Basetime {
        val: (unix + EPOCH_GAP_SECONDS) * 1_000_000,
    });
    cef::Cookie {
        name: spec.name.as_str().into(),
        value: spec.value.as_str().into(),
        domain: spec.domain.as_str().into(),
        path: spec.path.as_str().into(),
        secure: spec.secure.into(),
        httponly: spec.http_only.into(),
        has_expires: expires.is_some().into(),
        expires: expires.unwrap_or_default(),
        same_site: match spec.same_site {
            crate::SameSite::Unspecified => cef::CookieSameSite::UNSPECIFIED,
            crate::SameSite::None => cef::CookieSameSite::NO_RESTRICTION,
            crate::SameSite::Lax => cef::CookieSameSite::LAX_MODE,
            crate::SameSite::Strict => cef::CookieSameSite::STRICT_MODE,
        },
        ..Default::default()
    }
}

/// Where CEF's loader will look for the framework binary, if it is there:
/// `Contents/Frameworks` beside the executable's `Contents/MacOS`.
fn framework_binary(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    let binary = exe
        .parent()?
        .join("../Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework");
    binary.exists().then_some(binary)
}

#[cfg(test)]
mod tests {
    use super::framework_binary;

    #[test]
    fn the_framework_is_found_only_where_the_bundle_keeps_it() {
        let root = std::env::temp_dir().join(format!("gpui-cef-framework-{}", std::process::id()));
        let exe = root.join("Demo.app/Contents/MacOS/demo");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        assert_eq!(
            framework_binary(&exe),
            None,
            "a bare executable has no framework"
        );

        let binary = root.join(
            "Demo.app/Contents/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework",
        );
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"").unwrap();
        assert_eq!(
            framework_binary(&exe).map(|path| path.canonicalize().unwrap()),
            Some(binary.canonicalize().unwrap())
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
