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
