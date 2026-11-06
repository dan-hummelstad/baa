//! The dispatch and page logic against a fake `Core`: Rust stubs record what
//! BaaCore would have been asked, tabs are fake pointers.

use std::{cell::RefCell, ffi::CString};

use gpui::{div, IntoElement, Render, TestAppContext, WindowHandle};
use objc2::{class, msg_send};

use std::{
    cell::Cell,
    ffi::{c_char, c_int, c_void, CStr},
    sync::Arc,
    time::Duration,
};

use gpui::{Context, Entity, Window};
use objc2::runtime::AnyObject;

use super::{
    dispatch::{callbacks, Dispatch, ADOPTION_EXPIRY},
    ffi::{Callbacks, Core, Profile, Tab},
    from_c,
    host::CoreWebHost,
    menu::{menu_answer, menu_tree, Row},
    page::CorePage,
    *,
};
use crate::{
    DialogKind, DownloadState, ExtensionChange, MenuItem, PageEvent, PageHandle, Permission, PermissionState,
    ProfileId, SiteAccess, WebHost,
};

thread_local! {
    static CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static NEXT: Cell<usize> = const { Cell::new(0x1000) };
    /// What the fake `baa_tab_get_state` reports as the tab's view and url.
    static FIRST_VIEW: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) };
    static FIRST_URL: Cell<&'static CStr> = const { Cell::new(c"") };
    static QUITTING: Cell<bool> = const { Cell::new(false) };
    static SITE_STATE: Cell<c_int> = const { Cell::new(0) };
    static POPUP_NULL: Cell<bool> = const { Cell::new(false) };
    /// What the fake `baa_profile_extensions` lists.
    static EXTENSIONS: RefCell<Vec<FakeExtension>> = const { RefCell::new(Vec::new()) };
}

fn log(call: String) {
    CALLS.with(|c| c.borrow_mut().push(call));
}

fn calls(prefix: &str) -> Vec<String> {
    CALLS.with(|c| c.borrow().iter().filter(|c| c.starts_with(prefix)).cloned().collect())
}

fn fresh<T>() -> *mut T {
    NEXT.with(|n| n.replace(n.get() + 0x10)) as *mut T
}

fn arg(ptr: *const c_char) -> String {
    unsafe { from_c(ptr) }
}

extern "C" fn install_app(_: c_int, _: *const *const c_char, _: *const Callbacks, _: *mut c_void) {}
extern "C" fn exit_code() -> c_int {
    0
}
extern "C" fn is_quitting() -> bool {
    QUITTING.with(Cell::get)
}
extern "C" fn profile_open(name: *const c_char) -> *mut Profile {
    log(format!("profile_open {}", arg(name)));
    fresh()
}
extern "C" fn profile_open_ephemeral() -> *mut Profile {
    log("profile_open_ephemeral".into());
    fresh()
}
extern "C" fn profile_close(profile: *mut Profile) {
    log(format!("profile_close {profile:?}"));
}
extern "C" fn tab_create(profile: *mut Profile, url: *const c_char) -> *mut Tab {
    log(format!("tab_create {profile:?} {}", arg(url)));
    fresh()
}
extern "C" fn tab_close(tab: *mut Tab) {
    log(format!("tab_close {tab:?}"));
}
extern "C" fn profile_set_pref_bool(profile: *mut Profile, name: *const c_char, value: bool) {
    log(format!("profile_set_pref_bool {profile:?} {} {value}", arg(name)));
}
extern "C" fn profile_site_permission(_: *mut Profile, url: *const c_char, kind: u32) -> c_int {
    log(format!("profile_site_permission {} {kind}", arg(url)));
    SITE_STATE.with(Cell::get)
}
extern "C" fn profile_set_site_permission(_: *mut Profile, url: *const c_char, kind: u32, state: c_int) {
    log(format!("profile_set_site_permission {} {kind} {state}", arg(url)));
}
extern "C" fn profile_fetch(_: *mut Profile, url: *const c_char, id: u64) {
    log(format!("profile_fetch {} {id}", arg(url)));
}
extern "C" fn profile_extensions(_: *mut Profile, visit: ffi::ExtensionVisitor, ctx: *mut c_void) -> c_int {
    EXTENSIONS.with(|list| {
        for extension in list.borrow().iter() {
            visit(ctx, &extension.raw().0);
        }
        list.borrow().len() as c_int
    })
}
extern "C" fn profile_extension_change(_: *mut Profile, id: *const c_char, change: c_int, value: c_int) -> bool {
    log(format!("profile_extension_change {} {change} {value}", arg(id)));
    true
}
extern "C" fn extension_popup(_: *mut Profile, id: *const c_char) -> *mut Tab {
    log(format!("extension_popup {}", arg(id)));
    if POPUP_NULL.with(Cell::get) { std::ptr::null_mut() } else { fresh() }
}
extern "C" fn tab_set_init_script(t: *mut Tab, script: *const c_char) {
    log(format!("tab_set_init_script {t:?} {}", arg(script)));
}
extern "C" fn tab_set_color_scheme(t: *mut Tab, scheme: c_int) {
    log(format!("tab_set_color_scheme {t:?} {scheme}"));
}
extern "C" fn tab_open_devtools(t: *mut Tab, inspect: bool, x: c_int, y: c_int) -> *mut Tab {
    log(format!("tab_open_devtools {t:?} {inspect} {x} {y}"));
    fresh()
}
extern "C" fn tab_show_devtools_window(t: *mut Tab, inspect: bool, x: c_int, y: c_int) {
    log(format!("tab_show_devtools_window {t:?} {inspect} {x} {y}"));
}
extern "C" fn permission_answer(id: u64, allow: bool) {
    log(format!("permission_answer {id} {allow}"));
}
extern "C" fn context_menu_done(id: u64, command: c_int) {
    log(format!("context_menu_done {id} {command}"));
}
extern "C" fn download_cancel(id: u32) {
    log(format!("download_cancel {id}"));
}
extern "C" fn tab_get_state(_: *mut Tab, state: *mut TabState) -> bool {
    unsafe {
        (*state).url = FIRST_URL.with(Cell::get).as_ptr();
        (*state).view = FIRST_VIEW.with(Cell::get).cast();
        (*state).title = c"".as_ptr();
        (*state).favicon_url = c"".as_ptr();
    }
    true
}
extern "C" fn tab_load_url(tab: *mut Tab, url: *const c_char) {
    log(format!("tab_load_url {tab:?} {}", arg(url)));
}
fn note(name: &str, tab: *mut Tab) {
    log(format!("{name} {tab:?}"));
}
extern "C" fn tab_reload(t: *mut Tab) {
    note("tab_reload", t)
}
extern "C" fn tab_stop(t: *mut Tab) {
    note("tab_stop", t)
}
extern "C" fn tab_go_to_offset(t: *mut Tab, offset: c_int) {
    log(format!("tab_go_to_offset {t:?} {offset}"));
}
extern "C" fn tab_history(_: *mut Tab, visit: ffi::HistoryVisitor, ctx: *mut c_void) -> c_int {
    visit(ctx, c"One".as_ptr(), c"https://one".as_ptr());
    visit(ctx, c"Two".as_ptr(), c"https://two".as_ptr());
    1
}
extern "C" fn tab_eval(t: *mut Tab, script: *const c_char, gesture: bool) {
    log(format!("tab_eval {t:?} {} {gesture}", arg(script)));
}
extern "C" fn tab_set_zoom(t: *mut Tab, factor: f64) {
    log(format!("tab_set_zoom {t:?} {factor}"));
}
extern "C" fn tab_find(t: *mut Tab, text: *const c_char, forward: bool) {
    log(format!("tab_find {t:?} {} {forward}", arg(text)));
}
extern "C" fn tab_print(t: *mut Tab) {
    note("tab_print", t)
}
extern "C" fn tab_set_muted(t: *mut Tab, muted: bool) {
    log(format!("tab_set_muted {t:?} {muted}"));
}
extern "C" fn tab_activate(t: *mut Tab) {
    note("tab_activate", t)
}
extern "C" fn tab_focus(t: *mut Tab) {
    note("tab_focus", t)
}
extern "C" fn dialog_answer(id: u64, ok: bool, text: *const c_char) {
    log(format!("dialog_answer {id} {ok} {}", arg(text)));
}

fn fake_core() -> &'static Core {
    Box::leak(Box::new(Core {
        install_app,
        exit_code,
        is_quitting,
        profile_open,
        profile_open_ephemeral,
        profile_close,
        profile_set_pref_bool,
        profile_site_permission,
        profile_set_site_permission,
        profile_fetch,
        profile_extensions,
        profile_extension_change,
        extension_popup,
        tab_create,
        tab_close,
        tab_get_state,
        tab_load_url,
        tab_reload,
        tab_stop,
        tab_go_to_offset,
        tab_history,
        tab_eval,
        tab_set_zoom,
        tab_find,
        tab_print,
        tab_set_muted,
        tab_activate,
        tab_focus,
        tab_set_init_script,
        tab_set_color_scheme,
        tab_open_devtools,
        tab_show_devtools_window,
        permission_answer,
        context_menu_done,
        download_cancel,
        dialog_answer,
    }))
}

struct FakeExtension {
    id: &'static str,
    name: &'static str,
    permissions: &'static str,
    site_access: c_int,
    popup: &'static str,
    enabled: bool,
}

impl FakeExtension {
    fn raw(&self) -> (ffi::Extension, Vec<CString>) {
        let text = |s: &str| CString::new(s).unwrap();
        let strings = vec![
            text(self.id),
            text(self.name),
            text(""),
            text("1.0"),
            text(""),
            text(self.popup),
            text(""),
            text(self.permissions),
        ];
        let p = |i: usize| strings[i].as_ptr();
        let raw = ffi::Extension {
            id: p(0),
            name: p(1),
            description: p(2),
            version: p(3),
            icon_path: p(4),
            popup_url: p(5),
            options_url: p(6),
            permissions: p(7),
            site_access: self.site_access,
            enabled: self.enabled,
        };
        (raw, strings)
    }
}

type Fetched = (u32, Option<String>);

struct Root;

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

struct Env {
    host: CoreWebHost,
    dispatch: &'static Dispatch,
    callbacks: Callbacks,
    window: WindowHandle<Root>,
}

impl Env {
    fn new(cx: &mut TestAppContext) -> Self {
        CALLS.with(|c| c.borrow_mut().clear());
        FIRST_VIEW.with(|v| v.set(std::ptr::null_mut()));
        FIRST_URL.with(|u| u.set(c""));
        QUITTING.with(|q| q.set(false));
        SITE_STATE.with(|s| s.set(0));
        POPUP_NULL.with(|p| p.set(false));
        EXTENSIONS.with(|e| e.borrow_mut().clear());
        let core = fake_core();
        let dispatch: &'static Dispatch = Box::leak(Box::new(Dispatch::new(core)));
        let host = cx.update(|cx| CoreWebHost::new(core, dispatch, std::env::temp_dir().join("baa-core-host-test"), cx));
        let window = cx.add_window(|_, _| Root);
        Self { host, dispatch, callbacks: callbacks(), window }
    }

    fn ctx(&self) -> *mut c_void {
        self.dispatch as *const Dispatch as *mut c_void
    }

    fn page(&self, profile: u64, url: &str, cx: &mut TestAppContext) -> Entity<CorePage> {
        self.window.update(cx, |_, window, cx| self.host.create_page(ProfileId(profile), url, window, cx)).unwrap()
    }

    fn tab_of(&self, page: &Entity<CorePage>, cx: &mut TestAppContext) -> *mut Tab {
        page.read_with(cx, |page, _| page.shared.tab.get())
    }

    fn state(&self, tab: *mut Tab, state: &State) {
        let owned = state.raw();
        (self.callbacks.tab_state)(self.ctx(), tab, &owned.0);
    }

    fn created(&self, tab: *mut Tab, opener: *mut Tab, profile: ProfileId, url: &str) {
        self.created_in(tab, opener, profile, url, false);
    }

    fn created_in(&self, tab: *mut Tab, opener: *mut Tab, profile: ProfileId, url: &str, background: bool) {
        let url = CString::new(url).unwrap();
        let handle = self.host.profile_handle(profile);
        (self.callbacks.tab_created)(self.ctx(), tab, opener, handle, url.as_ptr(), background, true);
    }

    /// A page whose tab starts with `view` in it.
    fn page_with_view(&self, view: *mut AnyObject, cx: &mut TestAppContext) -> Entity<CorePage> {
        FIRST_VIEW.with(|v| v.set(view));
        self.page(1, "https://a.test", cx)
    }
}

fn events(page: &Entity<CorePage>, cx: &mut TestAppContext) -> Rc<RefCell<Vec<PageEvent>>> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    cx.update(|cx| cx.subscribe(page, move |_, event: &PageEvent, _| sink.borrow_mut().push(event.clone())).detach());
    seen
}

#[derive(Clone, Default)]
struct State {
    url: &'static str,
    title: &'static str,
    favicon: &'static str,
    loading: bool,
    can_go_back: bool,
    audible: bool,
    view: usize,
}

impl State {
    /// The C struct and the strings it borrows.
    fn raw(&self) -> (TabState, [CString; 3]) {
        let strings = [self.url, self.title, self.favicon].map(|s| CString::new(s).unwrap());
        let state = TabState {
            url: strings[0].as_ptr(),
            title: strings[1].as_ptr(),
            favicon_url: strings[2].as_ptr(),
            view: self.view as *mut c_void,
            loading: self.loading,
            can_go_back: self.can_go_back,
            can_go_forward: false,
            audible: self.audible,
            secure: false,
        };
        (state, strings)
    }
}

/// A real NSView standing in for Chromium's tab view.
fn new_view() -> *mut AnyObject {
    unsafe { msg_send![class!(NSView), new] }
}

fn superview(view: *mut AnyObject) -> *mut AnyObject {
    unsafe { msg_send![view, superview] }
}

fn container_of(page: &Entity<CorePage>, cx: &mut TestAppContext) -> *mut AnyObject {
    page.read_with(cx, |page, _| page.shared.container.as_ptr())
}

mod adoption;
mod devtools_tests;
mod dialogs;
mod extension_tests;
mod menus;
mod pages;
mod profile_tests;
