//! State shared between CEF's callbacks and gpui.
//!
//! CEF runs with `external_message_pump` and a single-threaded message loop, so
//! its handlers are called on the same main thread as gpui. `Rc` and `Cell` are
//! enough.

use std::cell::{Cell, RefCell};

use cef::{Browser, BrowserHost, ImplBrowser};
use gpui::{Bounds, CursorStyle, Pixels};

/// Shared state for one webview.
pub(crate) struct Shared {
    /// The rect gpui laid this out at, in window space. Context menu positions
    /// arrive relative to it.
    bounds: Cell<Bounds<Pixels>>,
    /// Set when gpui has to repaint (an event or a cursor change); the pump
    /// picks this up and calls `notify()`.
    dirty: Cell<bool>,
    title: RefCell<String>,
    url: RefCell<String>,
    /// No address has been reported yet: the first one is news even when it
    /// is the URL the webview was created with.
    no_address: Cell<bool>,
    /// Whether a page is loading, as reported by CEF's `LoadHandler`.
    is_loading: Cell<bool>,
    can_go_back: Cell<bool>,
    can_go_forward: Cell<bool>,
    /// The cursor the page wants, as reported by CEF's `OnCursorChange`.
    cursor: Cell<CursorStyle>,
    /// The browser, kept here so the message pump can reach `BrowserHost`
    /// without borrowing gpui's `App`.
    browser: RefCell<Option<Browser>>,
    /// Events waiting for the pump to emit them.
    events: RefCell<Vec<crate::WebviewEvent>>,
    /// gpui's NSView, which gets first look at the page's keyboard shortcuts.
    gpui_view: Cell<*mut objc2::runtime::AnyObject>,
    /// Whether gpui has the webview focused, for a browser created after that.
    wants_focus: Cell<bool>,
    /// Set when the browser arrives; the webview finishes setting it up.
    created: Cell<bool>,
    /// An init script and the URL to load once it is installed.
    start: RefCell<Option<(String, String)>>,
    auto_resize: Cell<Option<(gpui::Size<Pixels>, gpui::Size<Pixels>)>>,
    /// Set when Chromium took keyboard focus itself (a click in the page).
    focused_natively: Cell<bool>,
    /// The page's native view, kept until CEF has closed the browser.
    closing: RefCell<Option<std::rc::Rc<dyn std::any::Any>>>,
    /// The JavaScript dialog waiting for the user; Chromium allows one at a time.
    dialog: RefCell<Option<cef::JsdialogCallback>>,
    /// Permission requests waiting for the user, by request id.
    permissions: RefCell<Vec<(u64, PendingPermission)>>,
    audible: Cell<bool>,
    /// A Chrome-style tab's own window (see `chrome_window::create_tab`).
    tab_window: RefCell<Option<cef::Window>>,
    /// Its NSWindow, retained.
    tab_ns_window: Cell<*mut objc2::runtime::AnyObject>,
    /// Its content view once moved into the app's window. CEF's handle for
    /// the page is the window's content view, which is then gone.
    tab_view: Cell<*mut objc2::runtime::AnyObject>,
    /// The size last given to the tab's window, which Chrome lays the page out by.
    tab_size: Cell<(i32, i32)>,
    /// Set when the tab should become Chrome's active one on the next pump tick.
    activate: Cell<bool>,
    /// Whether the tab's window has been shown yet.
    tab_shown: Cell<bool>,
    /// The page lives in a Chrome window of its own whose view the app takes
    /// (a Chrome-style tab, or DevTools Chrome opened): the "tab" calls below.
    adopts: Cell<bool>,
    /// The client for the DevTools this page is about to open, so that it
    /// reports to the app's DevTools webview.
    devtools_client: RefCell<Option<cef::Client>>,
    /// For DevTools docked by the app: the side it starts on ("right",
    /// "bottom", "left"); see `handlers::dock_script`.
    devtools_side: RefCell<Option<String>>,
}

pub(crate) enum PendingPermission {
    /// Camera/microphone/screen: granted per request, and remembered for the
    /// site in the profile's content settings.
    Media {
        callback: cef::MediaAccessCallback,
        requested: u32,
        origin: String,
    },
    /// Everything else goes through Chrome's permission manager, which
    /// remembers the answer itself.
    Prompt(cef::PermissionPromptCallback),
}

impl Shared {
    pub(crate) fn new(url: String) -> std::rc::Rc<Self> {
        std::rc::Rc::new(Self {
            bounds: Cell::new(Bounds::default()),
            dirty: Cell::new(false),
            title: RefCell::new(String::new()),
            url: RefCell::new(url),
            no_address: Cell::new(true),
            is_loading: Cell::new(false),
            can_go_back: Cell::new(false),
            can_go_forward: Cell::new(false),
            cursor: Cell::new(CursorStyle::Arrow),
            browser: RefCell::new(None),
            events: RefCell::new(Vec::new()),
            gpui_view: Cell::new(std::ptr::null_mut()),
            wants_focus: Cell::new(false),
            created: Cell::new(false),
            start: RefCell::new(None),
            auto_resize: Cell::new(None),
            focused_natively: Cell::new(false),
            closing: RefCell::new(None),
            dialog: RefCell::new(None),
            permissions: RefCell::new(Vec::new()),
            audible: Cell::new(false),
            tab_window: RefCell::new(None),
            tab_ns_window: Cell::new(std::ptr::null_mut()),
            tab_view: Cell::new(std::ptr::null_mut()),
            tab_size: Cell::new((0, 0)),
            activate: Cell::new(false),
            tab_shown: Cell::new(false),
            adopts: Cell::new(false),
            devtools_client: RefCell::new(None),
            devtools_side: RefCell::new(None),
        })
    }

    /// Queues an event; the pump emits it on its next pass.
    pub(crate) fn push_event(&self, event: crate::WebviewEvent) {
        self.events.borrow_mut().push(event);
        self.dirty.set(true);
    }

    pub(crate) fn take_events(&self) -> Vec<crate::WebviewEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }

    pub(crate) fn set_bounds(&self, bounds: Bounds<Pixels>) {
        self.bounds.set(bounds);
    }

    pub(crate) fn bounds(&self) -> Bounds<Pixels> {
        self.bounds.get()
    }

    pub(crate) fn set_browser(&self, browser: Browser) {
        if let (Some((min, max)), Some(host)) = (self.auto_resize.get(), browser.host()) {
            use cef::ImplBrowserHost as _;
            let size = |s: gpui::Size<Pixels>| cef::Size {
                width: f32::from(s.width) as i32,
                height: f32::from(s.height) as i32,
            };
            host.set_auto_resize_enabled(1, Some(&size(min)), Some(&size(max)));
        }
        if self.wants_focus.get() {
            if let Some(host) = browser.host() {
                use cef::ImplBrowserHost as _;
                host.set_focus(1);
            }
        }
        if let Some((script, url)) = self.start.borrow_mut().take() {
            install_init_script(&browser, &script, url);
        }
        *self.browser.borrow_mut() = Some(browser);
        self.created.set(true);
        // Layout has to run again to place the new view.
        self.dirty.set(true);
    }

    pub(crate) fn set_auto_resize(&self, range: Option<(gpui::Size<Pixels>, gpui::Size<Pixels>)>) {
        self.auto_resize.set(range);
    }

    /// Opens `url` with `script` installed first, once the browser exists.
    pub(crate) fn start_with(&self, script: String, url: String) {
        *self.start.borrow_mut() = Some((script, url));
    }

    pub(crate) fn hold_until_closed(&self, view: std::rc::Rc<dyn std::any::Any>) {
        *self.closing.borrow_mut() = Some(view);
    }

    pub(crate) fn release_closed(&self) {
        self.closing.borrow_mut().take();
    }

    pub(crate) fn set_focused_natively(&self) {
        self.focused_natively.set(true);
        self.dirty.set(true);
    }

    pub(crate) fn take_focused_natively(&self) -> bool {
        self.focused_natively.replace(false)
    }

    pub(crate) fn take_created(&self) -> bool {
        self.created.replace(false)
    }

    pub(crate) fn set_wants_focus(&self, focused: bool) {
        self.wants_focus.set(focused);
    }

    pub(crate) fn browser(&self) -> Option<Browser> {
        self.browser.borrow().clone()
    }

    pub(crate) fn host(&self) -> Option<BrowserHost> {
        self.browser
            .borrow()
            .as_ref()
            .and_then(|browser| browser.host())
    }

    pub(crate) fn set_gpui_view(&self, view: *mut objc2::runtime::AnyObject) {
        self.gpui_view.set(view);
    }

    pub(crate) fn gpui_view(&self) -> *mut objc2::runtime::AnyObject {
        self.gpui_view.get()
    }

    pub(crate) fn take_dirty(&self) -> bool {
        self.dirty.replace(false)
    }

    pub(crate) fn set_title(&self, title: String) {
        if *self.title.borrow() != title {
            *self.title.borrow_mut() = title.clone();
            self.push_event(crate::WebviewEvent::TitleChanged(title));
        }
    }

    pub(crate) fn set_url(&self, url: String) {
        if self.no_address.replace(false) || *self.url.borrow() != url {
            *self.url.borrow_mut() = url.clone();
            self.push_event(crate::WebviewEvent::UrlChanged(url));
        }
    }

    pub(crate) fn set_load_state(&self, is_loading: bool, can_go_back: bool, can_go_forward: bool) {
        if self.is_loading.replace(is_loading) != is_loading {
            self.push_event(crate::WebviewEvent::LoadingChanged(is_loading));
        }
        let back = self.can_go_back.replace(can_go_back) != can_go_back;
        let forward = self.can_go_forward.replace(can_go_forward) != can_go_forward;
        if back || forward {
            self.push_event(crate::WebviewEvent::HistoryChanged {
                can_go_back,
                can_go_forward,
            });
        }
    }

    pub(crate) fn is_loading(&self) -> bool {
        self.is_loading.get()
    }

    pub(crate) fn can_go_back(&self) -> bool {
        self.can_go_back.get()
    }

    pub(crate) fn can_go_forward(&self) -> bool {
        self.can_go_forward.get()
    }

    pub(crate) fn set_cursor(&self, cursor: CursorStyle) {
        if self.cursor.replace(cursor) != cursor {
            // The cursor is applied while painting, so a repaint has to happen.
            self.dirty.set(true);
        }
    }

    pub(crate) fn cursor(&self) -> CursorStyle {
        self.cursor.get()
    }

    pub(crate) fn set_dialog(&self, callback: Option<cef::JsdialogCallback>) {
        *self.dialog.borrow_mut() = callback;
    }

    pub(crate) fn take_dialog(&self) -> Option<cef::JsdialogCallback> {
        self.dialog.borrow_mut().take()
    }

    pub(crate) fn add_permission(&self, id: u64, pending: PendingPermission) {
        self.permissions.borrow_mut().push((id, pending));
    }

    pub(crate) fn take_permission(&self, id: u64) -> Option<PendingPermission> {
        let mut list = self.permissions.borrow_mut();
        let index = list.iter().position(|(i, _)| *i == id)?;
        Some(list.remove(index).1)
    }

    pub(crate) fn set_audible(&self, audible: bool) {
        if self.audible.replace(audible) != audible {
            self.push_event(crate::WebviewEvent::AudibleChanged(audible));
        }
    }

    pub(crate) fn set_tab_window(&self, window: cef::Window, ns_window: *mut objc2::runtime::AnyObject) {
        *self.tab_window.borrow_mut() = Some(window);
        self.tab_ns_window.set(ns_window);
        self.adopts.set(true);
    }

    /// The page will come up in a Chrome window that CEF makes (DevTools).
    pub(crate) fn set_adopts_window(&self) {
        self.adopts.set(true);
    }

    pub(crate) fn is_tab(&self) -> bool {
        self.adopts.get()
    }

    pub(crate) fn set_devtools_client(&self, client: cef::Client) {
        *self.devtools_client.borrow_mut() = Some(client);
    }

    pub(crate) fn set_devtools_side(&self, side: &str) {
        *self.devtools_side.borrow_mut() = Some(side.to_string());
    }

    pub(crate) fn devtools_side(&self) -> Option<String> {
        self.devtools_side.borrow().clone()
    }

    pub(crate) fn take_devtools_client(&self) -> Option<cef::Client> {
        self.devtools_client.borrow_mut().take()
    }

    /// The tab's NSWindow; null for an Alloy page.
    pub(crate) fn tab_ns_window(&self) -> *mut objc2::runtime::AnyObject {
        self.tab_ns_window.get()
    }

    /// Takes the tab's page into `container`, once.
    pub(crate) fn adopt_tab_view(&self, container: *mut objc2::runtime::AnyObject) {
        if self.tab_ns_window.get().is_null() && self.adopts.get() {
            use cef::ImplBrowserHost as _;
            // CEF made this window: its handle is still in it now.
            let handle = self.host().map(|h| h.window_handle().cast()).unwrap_or(std::ptr::null_mut());
            self.tab_ns_window.set(super::native::retained(super::native::window_of(handle)));
        }
        if self.tab_view.get().is_null() {
            self.tab_view.set(super::native::adopt_content_view(self.tab_ns_window(), container));
        }
    }

    pub(crate) fn tab_view(&self) -> *mut objc2::runtime::AnyObject {
        self.tab_view.get()
    }

    /// Hands the page back to its window ahead of closing.
    pub(crate) fn restore_tab_view(&self) {
        super::native::restore_content_view(self.tab_ns_window(), self.tab_view.replace(std::ptr::null_mut()));
    }

    /// Sizes the tab's window to the page, off screen.
    pub(crate) fn fit_tab_window(&self, size: (i32, i32)) {
        use cef::ImplView as _;
        if self.tab_size.replace(size) == size {
            return;
        }
        match &*self.tab_window.borrow() {
            Some(window) => window.set_bounds(Some(&cef::Rect { x: -10000, y: -10000, width: size.0.max(1), height: size.1.max(1) })),
            None => super::native::set_window_size(self.tab_ns_window(), size),
        }
    }

    /// Makes Chrome take this tab's window as the last active one, which is
    /// what extensions read as "the current window" (`chrome.tabs.query`'s
    /// `currentWindow`/`lastFocusedWindow`, the action's active tab). The
    /// keyboard stays with the app's window. Done from the pump: activating
    /// swaps the key window under gpui, which must not happen mid-update.
    pub(crate) fn activate_tab(&self) {
        // Only tabs: DevTools is no window extensions should take as current.
        self.activate.set(self.tab_window.borrow().is_some());
    }

    /// Pump work for a tab's window, where gpui's state is not borrowed:
    /// showing it once, keeping it unseen, and activation.
    pub(crate) fn run_tab_window(&self) {
        use cef::ImplWindow as _;
        let ns_window = self.tab_ns_window();
        if ns_window.is_null() {
            return;
        }
        if !self.tab_shown.replace(true) {
            if let Some(window) = &*self.tab_window.borrow() {
                let key = super::native::window_of(self.gpui_view());
                window.show();
                super::native::make_key_soon(key);
            }
        }
        super::native::keep_unseen(ns_window);
        if !self.activate.replace(false) {
            return;
        }
        super::native::announce_key(ns_window);
    }

    pub(crate) fn title(&self) -> String {
        self.title.borrow().clone()
    }

    pub(crate) fn url(&self) -> String {
        self.url.borrow().clone()
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        super::native::forget_unseen(self.tab_ns_window.get());
        super::native::release(self.tab_ns_window.get());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_only_dirties_on_change() {
        let shared = Shared::new("about:blank".into());
        shared.take_dirty();

        shared.set_cursor(CursorStyle::PointingHand);
        assert!(shared.take_dirty());
        assert_eq!(shared.cursor(), CursorStyle::PointingHand);

        // CEF reports the cursor on every move; repeats must not force repaints.
        shared.set_cursor(CursorStyle::PointingHand);
        assert!(!shared.take_dirty());
    }

    #[test]
    fn title_and_url_track_the_page() {
        let shared = Shared::new("https://example.com/".into());
        assert_eq!(shared.url(), "https://example.com/");
        assert_eq!(shared.title(), "");

        shared.set_title("Example".into());
        shared.set_url("https://example.com/next".into());
        assert_eq!(shared.title(), "Example");
        assert_eq!(shared.url(), "https://example.com/next");
        assert!(shared.take_dirty());
    }

    #[test]
    fn load_state_round_trips() {
        let shared = Shared::new("about:blank".into());
        assert!(!shared.is_loading());
        assert!(!shared.can_go_back());

        shared.set_load_state(true, true, false);
        assert!(shared.is_loading());
        assert!(shared.can_go_back());
        assert!(!shared.can_go_forward());
        assert!(shared.take_dirty());
    }

    #[test]
    fn state_changes_are_queued_once() {
        use crate::WebviewEvent::*;
        let shared = Shared::new("about:blank".into());
        shared.set_url("about:blank".into());
        shared.set_url("about:blank".into());
        shared.set_title("A".into());
        shared.set_title("A".into());
        shared.set_load_state(true, false, false);
        shared.set_load_state(false, true, false);
        shared.set_load_state(false, true, false);
        assert_eq!(
            shared.take_events(),
            vec![
                // The first address is reported even when it is where the page started.
                UrlChanged("about:blank".into()),
                TitleChanged("A".into()),
                LoadingChanged(true),
                LoadingChanged(false),
                HistoryChanged {
                    can_go_back: true,
                    can_go_forward: false
                },
            ]
        );
        assert!(shared.take_events().is_empty());
    }
}

const INIT_SCRIPT: i32 = 1;

use cef::rc::Rc as _;
use cef::{DevToolsMessageObserver, ImplDevToolsMessageObserver, WrapDevToolsMessageObserver};

cef::wrap_dev_tools_message_observer! {
    struct InitScriptLoad {
        url: String,
        // Its own registration: held until the page is on its way, since the
        // session goes without an observer, and dropped then, since a held
        // one keeps the closed browser alive.
        registration: std::rc::Rc<RefCell<Option<cef::Registration>>>,
    }
    impl DevToolsMessageObserver {
        fn on_dev_tools_method_result(
            &self,
            browser: Option<&mut cef::Browser>,
            message_id: ::std::os::raw::c_int,
            success: ::std::os::raw::c_int,
            _result: Option<&[u8]>,
        ) {
            use cef::{ImplBrowser as _, ImplFrame as _};
            if message_id != INIT_SCRIPT {
                return;
            }
            if success == 0 {
                log::error!("the page's init script was refused");
            }
            if let Some(frame) = browser.and_then(|b| b.main_frame()) {
                frame.load_url(Some(&self.url.as_str().into()));
            }
            self.registration.borrow_mut().take();
        }
    }
}

/// Registers `script` for every new document, then loads `url` once Chromium
/// has it, so the first document runs it too.
fn install_init_script(browser: &Browser, script: &str, url: String) -> Option<()> {
    use cef::{ImplBrowser as _, ImplBrowserHost as _, ImplDictionaryValue as _};
    let host = browser.host()?;
    let slot = std::rc::Rc::default();
    let registration = host.add_dev_tools_message_observer(Some(&mut InitScriptLoad::new(url, std::rc::Rc::clone(&slot))));
    *slot.borrow_mut() = registration;
    let mut params = cef::dictionary_value_create()?;
    params.set_string(Some(&"source".into()), Some(&script.into()));
    // New-document scripts only run while the Page domain is on.
    host.execute_dev_tools_method(0, Some(&"Page.enable".into()), None);
    host.execute_dev_tools_method(INIT_SCRIPT, Some(&"Page.addScriptToEvaluateOnNewDocument".into()), Some(&mut params));
    Some(())
}
