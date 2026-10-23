//! Chrome-style pages the app never shows. Its own pages are Alloy-style:
//! they run extensions but have none of Chrome's UI, so Chrome's own pages
//! (chrome://extensions) work only here, hidden, driven by the app; and tabs
//! Chrome opens itself are handed to the app.

use cef::*;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use super::Profile;

wrap_client! {
    struct PlainClient;
    impl Client {}
}

wrap_browser_view_delegate! {
    struct ChromeViewDelegate {}
    impl ViewDelegate {}
    impl BrowserViewDelegate {
        fn browser_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::CHROME
        }
        fn chrome_toolbar_type(&self, _browser_view: Option<&mut BrowserView>) -> ChromeToolbarType {
            ChromeToolbarType::NORMAL
        }
    }
}

wrap_window_delegate! {
    struct HiddenWindowDelegate {
        view: BrowserView,
    }
    impl ViewDelegate {}
    impl PanelDelegate {}
    impl WindowDelegate {
        // Not shown until the app places it.
        fn on_window_created(&self, window: Option<&mut Window>) {
            let Some(window) = window else { return };
            window.add_child_view(Some(&mut View::from(&self.view)));
            window.set_bounds(Some(&Rect { x: -10000, y: -10000, width: 800, height: 600 }));
        }
        fn window_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::CHROME
        }
        fn is_frameless(&self, _window: Option<&mut Window>) -> ::std::os::raw::c_int {
            1
        }
        fn can_close(&self, _window: Option<&mut Window>) -> ::std::os::raw::c_int {
            1
        }
    }
}

type Pending = Rc<RefCell<std::collections::HashMap<i32, Box<dyn FnOnce(Option<String>)>>>>;

wrap_dev_tools_message_observer! {
    struct EvalResults {
        pending: Pending,
    }
    impl DevToolsMessageObserver {
        fn on_dev_tools_method_result(
            &self,
            _browser: Option<&mut Browser>,
            message_id: ::std::os::raw::c_int,
            success: ::std::os::raw::c_int,
            result: Option<&[u8]>,
        ) {
            let done = self.pending.borrow_mut().remove(&message_id);
            if let Some(done) = done {
                let result = result.filter(|_| success != 0).map(|r| String::from_utf8_lossy(r).into_owned());
                done(result);
            }
        }
    }
}

/// A Chrome-style page in a borderless window of its own, hidden until the
/// app puts it over part of its window. Its script has Chrome's own APIs (on
/// `chrome://extensions`: `chrome.developerPrivate`, `chrome.management`),
/// which the app's pages do not, so the app can drive Chrome's settings or
/// show Chrome's own pages as a panel of its own.
#[derive(Clone)]
pub struct ChromePage {
    view: BrowserView,
    window: Window,
    pending: Pending,
    next: Rc<Cell<i32>>,
    registration: Rc<RefCell<Option<Registration>>>,
    /// An init script and the URL to load once it is installed.
    start: Rc<RefCell<Option<(String, String)>>>,
    /// Where it was last put, in screen points; `None` while hidden.
    shown: Rc<Cell<Option<[f64; 4]>>>,
}

impl ChromePage {
    /// Opens `url` on `profile`, with `init_script` run before every
    /// document's own scripts. Call after [`crate::Runtime::start`]; `None`
    /// while the profile's context is still initialising, so retry shortly.
    pub fn open(profile: Option<&Profile>, url: &str, init_script: Option<String>) -> Option<Self> {
        let mut client = PlainClient::new();
        let mut delegate = ChromeViewDelegate::new();
        let mut context = profile.map(|p| p.context.clone());
        let first = if init_script.is_some() { "about:blank" } else { url };
        let view = browser_view_create(
            Some(&mut client),
            Some(&first.into()),
            Some(&BrowserSettings::default()),
            None,
            context.as_mut(),
            Some(&mut delegate),
        )?;
        let window = window_create_top_level(Some(&mut HiddenWindowDelegate::new(view.clone())))?;
        Some(Self {
            view,
            window,
            pending: Default::default(),
            next: Default::default(),
            registration: Default::default(),
            start: Rc::new(RefCell::new(init_script.map(|s| (s, url.to_string())))),
            shown: Default::default(),
        })
    }

    /// The browser's host once it exists, with the DevTools session attached
    /// and the init script in (the page then loads).
    fn host(&self) -> Option<BrowserHost> {
        let browser = self.view.browser()?;
        let host = browser.host()?;
        if self.registration.borrow().is_none() {
            let mut observer = EvalResults::new(self.pending.clone());
            *self.registration.borrow_mut() = host.add_dev_tools_message_observer(Some(&mut observer));
        }
        if let Some((script, url)) = self.start.borrow_mut().take() {
            host.execute_dev_tools_method(0, Some(&"Page.enable".into()), None);
            let mut params = dictionary_value_create()?;
            params.set_string(Some(&"source".into()), Some(&script.as_str().into()));
            let id = self.next_id();
            // The real page loads once Chromium has the script, so it runs there too.
            self.pending.borrow_mut().insert(
                id,
                Box::new(move |_| {
                    if let Some(frame) = browser.main_frame() {
                        frame.load_url(Some(&url.as_str().into()));
                    }
                }),
            );
            host.execute_dev_tools_method(id, Some(&"Page.addScriptToEvaluateOnNewDocument".into()), Some(&mut params));
        }
        Some(host)
    }

    fn next_id(&self) -> i32 {
        let id = self.next.get() + 1;
        self.next.set(id);
        id
    }

    /// Evaluates `expression` in the page, awaiting it if it is a promise.
    /// `done` gets DevTools' `Runtime.evaluate` result as JSON, or `None`
    /// when the page is not up yet. Main thread.
    pub fn eval(&self, expression: &str, done: impl FnOnce(Option<String>) + 'static) {
        let Some(host) = self.host() else {
            return done(None);
        };
        let (Some(mut params), Some(yes)) = (dictionary_value_create(), value_create()) else {
            return done(None);
        };
        params.set_string(Some(&"expression".into()), Some(&expression.into()));
        yes.set_bool(1);
        params.set_value(Some(&"awaitPromise".into()), Some(&mut yes.clone()));
        params.set_value(Some(&"returnByValue".into()), Some(&mut yes.clone()));
        let id = self.next_id();
        self.pending.borrow_mut().insert(id, Box::new(done));
        if host.execute_dev_tools_method(id, Some(&"Runtime.evaluate".into()), Some(&mut params)) == 0 {
            if let Some(done) = self.pending.borrow_mut().remove(&id) {
                done(None);
            }
        }
    }

    /// Whether the page's `prefers-color-scheme` is dark, whatever macOS's
    /// is: a skin's own colours otherwise meet Chrome's dark-mode rules. False
    /// while the page is not up.
    pub fn set_dark(&self, dark: bool) -> bool {
        let Some(host) = self.host() else { return false };
        let scheme = if dark { "dark" } else { "light" };
        let message = format!(
            r#"{{"id":{},"method":"Emulation.setEmulatedMedia","params":{{"features":[{{"name":"prefers-color-scheme","value":"{scheme}"}}]}}}}"#,
            self.next_id()
        );
        host.send_dev_tools_message(Some(message.as_bytes())) != 0
    }

    pub fn load_url(&self, url: &str) {
        if self.start.borrow().is_some() {
            if let Some(start) = &mut *self.start.borrow_mut() {
                start.1 = url.into();
            }
            return;
        }
        if let Some(frame) = self.view.browser().and_then(|b| b.main_frame()) {
            frame.load_url(Some(&url.into()));
        }
    }

    /// Puts the page over `bounds` of `window` (window coordinates), rounded
    /// by `corner_radius`, attached so it moves with it. False until the
    /// page is up; call again.
    pub fn show_over(&self, window: &gpui::Window, bounds: gpui::Bounds<gpui::Pixels>, corner_radius: f32) -> bool {
        if self.host().is_none() {
            return false;
        }
        if self.shown.get().is_none() {
            self.window.show();
        }
        let child = super::native::window_of(self.window.window_handle().cast());
        let frame = super::native::attach_over(child, super::native::gpui_view(window), bounds, corner_radius);
        self.shown.set(frame);
        frame.is_some()
    }

    pub fn hide(&self) {
        if self.shown.take().is_some() {
            super::native::detach_window(super::native::window_of(self.window.window_handle().cast()));
            self.window.hide();
        }
    }
}

type OpenTab = Box<dyn Fn(String)>;

thread_local! {
    static ON_CHROME_TAB: std::cell::RefCell<Option<OpenTab>> = const { std::cell::RefCell::new(None) };
    /// Chrome's tabs waiting to be closed: closing one mid-navigation, while
    /// Chrome is still setting up its window, crashes it.
    static CLOSING: std::cell::RefCell<Vec<Browser>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Closes the Chrome tabs handed to the app since the last call. Main thread,
/// outside CEF's callbacks.
pub(crate) fn close_handed_tabs() {
    for browser in CLOSING.with(|c| std::mem::take(&mut *c.borrow_mut())) {
        if let Some(host) = browser.host() {
            host.close_browser(1);
        }
    }
}

/// Chrome opens tabs of its own: an extension's `chrome.tabs.create` (a
/// welcome page after install, its options), or a link in the Chrome-style
/// window. Each is closed before it loads and its URL handed to `open`, so
/// the app can open it as one of its own pages instead. Main thread.
pub fn on_chrome_tab(open: impl Fn(String) + 'static) {
    ON_CHROME_TAB.with(|f| *f.borrow_mut() = Some(Box::new(open)));
}

wrap_client! {
    pub(crate) struct ChromeTabClient;
    impl Client {
        fn request_handler(&self) -> Option<RequestHandler> {
            Some(ChromeTabRequests::new())
        }
    }
}

wrap_request_handler! {
    struct ChromeTabRequests;
    impl RequestHandler {
        fn on_before_browse(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _user_gesture: ::std::os::raw::c_int,
            _is_redirect: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            if frame.is_none_or(|f| f.is_main() == 0) {
                return 0;
            }
            let url = request.map(|r| CefString::from(&r.url()).to_string()).unwrap_or_default();
            // A blank start is followed by the real address. The hidden pages
            // have their own client, so chrome:// pages here are for the app
            // too (it draws its own for most).
            let newtab = url.starts_with("chrome://newtab") || url.starts_with("chrome://new-tab-page");
            if url.is_empty() || url == "about:blank" {
                return 0;
            }
            // A new window's own New Tab has nothing for the app; it just closes.
            if !newtab {
                ON_CHROME_TAB.with(|f| {
                    if let Some(open) = &*f.borrow() {
                        open(url)
                    }
                });
            }
            if let Some(browser) = browser {
                CLOSING.with(|c| c.borrow_mut().push(browser.clone()));
                super::pump::schedule_work(0);
            }
            1
        }
    }
}
