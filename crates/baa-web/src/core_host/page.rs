//! One Chrome tab as a gpui entity.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::{c_char, c_void},
    rc::Rc,
};

use gpui::{
    div, AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, Styled, Subscription, WeakEntity, Window,
};
use objc2::runtime::AnyObject;
use serde_json::{json, Value};

use super::{
    c_string,
    cdp::{self, Incoming},
    dispatch::{Dispatch, Live, MEDIA_ACTIONS},
    from_c,
    menu::{present_menu, Row},
    native,
    skin,
    slot::PageSlot,
    ffi::{self, Profile, TabId},
    PageKind, Shared, Snapshot,
};
use crate::{Callback, Device, MediaAction, PageEvent, PageHandle, ProfileId};

const PHONE_USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";

/// What the page asked of its tab's CDP session (`cdp.rs` decides what may be sent).
#[derive(Default)]
struct Cdp {
    /// Ids are per tab.
    last_id: Cell<u64>,
    waiting: RefCell<HashMap<u64, Callback<Result<Value, String>>>>,
    console: Cell<bool>,
    emulation: Cell<Option<Device>>,
    /// The session ended with `console` or `emulation` on: both are sent again
    /// after the next navigation.
    lost: Cell<bool>,
}

/// One Chrome tab as a gpui entity.
pub struct CorePage {
    pub(super) dispatch: &'static Dispatch,
    cdp: Cdp,
    pub(super) shared: Rc<Shared>,
    pub(super) profile: ProfileId,
    state: Snapshot,
    focus_handle: FocusHandle,
    /// gpui's own view, which gets keyboard focus back when the page loses it.
    gpui_view: *mut AnyObject,
    corner_radius: Cell<f32>,
    square_corners: Cell<u8>,
    this: WeakEntity<CorePage>,
    _focus: [Subscription; 2],
}

impl CorePage {
    /// `tab` may be 0 (the app is terminating): the page then renders empty.
    pub(super) fn new(
        dispatch: &'static Dispatch,
        profile: ProfileId,
        handle: *mut Profile,
        tab: TabId,
        kind: PageKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let gpui_view = native::gpui_view(window);
        native::install_mouse_forwarding(gpui_view);
        let (shared, state) = Shared::open(dispatch.core, kind, handle, tab, gpui_view, 0.);
        if tab != 0 {
            dispatch.tabs.borrow_mut().insert(tab, Live { shared: shared.clone(), page: Some(cx.weak_entity()) });
        }
        let focus_handle = cx.focus_handle();
        let focus = subscribe(&focus_handle, window, cx);
        if tab != 0 {
            // An adopted tab already has a url and title: the UI hears them
            // once it has subscribed, which is after this constructor.
            let first = state.clone();
            cx.spawn(async move |page, cx| {
                page.update(cx, |page, cx| page.emit_changes(&Snapshot::default(), &first, cx)).ok();
            })
            .detach();
        }
        Self {
            dispatch,
            cdp: Cdp::default(),
            shared,
            profile,
            state,
            focus_handle,
            gpui_view,
            corner_radius: Cell::new(0.),
            square_corners: Cell::new(0),
            this: cx.weak_entity(),
            _focus: focus,
        }
    }

    /// Diffs a tab snapshot against the last one and reports what changed.
    /// The view was already swapped by the callback.
    pub(super) fn apply(&mut self, new: Snapshot, view_changed: bool, cx: &mut Context<Self>) {
        if self.shared.tab.get() == 0 {
            return;
        }
        let old = std::mem::replace(&mut self.state, new.clone());
        self.emit_changes(&old, &new, cx);
        if new.url != old.url || view_changed {
            self.resubscribe();
        }
        if view_changed {
            cx.notify();
        }
    }

    fn emit_changes(&self, old: &Snapshot, new: &Snapshot, cx: &mut Context<Self>) {
        if new.url != old.url {
            cx.emit(PageEvent::UrlChanged(new.url.clone()));
        }
        if new.title != old.title {
            cx.emit(PageEvent::TitleChanged(new.title.clone()));
        }
        if new.favicon != old.favicon {
            cx.emit(PageEvent::FaviconChanged((!new.favicon.is_empty()).then(|| new.favicon.clone())));
        }
        if new.loading != old.loading {
            cx.emit(PageEvent::LoadingChanged(new.loading));
        }
        if (new.can_go_back, new.can_go_forward) != (old.can_go_back, old.can_go_forward) {
            cx.emit(PageEvent::HistoryChanged { can_go_back: new.can_go_back, can_go_forward: new.can_go_forward });
        }
        if new.audible != old.audible {
            cx.emit(PageEvent::AudibleChanged(new.audible));
        }
        if new.muted != old.muted {
            cx.emit(PageEvent::Muted(new.muted));
        }
        if new.crashed != old.crashed {
            cx.emit(PageEvent::Crashed(new.crashed));
        }
    }

    /// Shows Chrome's menu natively over the page; Chrome is told exactly once.
    pub(super) fn show_context_menu(&mut self, id: u64, x: f32, y: f32, rows: Vec<Row>, cx: &mut Context<Self>) {
        let (app, this) = (cx.to_async(), cx.weak_entity());
        let kind = self.shared.kind;
        present_menu(self.shared.clone(), id, x, y, rows, kind, move || {
            app.update(|cx| {
                this.update(cx, |_, cx| cx.emit(PageEvent::DevToolsRequested { x: x as i32, y: y as i32 })).ok();
            });
        });
    }

    /// A page that never had a tab tells the UI so, once it listens.
    pub(super) fn close_soon(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |page, cx| {
            page.update(cx, |_, cx| cx.emit(PageEvent::Closed)).ok();
        })
        .detach();
    }

    /// Sends `method` if `cdp.rs` allows it; `done` gets the reply's result.
    fn cdp(&self, method: &str, params: Value, done: Option<Callback<Result<Value, String>>>) {
        let id = self.cdp.last_id.get() + 1;
        self.cdp.last_id.set(id);
        let Some(json) = cdp::command(id, method, params) else {
            log::error!("CDP method {method} is not allowed");
            return;
        };
        self.shared.run(|core, tab| {
            if let Some(done) = done {
                self.cdp.waiting.borrow_mut().insert(id, done);
            }
            (core.tab_cdp)(tab, id, c_string(&json).as_ptr());
        });
    }

    /// A reply or event of the tab's CDP session.
    pub(super) fn on_cdp(&mut self, id: u64, json: &str, cx: &mut Context<Self>) {
        match cdp::parse(id, json) {
            Incoming::Reply(id, result) => {
                let done = self.cdp.waiting.borrow_mut().remove(&id);
                // Outside this page's update, so `done` may read the page.
                if let Some(done) = done {
                    cx.defer(move |cx| done(result, cx));
                }
            }
            Incoming::Console { level, text } => cx.emit(PageEvent::Console { level, text }),
            Incoming::Detached => self.cdp.lost.set(self.cdp.console.get() || self.cdp.emulation.get().is_some()),
            Incoming::Other => {}
        }
    }

    fn resubscribe(&self) {
        if self.cdp.lost.replace(false) {
            if self.cdp.console.get() {
                self.cdp("Runtime.enable", json!({}), None);
            }
            if let Some(device) = self.cdp.emulation.get() {
                self.send_emulation(Some(device));
            }
        }
    }

    fn send_emulation(&self, device: Option<Device>) {
        match device {
            Some(Device::Phone) => {
                let metrics = json!({ "width": 393, "height": 852, "deviceScaleFactor": 3, "mobile": true });
                self.cdp("Emulation.setDeviceMetricsOverride", metrics, None);
                self.cdp("Emulation.setTouchEmulationEnabled", json!({ "enabled": true, "maxTouchPoints": 5 }), None);
                let agent = json!({ "userAgent": PHONE_USER_AGENT, "platform": "iPhone" });
                self.cdp("Emulation.setUserAgentOverride", agent, None);
            }
            None => {
                self.cdp("Emulation.clearDeviceMetricsOverride", json!({}), None);
                self.cdp("Emulation.setTouchEmulationEnabled", json!({ "enabled": false }), None);
                // An empty user agent ends the override.
                self.cdp("Emulation.setUserAgentOverride", json!({ "userAgent": "" }), None);
            }
        }
    }

    fn set_browser_focus(&self, focused: bool) {
        let view = self.shared.container.page();
        if focused {
            self.shared.run(|core, tab| {
                (core.tab_focus)(tab);
            });
            PageHandle::activate(self);
        } else if native::has_first_responder(view) {
            native::make_first_responder(self.gpui_view);
        }
    }
}

/// Keyboard focus follows gpui's: the page's view is first responder while
/// the page is focused, so typing and IME go to Chromium natively.
fn subscribe(focus_handle: &FocusHandle, window: &mut Window, cx: &mut Context<CorePage>) -> [Subscription; 2] {
    [
        cx.on_focus_in(focus_handle, window, |page, _, cx| {
            page.set_browser_focus(true);
            cx.emit(PageEvent::Focused);
        }),
        cx.on_focus_out(focus_handle, window, |page, _, _, _| page.set_browser_focus(false)),
    ]
}

impl Drop for CorePage {
    fn drop(&mut self) {
        // The view leaves the window before Chrome destroys it.
        self.shared.container.set_visible(false);
        let tab = self.shared.tab.get();
        self.shared.release();
        if tab != 0 {
            self.dispatch.tabs.borrow_mut().remove(&tab);
            (self.dispatch.core.tab_close)(tab);
        }
    }
}

impl EventEmitter<PageEvent> for CorePage {}

impl Focusable for CorePage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CorePage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus_handle).key_context("Webview").size_full().child(PageSlot {
            shared: self.shared.clone(),
            focus_handle: self.focus_handle.clone(),
            corner_radius: self.corner_radius.get(),
            square_corners: self.square_corners.get(),
        })
    }
}

impl PageHandle for CorePage {
    fn load_url(&self, url: &str) {
        self.shared.run(|core, tab| (core.tab_load_url)(tab, c_string(&skin::to_chrome(url)).as_ptr()));
    }
    fn reload(&self, bypass_cache: bool) {
        self.shared.run(|core, tab| (core.tab_reload)(tab, bypass_cache));
    }
    fn stop(&self) {
        self.shared.run(|core, tab| (core.tab_stop)(tab));
    }
    fn go_back(&self) {
        self.go_to_offset(-1);
    }
    fn go_forward(&self) {
        self.go_to_offset(1);
    }
    fn url(&self) -> String {
        self.state.url.clone()
    }
    fn title(&self) -> String {
        self.state.title.clone()
    }
    fn is_loading(&self) -> bool {
        self.state.loading
    }
    fn can_go_back(&self) -> bool {
        self.state.can_go_back
    }
    fn can_go_forward(&self) -> bool {
        self.state.can_go_forward
    }
    fn history(&self) -> (Vec<(String, String)>, usize) {
        extern "C" fn visit(entries: *mut c_void, title: *const c_char, url: *const c_char) {
            let entries = unsafe { &mut *entries.cast::<Vec<(String, String)>>() };
            entries.push(unsafe { (from_c(title), skin::to_baa(from_c(url))) });
        }
        let mut entries = Vec::new();
        let mut current = 0;
        self.shared.run(|core, tab| {
            current = (core.tab_history)(tab, visit, (&mut entries as *mut Vec<_>).cast());
        });
        (entries, current.max(0) as usize)
    }
    fn go_to_offset(&self, offset: i32) {
        self.shared.run(|core, tab| (core.tab_go_to_offset)(tab, offset));
    }
    fn eval(&self, script: &str) {
        self.shared.run(|core, tab| (core.tab_eval)(tab, c_string(script).as_ptr(), false));
    }
    fn set_zoom(&self, level: f64) {
        self.shared.run(|core, tab| (core.tab_set_zoom)(tab, level));
    }
    fn find(&self, text: &str, forward: bool) {
        self.shared.run(|core, tab| (core.tab_find)(tab, c_string(text).as_ptr(), forward));
    }
    fn open_devtools(&self, inspect_at: Option<(i32, i32)>) {
        let (x, y) = inspect_at.unwrap_or_default();
        self.shared.run(|core, tab| (core.tab_show_devtools_window)(tab, inspect_at.is_some(), x, y));
    }
    fn answer_permission(&self, id: u64, allow: bool) {
        // Ids are global; one Chrome no longer waits on does nothing.
        self.dispatch.core.answer_ok(id, allow);
    }
    fn answer_dialog(&self, id: u64, answer: crate::DialogAnswer) {
        let (text, password) = (c_string(&answer.text), c_string(&answer.password));
        let answer = ffi::Answer { ok: answer.ok, text: text.as_ptr(), password: password.as_ptr(), checked: answer.checked };
        (self.dispatch.core.answer)(id, &answer);
    }
    fn exit_fullscreen(&self) {
        self.shared.run(|core, tab| (core.tab_exit_fullscreen)(tab));
    }
    fn set_group(&self, group: Option<(u64, &str)>) {
        let (key, title) = group.unwrap_or((0, ""));
        self.shared.run(|core, tab| (core.tab_set_group)(tab, key, c_string(title).as_ptr()));
    }
    fn print(&self) {
        self.shared.run(|core, tab| (core.tab_print)(tab));
    }
    fn set_muted(&self, muted: bool) {
        self.shared.run(|core, tab| (core.tab_set_muted)(tab, muted));
    }
    fn is_secure(&self) -> bool {
        self.state.secure
    }
    fn eval_with_gesture(&self, script: &str) {
        self.shared.run(|core, tab| (core.tab_eval)(tab, c_string(script).as_ptr(), true));
    }
    fn set_visible(&self, visible: bool) {
        self.shared.container.set_visible(visible);
    }
    fn activate(&self) {
        // Popups and DevTools have no tab strip.
        if self.shared.kind == PageKind::Tab {
            self.shared.run(|core, tab| (core.tab_activate)(tab));
        }
    }
    fn eval_value(&self, script: &str, done: Callback<Result<Value, String>>) {
        let params = json!({ "expression": script, "returnByValue": true, "awaitPromise": true });
        self.cdp("Runtime.evaluate", params, Some(Box::new(move |result, cx| done(result.and_then(cdp::eval_value), cx))));
    }
    fn screenshot(&self, done: Callback<Result<String, String>>) {
        let read = move |result: Result<Value, String>, cx: &mut App| {
            let png = result.and_then(|r| r["data"].as_str().map(str::to_string).ok_or_else(|| "no image".to_string()));
            done(png, cx)
        };
        self.cdp("Page.captureScreenshot", json!({ "format": "png" }), Some(Box::new(read)));
    }
    fn click(&self, x: f32, y: f32) {
        self.cdp("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": x, "y": y }), None);
        for kind in ["mousePressed", "mouseReleased"] {
            let event = json!({ "type": kind, "x": x, "y": y, "button": "left", "clickCount": 1 });
            self.cdp("Input.dispatchMouseEvent", event, None);
        }
    }
    fn insert_text(&self, text: &str) {
        self.cdp("Input.insertText", json!({ "text": text }), None);
    }
    fn press_key(&self, key: &str) {
        for event in cdp::key_events(key) {
            self.cdp("Input.dispatchKeyEvent", event, None);
        }
    }
    fn set_emulation(&self, device: Option<Device>) {
        if self.cdp.emulation.replace(device) != device {
            self.send_emulation(device);
        }
    }
    fn set_media_action(&self, action: MediaAction) {
        if let Some((bit, _)) = MEDIA_ACTIONS.iter().find(|(_, a)| *a == action) {
            self.shared.run(|core, tab| (core.tab_media_action)(tab, *bit));
        }
    }
    fn watch_console(&self) {
        if !self.cdp.console.replace(true) {
            self.cdp("Runtime.enable", json!({}), None);
        }
    }
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        Focusable::focus_handle(self, cx)
    }
    fn render(&self, _window: &mut Window, _cx: &mut App) -> AnyElement {
        match self.this.upgrade() {
            Some(page) => page.into_any_element(),
            None => div().into_any_element(),
        }
    }
    fn set_corner_radius(&self, radius: f32) {
        self.shared.container.set_corner_radius(radius);
        self.corner_radius.set(radius);
    }
    fn set_square_corners(&self, square: u8) {
        self.shared.container.set_square_corners(square);
        self.square_corners.set(square);
    }
    fn set_floating(&self, floating: bool) {
        self.shared.container.set_floating(floating);
    }
    fn move_to_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let gpui_view = native::gpui_view(window);
        self.gpui_view = gpui_view;
        self.shared.container.move_below(gpui_view);
        native::install_mouse_forwarding(gpui_view);
        self._focus = subscribe(&self.focus_handle, window, cx);
        cx.notify();
    }
}
