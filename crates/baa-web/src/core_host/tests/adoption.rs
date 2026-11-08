use super::*;

#[gpui::test]
fn chrome_made_tabs_go_to_their_opener_or_the_tab_list(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let opener = env.page(1, "https://a.test", cx);
    let opener_tab = env.tab_of(&opener, cx);
    let seen = events(&opener, cx);

    env.created(0x9000, opener_tab, ProfileId(1), "https://popup.test");
    cx.run_until_parked();
    assert_eq!(
        *seen.borrow(),
        [PageEvent::OpenRequested { url: "https://popup.test".into(), from_user_gesture: true, background: false }]
    );

    env.created(0x9010, 0, ProfileId(1), "https://ext.test");
    cx.run_until_parked();
    assert_eq!(env.host.chrome_tabs().read_with(cx, |tabs, _| tabs.0.clone()), ["https://ext.test"]);
    assert_eq!(seen.borrow().len(), 1, "no opener, nobody to ask");
}

#[gpui::test]
fn a_page_for_the_same_profile_and_url_adopts_the_tab(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.created(0x9000, 0, ProfileId(1), "https://popup.test");
    let creates = calls("tab_create").len();

    let adopted = env.opened(1, "https://popup.test", cx);
    assert_eq!(env.tab_of(&adopted, cx), 0x9000);
    assert_eq!(calls("tab_create").len(), creates, "no second tab");

    let other = env.opened(1, "https://other.test", cx);
    assert_ne!(env.tab_of(&other, cx), 0x9000);
    assert_eq!(calls("tab_create").len(), creates + 1);

    // Adopted tabs are not closed when their timer runs out.
    cx.executor().advance_clock(ADOPTION_EXPIRY + Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(calls("tab_close"), Vec::<String>::new());
}

#[gpui::test]
fn an_unadopted_tab_is_closed_after_the_expiry(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.created(0x9000, 0, ProfileId(1), "https://nobody.test");
    cx.executor().advance_clock(ADOPTION_EXPIRY - Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(calls("tab_close"), Vec::<String>::new());

    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(calls("tab_close"), [format!("tab_close {:?}", 0x9000)]);
    // Too late to adopt it now.
    let late = env.opened(1, "https://nobody.test", cx);
    assert_ne!(env.tab_of(&late, cx), 0x9000);
}

#[gpui::test]
fn a_tab_in_a_profile_baa_never_opened_is_closed_and_its_url_kept(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let url = c"https://x.test";
    (env.callbacks.tab_created)(env.ctx(), 0x9000, 0, std::ptr::null_mut(), url.as_ptr(), false, false);
    cx.run_until_parked();
    assert_eq!(calls("tab_close"), [format!("tab_close {:?}", 0x9000)]);
    assert_eq!(env.host.chrome_tabs().read_with(cx, |tabs, _| tabs.0.clone()), ["https://x.test"]);
}

#[gpui::test]
fn a_foreground_open_nobody_adopts_is_closed_this_turn_but_background_waits(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let opener = env.page(1, "https://a.test", cx);
    let opener_tab = env.tab_of(&opener, cx);

    env.created_in(0x9000, opener_tab, ProfileId(1), "https://fg.test", false);
    env.created_in(0x9010, opener_tab, ProfileId(1), "https://bg.test", true);
    cx.run_until_parked();
    assert_eq!(calls("tab_close"), [format!("tab_close {:?}", 0x9000)]);

    cx.executor().advance_clock(ADOPTION_EXPIRY + Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(calls("tab_close").len(), 2);
}

#[gpui::test]
fn devtools_never_adopt_a_pending_tab(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let of = env.page(1, "https://a.test", cx);
    env.created(0x9000, 0, ProfileId(1), "about:blank");
    let creates = calls("tab_create").len();
    let devtools = env
        .window
        .update(cx, |_, window, cx| env.host.create_devtools(&of, None, "right", window, cx))
        .unwrap();
    assert_eq!(calls("tab_create").len(), creates, "Chrome makes the front-end itself");
    assert_eq!(calls("tab_open_devtools").len(), 1);
    assert_ne!(env.tab_of(&devtools, cx), 0x9000);
}

#[gpui::test]
fn a_page_supplied_url_that_misses_adoption_is_never_loaded(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let opener = env.page(1, "https://a.test", cx);
    let opener_tab = env.tab_of(&opener, cx);
    env.created_in(0x9000, opener_tab, ProfileId(1), "data:text/html,x", true);
    env.created_in(0x9010, opener_tab, ProfileId(1), "chrome://flags", true);
    cx.executor().advance_clock(ADOPTION_EXPIRY + Duration::from_secs(1));
    cx.run_until_parked();
    let creates = calls("tab_create").len();

    for url in ["data:text/html,x", "baa://flags", "file:///etc/passwd", "chrome-extension://abc/p.html", "javascript:alert(1)"] {
        let page = env.opened(1, url, cx);
        assert!(![0, 0x9000, 0x9010].contains(&env.tab_of(&page, cx)), "{url}");
    }
    let created = calls("tab_create");
    assert_eq!(created.len(), creates + 5);
    assert!(created[creates..].iter().all(|c| c.ends_with(" about:blank")), "{created:?}");
    assert_eq!(calls("tab_load_url"), Vec::<String>::new(), "no to_chrome, no typed load");

    env.opened(1, "http://b.test/", cx);
    assert!(calls("tab_create").last().unwrap().ends_with(" http://b.test/"));
}

#[gpui::test]
fn the_user_never_gets_a_tab_a_page_opened(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.created(0x9000, 0, ProfileId(1), "chrome://flags");
    let page = env.page(1, "baa://flags", cx);
    assert_ne!(env.tab_of(&page, cx), 0x9000);
    assert!(calls("tab_load_url").last().unwrap().ends_with(" chrome://flags"));
}
