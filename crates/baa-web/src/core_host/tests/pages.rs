use super::*;

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
fn dropping_a_page_closes_its_tab_once_and_unknown_tabs_are_ignored(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    (env.callbacks.tab_closed)(env.ctx(), 0xdead0, ffi::CLOSED_BY_CHROME);
    cx.run_until_parked();
    assert!(seen.borrow().is_empty());

    cx.update(|_| drop(page));
    assert_eq!(calls("tab_close"), [format!("tab_close {tab:?}")]);
    (env.callbacks.tab_closed)(env.ctx(), tab, ffi::CLOSED_BY_APP);
    cx.run_until_parked();
    assert_eq!(calls("tab_close").len(), 1);
}

#[gpui::test]
fn a_tab_chrome_closed_says_so_and_is_not_closed_again(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    (env.callbacks.tab_closed)(env.ctx(), tab, ffi::CLOSED_BY_CHROME);
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [PageEvent::Closed]);
    (env.callbacks.tab_closed)(env.ctx(), tab, ffi::CLOSED_BY_CHROME);
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [PageEvent::Closed], "an id used after tab_closed is ignored");
    page.read_with(cx, |page, _| page.reload(false));
    assert_eq!(calls("tab_reload"), Vec::<String>::new(), "the handle is dead");
    cx.update(|_| drop(page));
    assert_eq!(calls("tab_close"), Vec::<String>::new());
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
    let args: Vec<String> = ["baa", "https://x.test", "--proxy-server=p", "--disable-features=Foo", "--enable-features=Bar"].map(Into::into).into();
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
    let enabled: Vec<_> = argv.iter().filter(|a| a.starts_with("--enable-features=")).collect();
    assert_eq!(enabled, ["--enable-features=Bar,BrowserInitiatedAutomaticPictureInPicture"]);

    let own = chrome_argv(&["baa".into(), "--user-data-dir=/mine".into()], Path::new("/data/chromium"));
    assert_eq!(own.iter().filter(|a| a.starts_with("--user-data-dir=")).collect::<Vec<_>>(), ["--user-data-dir=/mine"]);
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
    page.read_with(cx, |page, _| assert_eq!(page.shared.container.page(), new));
}

/// gpui re-sends a held drag every 16 ms from a timer, with no NSEvent behind it (as every
/// simulated event here). The drag's target must outlive those, or the page never hears the
/// release: a dragged map stays held, and a click that moved a pixel never clicks.
#[gpui::test]
fn a_held_drag_keeps_its_target_until_the_release(cx: &mut TestAppContext) {
    struct Shows(Entity<CorePage>);
    impl Render for Shows {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            use gpui::{ParentElement, Styled};
            div().size_full().child(self.0.clone())
        }
    }
    let env = Env::new(cx);
    let (view, widget) = (new_view(), new_view());
    unsafe {
        let _: () = msg_send![view, addSubview: widget];
    }
    let page = env.page_with_view(view, cx);
    let shared = page.read_with(cx, |page, _| page.shared.clone());
    let (_, vcx) = cx.add_window_view(|_, _| Shows(page.clone()));
    vcx.run_until_parked();
    let at = gpui::point(gpui::px(20.), gpui::px(20.));
    shared.drag.set(unsafe { objc2::rc::Retained::retain(widget) });

    vcx.simulate_mouse_move(at, Some(gpui::MouseButton::Left), gpui::Modifiers::none());
    let held = shared.drag.take();
    assert!(held.is_some(), "a drag tick with nothing to forward keeps the target");
    shared.drag.set(held);
    vcx.simulate_mouse_up(at, gpui::MouseButton::Left, gpui::Modifiers::none());
    assert!(shared.drag.take().is_none(), "the release ends the drag");

    // No button down: the release was missed, and the next move ends it.
    shared.drag.set(unsafe { objc2::rc::Retained::retain(widget) });
    vcx.simulate_mouse_move(at, None, gpui::Modifiers::none());
    assert!(shared.drag.take().is_none());
}

#[gpui::test]
fn a_drag_target_is_dropped_with_its_view(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let (old, widget, new) = (new_view(), new_view(), new_view());
    unsafe {
        let _: () = msg_send![old, addSubview: widget];
    }
    let page = env.page_with_view(old, cx);
    let shared = page.read_with(cx, |page, _| page.shared.clone());
    assert!(native::is_inside(shared.container.page(), unsafe { &*widget }));
    shared.drag.set(unsafe { objc2::rc::Retained::retain(widget) });

    env.state(env.tab_of(&page, cx), &State { view: new as usize, ..Default::default() });
    assert!(shared.drag.take().is_none(), "the swap forgets it");
    assert!(!native::is_inside(shared.container.page(), unsafe { &*widget }));
    // A widget Chromium replaces inside the same page view (no swap) fails the check too.
    unsafe {
        let _: () = msg_send![widget, removeFromSuperview];
    }
    assert!(!native::is_inside(old, unsafe { &*widget }));
}

#[gpui::test]
fn a_tab_chrome_closed_lets_go_of_its_view_at_once(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let view = new_view();
    let page = env.page_with_view(view, cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    (env.callbacks.tab_closed)(env.ctx(), tab, ffi::CLOSED_BY_CHROME);
    assert!(superview(view).is_null());
    page.read_with(cx, |page, _| assert!(page.shared.container.page().is_null()));
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
    (env.callbacks.tab_closed)(env.ctx(), env.tab_of(&page, cx), ffi::CLOSED_BY_QUIT);
    cx.run_until_parked();
    assert!(superview(view).is_null());
    assert!(seen.borrow().is_empty(), "no Closed: the tab is still in the session");
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
fn only_tabs_are_activated(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let tab_page = env.page(1, "https://a.test", cx);
    tab_page.read_with(cx, |page, _| page.set_visible(true));
    assert!(calls("tab_activate").is_empty(), "showing is not activating");
    tab_page.read_with(cx, |page, _| page.activate());
    assert_eq!(calls("tab_activate").len(), 1);

    let popup = env
        .window
        .update(cx, |_, window, cx| env.host.create_popup(ProfileId(1), "chrome-extension://bbb/p.html", window, cx))
        .unwrap();
    popup.read_with(cx, |page, _| page.activate());
    assert_eq!(calls("tab_activate").len(), 1, "a popup has no strip");
}

#[gpui::test]
fn a_tab_the_app_closed_is_released_without_a_closed_event(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    (env.callbacks.tab_closed)(env.ctx(), tab, ffi::CLOSED_BY_APP);
    cx.run_until_parked();
    assert!(seen.borrow().is_empty());
    assert_eq!(env.tab_of(&page, cx), 0);
}

#[test]
fn a_swipe_copies_its_picture_and_keeps_amount_in_range() {
    use crate::{SwipePhase, SwipePicture};
    let pixels = [1u8, 2, 3, 4, 5, 6, 7, 8];
    let began = super::super::dispatch::swipe_event(ffi::SWIPE_BEGAN, true, 1.7, pixels.as_ptr(), 2, 1);
    let picture = SwipePicture { width: 2, height: 1, bgra: std::sync::Arc::new(pixels.to_vec()) };
    assert_eq!(began, PageEvent::Swipe { phase: SwipePhase::Began(Some(picture)), forward: true, amount: 1.0 });
    let none = super::super::dispatch::swipe_event(ffi::SWIPE_BEGAN, false, 0.0, std::ptr::null(), 0, 0);
    assert_eq!(none, PageEvent::Swipe { phase: SwipePhase::Began(None), forward: false, amount: 0.0 });
    let done = super::super::dispatch::swipe_event(ffi::SWIPE_DONE, false, 1.0, std::ptr::null(), 0, 0);
    assert_eq!(done, PageEvent::Swipe { phase: SwipePhase::Done, forward: false, amount: 1.0 });
    let cancelled = super::super::dispatch::swipe_event(ffi::SWIPE_CANCELLED, false, -0.1, std::ptr::null(), 0, 0);
    assert_eq!(cancelled, PageEvent::Swipe { phase: SwipePhase::Cancelled, forward: false, amount: 0.0 });
}

#[gpui::test]
/// The engine frees the picture when the call returns; the event is handled later.
fn a_swipe_picture_is_copied_before_the_engine_frees_it(cx: &mut TestAppContext) {
    use crate::{SwipePhase, SwipePicture};
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);
    let mut pixels = vec![7u8; 8];
    (env.callbacks.swipe)(env.ctx(), tab, ffi::SWIPE_BEGAN, false, 0.0, pixels.as_ptr(), 2, 1);
    pixels.fill(0);
    drop(pixels);
    cx.run_until_parked();
    let picture = SwipePicture { width: 2, height: 1, bgra: Arc::new(vec![7; 8]) };
    assert_eq!(*seen.borrow(), [PageEvent::Swipe { phase: SwipePhase::Began(Some(picture)), forward: false, amount: 0.0 }]);
}
