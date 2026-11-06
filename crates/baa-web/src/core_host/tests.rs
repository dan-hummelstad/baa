//! The dispatch and page logic against a fake `Core`: Rust stubs record what
//! BaaCore would have been asked, tabs are fake pointers.

use std::{cell::RefCell, ffi::CString};

use gpui::{div, IntoElement, Render, TestAppContext, WindowHandle};
use objc2::{class, msg_send};

use super::*;
use crate::SiteAccess;

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

#[gpui::test]
fn tab_state_becomes_exactly_the_changed_events(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    let loading = State { url: "https://a.test/", loading: true, ..Default::default() };
    env.state(tab, &loading);
    env.state(tab, &loading);
    cx.run_until_parked();
    assert_eq!(
        *seen.borrow(),
        [PageEvent::UrlChanged("https://a.test/".into()), PageEvent::LoadingChanged(true)],
        "an unchanged snapshot says nothing"
    );

    seen.borrow_mut().clear();
    let done = State {
        url: "https://a.test/",
        title: "A",
        favicon: "https://a.test/i.png",
        can_go_back: true,
        audible: true,
        ..Default::default()
    };
    env.state(tab, &done);
    cx.run_until_parked();
    assert_eq!(
        *seen.borrow(),
        [
            PageEvent::TitleChanged("A".into()),
            PageEvent::FaviconChanged(Some("https://a.test/i.png".into())),
            PageEvent::LoadingChanged(false),
            PageEvent::HistoryChanged { can_go_back: true, can_go_forward: false },
            PageEvent::AudibleChanged(true),
        ]
    );

    seen.borrow_mut().clear();
    env.state(tab, &State { favicon: "", ..done });
    let png = [0x89, b'P', b'N', b'G'];
    (env.callbacks.tab_favicon)(env.ctx(), tab, c"https://a.test/i.png".as_ptr(), png.as_ptr(), png.len());
    cx.run_until_parked();
    assert_eq!(
        *seen.borrow(),
        [
            PageEvent::FaviconChanged(None),
            PageEvent::FaviconImage { url: "https://a.test/i.png".into(), png: Arc::new(png.to_vec()) },
        ]
    );
    page.read_with(cx, |page, _| assert_eq!(page.url(), "https://a.test/"));
}

#[gpui::test]
fn chrome_made_tabs_go_to_their_opener_or_the_tab_list(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let opener = env.page(1, "https://a.test", cx);
    let opener_tab = env.tab_of(&opener, cx);
    let seen = events(&opener, cx);

    env.created(0x9000 as *mut Tab, opener_tab, ProfileId(1), "https://popup.test");
    cx.run_until_parked();
    assert_eq!(
        *seen.borrow(),
        [PageEvent::OpenRequested { url: "https://popup.test".into(), from_user_gesture: true, background: false }]
    );

    env.created(0x9010 as *mut Tab, std::ptr::null_mut(), ProfileId(1), "https://ext.test");
    cx.run_until_parked();
    assert_eq!(env.host.chrome_tabs().read_with(cx, |tabs, _| tabs.0.clone()), ["https://ext.test"]);
    assert_eq!(seen.borrow().len(), 1, "no opener, nobody to ask");
}

#[gpui::test]
fn a_page_for_the_same_profile_and_url_adopts_the_tab(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.created(0x9000 as *mut Tab, std::ptr::null_mut(), ProfileId(1), "https://popup.test");
    let creates = calls("tab_create").len();

    let adopted = env.page(1, "https://popup.test", cx);
    assert_eq!(env.tab_of(&adopted, cx), 0x9000 as *mut Tab);
    assert_eq!(calls("tab_create").len(), creates, "no second tab");

    let other = env.page(1, "https://other.test", cx);
    assert_ne!(env.tab_of(&other, cx), 0x9000 as *mut Tab);
    assert_eq!(calls("tab_create").len(), creates + 1);

    // Adopted tabs are not closed when their timer runs out.
    cx.executor().advance_clock(ADOPTION_EXPIRY + Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(calls("tab_close"), Vec::<String>::new());
}

#[gpui::test]
fn an_unadopted_tab_is_closed_after_the_expiry(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.created(0x9000 as *mut Tab, std::ptr::null_mut(), ProfileId(1), "https://nobody.test");
    cx.executor().advance_clock(ADOPTION_EXPIRY - Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(calls("tab_close"), Vec::<String>::new());

    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(calls("tab_close"), [format!("tab_close {:?}", 0x9000 as *mut Tab)]);
    // Too late to adopt it now.
    let late = env.page(1, "https://nobody.test", cx);
    assert_ne!(env.tab_of(&late, cx), 0x9000 as *mut Tab);
}

#[gpui::test]
fn a_tab_in_a_profile_baa_never_opened_is_closed_and_its_url_kept(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let url = c"https://x.test";
    (env.callbacks.tab_created)(env.ctx(), 0x9000 as *mut Tab, std::ptr::null_mut(), std::ptr::null_mut(), url.as_ptr(), false, false);
    cx.run_until_parked();
    assert_eq!(calls("tab_close"), [format!("tab_close {:?}", 0x9000 as *mut Tab)]);
    assert_eq!(env.host.chrome_tabs().read_with(cx, |tabs, _| tabs.0.clone()), ["https://x.test"]);
}

#[gpui::test]
fn dialogs_are_answered_once(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    (env.callbacks.dialog)(env.ctx(), tab, 7, 1, c"Sure?".as_ptr(), c"".as_ptr());
    cx.run_until_parked();
    assert_eq!(
        *seen.borrow(),
        [PageEvent::Dialog { kind: DialogKind::Confirm, message: "Sure?".into(), default_text: String::new() }]
    );
    page.read_with(cx, |page, _| {
        page.answer_dialog(true, "yes");
        page.answer_dialog(false, "");
    });
    assert_eq!(calls("dialog_answer"), ["dialog_answer 7 true yes"]);

    (env.callbacks.dialog)(env.ctx(), tab, 8, 0, c"Hi".as_ptr(), c"".as_ptr());
    (env.callbacks.dialog_closed)(env.ctx(), tab, 8);
    cx.run_until_parked();
    assert_eq!(seen.borrow().last(), Some(&PageEvent::DialogClosed));
    page.read_with(cx, |page, _| page.answer_dialog(true, ""));
    assert_eq!(calls("dialog_answer").len(), 1, "the page went away first");
}

#[gpui::test]
fn dropping_a_page_closes_its_tab_once_and_unknown_tabs_are_ignored(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    (env.callbacks.tab_closed)(env.ctx(), 0xdead0 as *mut Tab);
    cx.run_until_parked();
    assert!(seen.borrow().is_empty());

    cx.update(|_| drop(page));
    assert_eq!(calls("tab_close"), [format!("tab_close {tab:?}")]);
    (env.callbacks.tab_closed)(env.ctx(), tab);
    cx.run_until_parked();
    assert_eq!(calls("tab_close").len(), 1);
}

#[gpui::test]
fn a_tab_chrome_closed_says_so_and_is_not_closed_again(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    (env.callbacks.tab_closed)(env.ctx(), tab);
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [PageEvent::Closed]);
    page.read_with(cx, |page, _| page.reload());
    assert_eq!(calls("tab_reload"), Vec::<String>::new(), "the handle is dead");
    cx.update(|_| drop(page));
    assert_eq!(calls("tab_close"), Vec::<String>::new());
}

#[gpui::test]
fn ephemeral_profiles_count_down_and_open_once_each(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let (first, second) = (env.host.ephemeral_profile(), env.host.ephemeral_profile());
    assert_eq!((first.0, second.0), (u64::MAX, u64::MAX - 1));

    env.page(first.0, "https://a.test", cx);
    env.page(second.0, "https://b.test", cx);
    env.page(first.0, "https://c.test", cx);
    assert_eq!(calls("profile_open_ephemeral").len(), 2);
    assert_eq!(calls("profile_open ").len(), 0, "no disk profile");

    env.page(3, "https://d.test", cx);
    assert_eq!(calls("profile_open ").as_slice(), ["profile_open space-3"]);
}

#[gpui::test]
fn page_calls_reach_the_tab(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    page.read_with(cx, |page, _| {
        page.load_url("https://b.test");
        page.go_back();
        page.eval_with_gesture("x()");
        assert_eq!(page.history(), (vec![("One".into(), "https://one".into()), ("Two".into(), "https://two".into())], 1));
    });
    assert_eq!(calls("tab_load_url"), [format!("tab_load_url {tab:?} https://b.test")]);
    assert_eq!(calls("tab_go_to_offset"), [format!("tab_go_to_offset {tab:?} -1")]);
    assert_eq!(calls("tab_eval"), [format!("tab_eval {tab:?} x() true")]);
}

#[test]
fn chrome_gets_user_flags_our_switches_and_no_urls() {
    let args: Vec<String> = ["baa", "https://x.test", "--proxy-server=p", "--disable-features=Foo"].map(Into::into).into();
    let argv = chrome_argv(&args, Path::new("/data/chromium"));
    assert_eq!(argv[0], "baa");
    assert_eq!(argv[1], "--proxy-server=p");
    for flag in ["--no-startup-window", "--no-first-run", "--user-data-dir=/data/chromium", "--no-pings", "--disable-background-networking"] {
        assert_eq!(argv.iter().filter(|a| *a == flag).count(), 1, "{flag}");
    }
    assert!(!argv.iter().any(|a| a.contains("x.test")));
    let disabled: Vec<_> = argv.iter().filter(|a| a.starts_with("--disable-features=")).collect();
    assert_eq!(disabled.len(), 1);
    assert!(disabled[0].contains("Foo") && disabled[0].contains("MediaRouter"));

    let own = chrome_argv(&["baa".into(), "--user-data-dir=/mine".into()], Path::new("/data/chromium"));
    assert_eq!(own.iter().filter(|a| a.starts_with("--user-data-dir=")).collect::<Vec<_>>(), ["--user-data-dir=/mine"]);
}

/// A real NSView standing in for Chromium's tab view.
fn new_view() -> *mut AnyObject {
    unsafe { msg_send![class!(NSView), new] }
}

fn superview(view: *mut AnyObject) -> *mut AnyObject {
    unsafe { msg_send![view, superview] }
}

fn container_of(page: &Entity<CorePage>, cx: &mut TestAppContext) -> *mut AnyObject {
    page.read_with(cx, |page, _| page.container.as_ptr())
}

#[gpui::test]
fn a_replaced_view_is_swapped_in_inside_the_callback(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let (old, new) = (new_view(), new_view());
    let page = env.page_with_view(old, cx);
    let container = container_of(&page, cx);
    assert_eq!(superview(old), container);

    // Chrome frees `old` when the callback returns: it must already be out.
    env.state(env.tab_of(&page, cx), &State { view: new as usize, ..Default::default() });
    assert!(superview(old).is_null());
    assert_eq!(superview(new), container);
    page.read_with(cx, |page, _| assert_eq!(page.shared.view.get(), new));
}

#[gpui::test]
fn a_tab_chrome_closed_lets_go_of_its_view_at_once(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let view = new_view();
    let page = env.page_with_view(view, cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    (env.callbacks.tab_closed)(env.ctx(), tab);
    assert!(superview(view).is_null());
    page.read_with(cx, |page, _| assert!(page.shared.view.get().is_null()));
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [PageEvent::Closed]);
    cx.update(|_| drop(page));
    assert_eq!(calls("tab_close"), Vec::<String>::new());
}

#[gpui::test]
fn quitting_releases_views_but_keeps_the_session(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let view = new_view();
    let page = env.page_with_view(view, cx);
    let seen = events(&page, cx);

    QUITTING.with(|q| q.set(true));
    (env.callbacks.tab_closed)(env.ctx(), env.tab_of(&page, cx));
    cx.run_until_parked();
    assert!(superview(view).is_null());
    assert!(seen.borrow().is_empty(), "no Closed: the tab is still in the session");
}

#[gpui::test]
fn deleting_a_profile_lets_go_of_its_pages_and_pending_tabs(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let view = new_view();
    let page = env.page_with_view(view, cx);
    env.created(0x9000 as *mut Tab, std::ptr::null_mut(), ProfileId(1), "https://p.test");

    env.host.delete_profile(ProfileId(1));
    assert!(superview(view).is_null());
    assert_eq!(calls("profile_close").len(), 1);
    cx.update(|_| drop(page));
    cx.executor().advance_clock(ADOPTION_EXPIRY + Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(calls("tab_close"), Vec::<String>::new(), "Chrome owns those tabs' end now");
}

#[gpui::test]
fn a_foreground_open_nobody_adopts_is_closed_this_turn_but_background_waits(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let opener = env.page(1, "https://a.test", cx);
    let opener_tab = env.tab_of(&opener, cx);

    env.created_in(0x9000 as *mut Tab, opener_tab, ProfileId(1), "https://fg.test", false);
    env.created_in(0x9010 as *mut Tab, opener_tab, ProfileId(1), "https://bg.test", true);
    cx.run_until_parked();
    assert_eq!(calls("tab_close"), [format!("tab_close {:?}", 0x9000 as *mut Tab)]);

    cx.executor().advance_clock(ADOPTION_EXPIRY + Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(calls("tab_close").len(), 2);
}

#[gpui::test]
fn an_adopted_tabs_current_state_reaches_a_late_subscriber(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    FIRST_URL.with(|u| u.set(c"https://adopted.test/"));
    let page = env.page(1, "https://adopted.test/", cx);
    let seen = events(&page, cx);
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [PageEvent::UrlChanged("https://adopted.test/".into())]);
}

#[gpui::test]
fn devtools_never_adopt_a_pending_tab(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let of = env.page(1, "https://a.test", cx);
    env.created(0x9000 as *mut Tab, std::ptr::null_mut(), ProfileId(1), "about:blank");
    let creates = calls("tab_create").len();
    let devtools = env
        .window
        .update(cx, |_, window, cx| env.host.create_devtools(&of, None, "right", window, cx))
        .unwrap();
    assert_eq!(calls("tab_create").len(), creates, "Chrome makes the front-end itself");
    assert_eq!(calls("tab_open_devtools").len(), 1);
    assert_ne!(env.tab_of(&devtools, cx), 0x9000 as *mut Tab);
}

#[gpui::test]
fn permissions_arrive_in_bit_order_and_are_answered_through_chrome(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    (env.callbacks.permission)(env.ctx(), tab, 5, c"https://a.test".as_ptr(), 0b100_0011);
    (env.callbacks.permission_closed)(env.ctx(), tab, 5);
    cx.run_until_parked();
    assert_eq!(
        *seen.borrow(),
        [
            PageEvent::PermissionRequested {
                id: 5,
                origin: "https://a.test".into(),
                kinds: vec![Permission::Camera, Permission::Microphone, Permission::Other],
            },
            PageEvent::PermissionDismissed(5),
        ]
    );
    page.read_with(cx, |page, _| page.answer_permission(6, true));
    assert_eq!(calls("permission_answer"), ["permission_answer 6 true"]);
}

#[gpui::test]
fn beforeunload_kinds_and_the_quit_fallback(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    (env.callbacks.dialog)(env.ctx(), tab, 1, 3, c"Leave?".as_ptr(), c"".as_ptr());
    (env.callbacks.dialog)(env.ctx(), tab, 2, 4, c"Reload?".as_ptr(), c"".as_ptr());
    cx.run_until_parked();
    let kinds: Vec<_> = seen
        .borrow()
        .iter()
        .map(|e| match e {
            PageEvent::Dialog { kind, .. } => *kind,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(kinds, [DialogKind::BeforeUnload { is_reload: false }, DialogKind::BeforeUnload { is_reload: true }]);

    QUITTING.with(|q| q.set(true));
    seen.borrow_mut().clear();
    (env.callbacks.dialog)(env.ctx(), tab, 3, 3, c"Leave?".as_ptr(), c"".as_ptr());
    assert_eq!(calls("dialog_answer"), Vec::<String>::new(), "not inside Chrome's dialog code");
    cx.run_until_parked();
    assert!(seen.borrow().is_empty(), "nobody is asked while quitting");
    assert_eq!(calls("dialog_answer"), ["dialog_answer 3 true "]);
}

#[gpui::test]
fn site_permissions_go_through_the_profile(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    SITE_STATE.with(|s| s.set(2));
    assert_eq!(env.host.site_permission(ProfileId(1), "https://a.test", Permission::Location), PermissionState::Block);
    assert_eq!(calls("profile_site_permission"), ["profile_site_permission https://a.test 4"]);
    SITE_STATE.with(|s| s.set(1));
    assert_eq!(env.host.site_permission(ProfileId(1), "https://a.test", Permission::Camera), PermissionState::Allow);

    env.host.set_site_permission(ProfileId(1), "https://a.test", Permission::Notifications, PermissionState::Ask);
    env.host.set_site_permission(ProfileId(1), "https://a.test", Permission::Other, PermissionState::Block);
    assert_eq!(calls("profile_set_site_permission"), ["profile_set_site_permission https://a.test 8 0"]);
    assert_eq!(env.host.site_permission(ProfileId(1), "https://a.test", Permission::Other), PermissionState::Ask);
}

fn row(label: &str, command: c_int, depth: u8, kind: u8) -> Row {
    Row { label: label.into(), command, depth, kind, enabled: true, checked: false }
}

fn sample_rows() -> Vec<Row> {
    vec![
        row("Open link", 100, 0, 0),
        row("", -1, 0, ffi::MENU_SEPARATOR),
        row("Share", -1, 0, ffi::MENU_SUBMENU),
        row("Mail", 200, 1, 0),
        row("More", -1, 1, ffi::MENU_SUBMENU),
        row("Deep", 201, 2, 0),
        row("Copy", 101, 0, 0),
        row("Inspect", 300, 0, ffi::MENU_INSPECT),
    ]
}

#[test]
fn menu_rows_become_a_tree_with_the_row_index_as_id() {
    let action = |label: &str, id| MenuItem::Action { label: label.into(), enabled: true, checked: false, id };
    assert_eq!(
        menu_tree(&sample_rows()),
        [
            action("Open link", 0),
            MenuItem::Separator,
            MenuItem::Submenu {
                label: "Share".into(),
                items: vec![
                    action("Mail", 3),
                    MenuItem::Submenu { label: "More".into(), items: vec![action("Deep", 5)] },
                ],
            },
            action("Copy", 6),
            action("Inspect", 7),
        ]
    );
    assert_eq!(menu_tree(&[]), []);
    assert_eq!(menu_tree(&[row("lost", 1, 1, 0)]), [], "a row below nothing ends the menu");
}

#[test]
fn a_picked_row_runs_unless_it_is_inspect_on_a_tab() {
    let rows = sample_rows();
    assert_eq!(menu_answer(&rows, Some(6), PageKind::Tab), (101, false));
    assert_eq!(menu_answer(&rows, Some(5), PageKind::Popup), (201, false));
    assert_eq!(menu_answer(&rows, Some(7), PageKind::Tab), (-1, true), "baa's DevTools, not Chrome's");
    assert_eq!(menu_answer(&rows, Some(7), PageKind::Popup), (300, false), "Chrome's window");
    assert_eq!(menu_answer(&rows, Some(7), PageKind::DevTools), (300, false));
    assert_eq!(menu_answer(&rows, None, PageKind::Tab), (-1, false));
    assert_eq!(menu_answer(&rows, Some(99), PageKind::Tab), (-1, false));
}

#[gpui::test]
fn a_menu_for_a_page_that_is_gone_is_closed_at_once_and_never_by_the_callback(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let label = c"Copy";
    let items = [ffi::MenuItem {
        label: label.as_ptr(),
        command: 101,
        depth: 0,
        kind: 0,
        enabled: true,
        checked: false,
    }];
    (env.callbacks.context_menu)(env.ctx(), 0xdead0 as *mut Tab, 9, 1., 2., items.as_ptr(), 1);
    assert_eq!(calls("context_menu_done"), Vec::<String>::new(), "not inside the callback");
    cx.run_until_parked();
    assert_eq!(calls("context_menu_done"), ["context_menu_done 9 -1"]);
}

#[gpui::test]
fn a_download_is_listed_in_place_and_announced_once(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);
    let send = |tab: *mut Tab, id: u32, received: i64, state: c_int| {
        let download = ffi::Download {
            id,
            url: c"https://a.test/f.zip".as_ptr(),
            path: c"/tmp/f.zip".as_ptr(),
            received_bytes: received,
            total_bytes: 10,
            state,
        };
        (env.callbacks.download)(env.ctx(), tab, &download);
    };

    send(tab, 4, 0, 0);
    send(tab, 4, 5, 0);
    send(std::ptr::null_mut(), 5, 0, 0);
    send(tab, 4, 10, 1);
    cx.run_until_parked();

    assert_eq!(*seen.borrow(), [PageEvent::DownloadStarted], "once for id 4; id 5 has no page");
    let items = env.host.downloads().read_with(cx, |list, _| list.items.clone());
    assert_eq!(items.len(), 2);
    assert_eq!((items[0].id, items[0].received_bytes, items[0].state), (4, 10, DownloadState::Complete));
    assert_eq!(items[1].state, DownloadState::InProgress);

    env.host.cancel_download(5);
    assert_eq!(calls("download_cancel"), ["download_cancel 5"]);
}

#[gpui::test]
fn devtools_dock_in_two_halves_and_report_the_picker(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let inspected = env.page(1, "https://a.test", cx);
    let inspected_tab = env.tab_of(&inspected, cx);
    let devtools = env
        .window
        .update(cx, |_, window, cx| env.host.create_devtools(&inspected, Some((3, 4)), "left", window, cx))
        .unwrap();
    assert_eq!(calls("tab_open_devtools"), [format!("tab_open_devtools {inspected_tab:?} true 3 4")]);
    let front = env.tab_of(&devtools, cx);
    let seen = events(&devtools, cx);

    let first = "devtools://devtools/bundled/devtools_app.html?remoteBase=x&dockSide=left";
    let loading = State { url: first, loading: true, ..Default::default() };
    env.state(front, &loading);
    cx.run_until_parked();
    assert_eq!(calls("tab_eval"), Vec::<String>::new(), "not before it has loaded");

    env.state(front, &State { loading: false, ..loading.clone() });
    env.state(front, &State { loading: false, ..loading.clone() });
    cx.run_until_parked();
    let evals = calls("tab_eval");
    assert_eq!(evals.len(), 1);
    assert!(evals[0].contains("currentDockState") && evals[0].contains("\"left\""), "{}", evals[0]);

    // The script reloads the front-end with can_dock; the second half follows.
    let second = "devtools://devtools/bundled/devtools_app.html?remoteBase=x&can_dock=true";
    env.state(front, &State { url: second, loading: false, ..Default::default() });
    cx.run_until_parked();
    let evals = calls("tab_eval");
    assert_eq!(evals.len(), 2);
    assert!(evals[1].contains("__appDock") && evals[1].contains("setIsDocked(false, done)"));
    assert!(evals[1].contains("baa:"));

    (env.callbacks.page_message)(env.ctx(), front, c"devtools-dock:bottom".as_ptr());
    (env.callbacks.page_message)(env.ctx(), front, c"something-else".as_ptr());
    (env.callbacks.page_message)(env.ctx(), inspected_tab, c"devtools-dock:right".as_ptr());
    cx.run_until_parked();
    let docks: Vec<_> = seen.borrow().iter().filter(|e| matches!(e, PageEvent::DevToolsDock(_))).cloned().collect();
    assert_eq!(docks, [PageEvent::DevToolsDock("bottom".into())], "only DevTools' picker is heard");

    inspected.read_with(cx, |page, _| page.open_devtools(None));
    assert_eq!(calls("tab_show_devtools_window"), [format!("tab_show_devtools_window {inspected_tab:?} false 0 0")]);
}

#[gpui::test]
fn devtools_for_a_gone_page_close_themselves(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let inspected = env.page(1, "https://a.test", cx);
    (env.callbacks.tab_closed)(env.ctx(), env.tab_of(&inspected, cx));
    let devtools = env
        .window
        .update(cx, |_, window, cx| env.host.create_devtools(&inspected, None, "right", window, cx))
        .unwrap();
    let seen = events(&devtools, cx);
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [PageEvent::Closed]);
    assert_eq!(calls("tab_open_devtools"), Vec::<String>::new());
}

#[gpui::test]
fn extensions_serve_the_toolbar_and_the_page_from_one_list(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    EXTENSIONS.with(|list| {
        list.borrow_mut().extend([
            FakeExtension { id: "bbb", name: "beta", permissions: "Read x\nChange y", site_access: 3, popup: "chrome-extension://bbb/p.html", enabled: true },
            FakeExtension { id: "aaa", name: "Alpha", permissions: "", site_access: 0, popup: "", enabled: false },
            FakeExtension { id: "ccc", name: "charlie", permissions: "Z", site_access: 2, popup: "", enabled: true },
        ]);
    });

    let toolbar = env.host.extensions(ProfileId(1));
    assert_eq!(toolbar.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["bbb", "ccc"], "enabled ones only");
    assert_eq!(toolbar[0].popup.as_deref(), Some("chrome-extension://bbb/p.html"));
    assert_eq!(toolbar[1].popup, None);

    let details = Rc::new(RefCell::new(None));
    let sink = details.clone();
    cx.update(|cx| env.host.extension_details(ProfileId(1), cx, Box::new(move |infos, _| *sink.borrow_mut() = infos)));
    let details = details.borrow().clone().unwrap();
    assert_eq!(details.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["aaa", "bbb", "ccc"], "by lowercased name");
    assert!(!details[0].enabled && details[1].enabled);
    assert_eq!(details[1].permissions, ["Read x", "Change y"]);
    assert_eq!(details[1].site_access, Some(SiteAccess::OnAllSites));
    assert_eq!(details[0].site_access, None);

    let ephemeral = env.host.ephemeral_profile();
    assert_eq!(env.host.extensions(ephemeral), []);
    let ok = Rc::new(Cell::new(true));
    let sink = ok.clone();
    cx.update(|cx| env.host.change_extension(ephemeral, "bbb", ExtensionChange::Remove, cx, Box::new(move |v, _| sink.set(v))));
    assert!(!ok.get());
    assert_eq!(calls("profile_extension_change"), Vec::<String>::new());
}

#[gpui::test]
fn extension_changes_are_forwarded_with_their_codes(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let result = Rc::new(Cell::new(false));
    for change in [ExtensionChange::Enable(false), ExtensionChange::Remove, ExtensionChange::SiteAccess(SiteAccess::OnSpecificSites)] {
        let sink = result.clone();
        cx.update(|cx| env.host.change_extension(ProfileId(1), "bbb", change, cx, Box::new(move |v, _| sink.set(v))));
        assert!(result.get());
    }
    assert_eq!(
        calls("profile_extension_change"),
        ["profile_extension_change bbb 0 0", "profile_extension_change bbb 1 0", "profile_extension_change bbb 2 2"]
    );
}

#[gpui::test]
fn a_popup_is_a_page_that_reports_its_size_and_a_missing_one_closes(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let popup = env
        .window
        .update(cx, |_, window, cx| env.host.create_popup(ProfileId(1), "chrome-extension://bbb/p.html", window, cx))
        .unwrap();
    assert_eq!(calls("extension_popup"), ["extension_popup bbb"]);
    let seen = events(&popup, cx);
    (env.callbacks.resized)(env.ctx(), env.tab_of(&popup, cx), 300, 200);
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [PageEvent::PreferredSize(gpui::size(gpui::px(300.), gpui::px(200.)))]);

    POPUP_NULL.with(|p| p.set(true));
    let none = env
        .window
        .update(cx, |_, window, cx| env.host.create_popup(ProfileId(1), "chrome-extension://bbb/p.html", window, cx))
        .unwrap();
    let seen = events(&none, cx);
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [PageEvent::Closed]);
}

#[test]
fn a_popup_url_names_its_extension() {
    assert_eq!(extensions::id_of("chrome-extension://abc/popup.html"), Some("abc"));
    assert_eq!(extensions::id_of("https://abc/popup.html"), None);
    assert_eq!(extensions::id_of("chrome-extension:///x"), None);
}

#[gpui::test]
fn each_fetch_answers_its_own_closure_once(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let got: Rc<RefCell<Vec<Fetched>>> = Rc::default();
    for n in 0..2u32 {
        let sink = got.clone();
        env.host.fetch(ProfileId(1), &format!("https://s.test/{n}"), Box::new(move |body| sink.borrow_mut().push((n, body))));
    }
    assert_eq!(calls("profile_fetch"), ["profile_fetch https://s.test/0 1", "profile_fetch https://s.test/1 2"]);

    (env.callbacks.fetch_done)(env.ctx(), 2, false, std::ptr::null(), 0);
    (env.callbacks.fetch_done)(env.ctx(), 1, true, c"[1,2]".as_ptr(), 5);
    (env.callbacks.fetch_done)(env.ctx(), 1, true, c"again".as_ptr(), 5);
    assert!(got.borrow().is_empty(), "not inside the callback");
    cx.run_until_parked();
    assert_eq!(*got.borrow(), [(1, None), (0, Some("[1,2]".to_string()))], "id 2 was fetch 1; a repeated id does nothing");
}

#[gpui::test]
fn every_opened_profile_gets_its_prefs_pinned_off(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.page(1, "https://a.test", cx);
    env.page(1, "https://b.test", cx);
    let calls = calls("profile_set_pref_bool");
    assert_eq!(calls.len(), baa_privacy::PREFS_OFF.len(), "once per profile, not per page");
    assert!(calls[0].ends_with("alternate_error_pages.enabled false"));
}

#[gpui::test]
fn the_settings_sheet_is_one_skinned_tab_per_profile(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let sheet = |url: &str, css: &str, dark| crate::ChromeSheet {
        url: url.into(),
        bounds: gpui::Bounds::new(gpui::point(gpui::px(0.), gpui::px(0.)), gpui::size(gpui::px(100.), gpui::px(100.))),
        corner_radius: 8.,
        css: css.into(),
        dark,
    };
    let show = |s: Option<crate::ChromeSheet>, cx: &mut TestAppContext| {
        env.window.update(cx, |_, window, _| env.host.show_chrome_page(ProfileId(1), s, window)).unwrap()
    };

    assert!(show(Some(sheet("baa://settings", "a{}", true)), cx));
    let calls_now = |prefix: &str| calls(prefix);
    assert_eq!(calls_now("tab_create").len(), 1);
    assert!(calls_now("tab_create")[0].ends_with("about:blank"));
    assert!(calls_now("tab_set_init_script")[0].contains("__baaSkin"), "the skin is in before the page loads");
    assert!(calls_now("tab_load_url")[0].ends_with("chrome://settings"), "baa:// is Chrome's page");
    assert!(calls_now("tab_set_color_scheme")[0].ends_with(" 2"));

    assert!(show(Some(sheet("baa://settings/privacy", "b{}", false)), cx));
    assert_eq!(calls_now("tab_create").len(), 1, "same tab");
    assert!(calls_now("tab_load_url")[1].ends_with("chrome://settings/privacy"));
    assert!(calls_now("tab_eval")[0].contains("__baaSkin") && calls_now("tab_eval")[0].contains("b{}"));
    assert!(calls_now("tab_set_color_scheme")[1].ends_with(" 1"));

    assert!(show(None, cx));
    assert_eq!(calls_now("tab_close"), Vec::<String>::new(), "hidden, not closed");
    env.host.delete_profile(ProfileId(1));
    assert_eq!(calls_now("tab_close").len(), 1);
}

#[gpui::test]
fn pages_without_a_ui_page_are_refused_not_left_waiting(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let unknown = 0xdead0 as *mut Tab;
    (env.callbacks.dialog)(env.ctx(), unknown, 11, 0, c"hi".as_ptr(), c"".as_ptr());
    (env.callbacks.dialog)(env.ctx(), unknown, 12, 3, c"leave?".as_ptr(), c"".as_ptr());
    (env.callbacks.permission)(env.ctx(), unknown, 13, c"https://x.test".as_ptr(), 1);
    assert_eq!(calls("dialog_answer").len() + calls("permission_answer").len(), 0, "queued, not inline");
    cx.run_until_parked();
    assert_eq!(calls("dialog_answer"), ["dialog_answer 11 false ", "dialog_answer 12 false "]);
    assert_eq!(calls("permission_answer"), ["permission_answer 13 false"]);
}

#[gpui::test]
fn only_tabs_are_activated(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let tab_page = env.page(1, "https://a.test", cx);
    tab_page.read_with(cx, |page, _| page.set_visible(true));
    assert_eq!(calls("tab_activate").len(), 1);

    let popup = env
        .window
        .update(cx, |_, window, cx| env.host.create_popup(ProfileId(1), "chrome-extension://bbb/p.html", window, cx))
        .unwrap();
    popup.read_with(cx, |page, _| page.set_visible(true));
    assert_eq!(calls("tab_activate").len(), 1, "a popup has no strip");
}

#[gpui::test]
fn ephemeral_profiles_are_not_opened_to_find_no_extensions(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let ephemeral = env.host.ephemeral_profile();
    assert_eq!(env.host.extensions(ephemeral), []);
    cx.update(|cx| env.host.change_extension(ephemeral, "x", ExtensionChange::Remove, cx, Box::new(|_, _| {})));
    assert_eq!(calls("profile_open_ephemeral"), Vec::<String>::new());
}

#[gpui::test]
fn a_sheet_chrome_closed_is_opened_again_and_its_menu_is_not_dropped(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let sheet = || crate::ChromeSheet {
        url: "baa://settings".into(),
        bounds: gpui::Bounds::new(gpui::point(gpui::px(0.), gpui::px(0.)), gpui::size(gpui::px(100.), gpui::px(100.))),
        corner_radius: 8.,
        css: String::new(),
        dark: false,
    };
    let show = |cx: &mut TestAppContext| {
        env.window.update(cx, |_, window, _| env.host.show_chrome_page(ProfileId(1), Some(sheet()), window)).unwrap()
    };
    assert!(show(cx));
    let first: *mut Tab = *env.dispatch.bare.borrow().keys().next().unwrap();

    // Its menu goes up over the sheet; Chrome is told when it closes, not at once.
    let items = [ffi::MenuItem { label: c"Copy".as_ptr(), command: 7, depth: 0, kind: 0, enabled: true, checked: false }];
    (env.callbacks.context_menu)(env.ctx(), first, 4, 1., 2., items.as_ptr(), 1);
    cx.run_until_parked();
    assert_eq!(calls("context_menu_done"), Vec::<String>::new());

    (env.callbacks.tab_closed)(env.ctx(), first);
    assert!(show(cx));
    assert_eq!(calls("tab_create").len(), 2, "a fresh tab replaced the dead one");
    assert_eq!(calls("tab_close"), Vec::<String>::new(), "the dead one is not closed again");
}

#[test]
fn a_floating_page_rises_to_just_below_gpui_with_an_overlay_above() {
    let at = |content: *mut AnyObject, i: usize| -> *mut AnyObject {
        unsafe {
            let subviews: *mut AnyObject = msg_send![content, subviews];
            msg_send![subviews, objectAtIndex: i]
        }
    };
    let (content, gpui_view) = (new_view(), new_view());
    unsafe {
        let _: () = msg_send![content, addSubview: gpui_view];
    }
    let floating = native::Container::new(gpui_view, std::ptr::null_mut(), 0.);
    let other = native::Container::new(gpui_view, std::ptr::null_mut(), 0.);
    let overlay = native::Container::new(std::ptr::null_mut(), std::ptr::null_mut(), 0.);
    overlay.move_above(gpui_view);
    floating.set_floating(true);
    assert_eq!(at(content, 0), floating.as_ptr());

    let bounds = gpui::Bounds::new(gpui::point(gpui::px(0.), gpui::px(0.)), gpui::size(gpui::px(10.), gpui::px(10.)));
    floating.place(std::ptr::null_mut(), bounds, bounds);
    assert_eq!([at(content, 0), at(content, 1), at(content, 2), at(content, 3)], [other.as_ptr(), floating.as_ptr(), gpui_view, overlay.as_ptr()]);
    assert!(floating.is_beside(gpui_view));
}
