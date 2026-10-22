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
    /// Set when Chromium took keyboard focus itself (a click in the page).
    focused_natively: Cell<bool>,
}

impl Shared {
    pub(crate) fn new(url: String) -> std::rc::Rc<Self> {
        std::rc::Rc::new(Self {
            bounds: Cell::new(Bounds::default()),
            dirty: Cell::new(false),
            title: RefCell::new(String::new()),
            url: RefCell::new(url),
            is_loading: Cell::new(false),
            can_go_back: Cell::new(false),
            can_go_forward: Cell::new(false),
            cursor: Cell::new(CursorStyle::Arrow),
            browser: RefCell::new(None),
            events: RefCell::new(Vec::new()),
            gpui_view: Cell::new(std::ptr::null_mut()),
            wants_focus: Cell::new(false),
            created: Cell::new(false),
            focused_natively: Cell::new(false),
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
        if self.wants_focus.get() {
            if let Some(host) = browser.host() {
                use cef::ImplBrowserHost as _;
                host.set_focus(1);
            }
        }
        *self.browser.borrow_mut() = Some(browser);
        self.created.set(true);
        // Layout has to run again to place the new view.
        self.dirty.set(true);
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
        if *self.url.borrow() != url {
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

    pub(crate) fn title(&self) -> String {
        self.title.borrow().clone()
    }

    pub(crate) fn url(&self) -> String {
        self.url.borrow().clone()
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
        shared.set_title("A".into());
        shared.set_title("A".into());
        shared.set_load_state(true, false, false);
        shared.set_load_state(false, true, false);
        shared.set_load_state(false, true, false);
        assert_eq!(
            shared.take_events(),
            vec![
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
