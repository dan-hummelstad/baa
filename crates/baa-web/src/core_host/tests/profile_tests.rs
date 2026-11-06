use super::*;

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
    let first: *mut Tab = *env.dispatch.tabs.borrow().keys().next().unwrap();

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
