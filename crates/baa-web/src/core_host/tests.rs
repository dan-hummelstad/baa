//! The dispatch and page logic against a fake `Core`: Rust stubs record what
//! BaaCore would have been asked, tabs are fake pointers.

use std::{cell::RefCell, ffi::CString};

use gpui::{div, IntoElement, Render, TestAppContext, WindowHandle};

use super::*;

thread_local! {
    static CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static NEXT: Cell<usize> = const { Cell::new(0x1000) };
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
extern "C" fn tab_get_state(_: *mut Tab, state: *mut TabState) -> bool {
    unsafe {
        (*state).url = c"".as_ptr();
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
        profile_open,
        profile_open_ephemeral,
        profile_close,
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
        dialog_answer,
    }))
}

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
        let url = CString::new(url).unwrap();
        let handle = self.host.profile_handle(profile);
        (self.callbacks.tab_created)(self.ctx(), tab, opener, handle, url.as_ptr(), false, true);
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
}

impl State {
    /// The C struct and the strings it borrows.
    fn raw(&self) -> (TabState, [CString; 3]) {
        let strings = [self.url, self.title, self.favicon].map(|s| CString::new(s).unwrap());
        let state = TabState {
            url: strings[0].as_ptr(),
            title: strings[1].as_ptr(),
            favicon_url: strings[2].as_ptr(),
            view: std::ptr::null_mut(),
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
    for flag in ["--no-startup-window", "--no-first-run", "--user-data-dir=/data/chromium", "--disable-sync"] {
        assert_eq!(argv.iter().filter(|a| *a == flag).count(), 1, "{flag}");
    }
    assert!(!argv.iter().any(|a| a.contains("x.test")));
    let disabled: Vec<_> = argv.iter().filter(|a| a.starts_with("--disable-features=")).collect();
    assert_eq!(disabled.len(), 1);
    assert!(disabled[0].contains("Foo") && disabled[0].contains("MediaRouter"));

    let own = chrome_argv(&["baa".into(), "--user-data-dir=/mine".into()], Path::new("/data/chromium"));
    assert_eq!(own.iter().filter(|a| a.starts_with("--user-data-dir=")).collect::<Vec<_>>(), ["--user-data-dir=/mine"]);
}
