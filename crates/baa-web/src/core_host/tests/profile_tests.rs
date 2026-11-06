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
fn deleting_a_profile_leaves_every_tab_to_chromes_report(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let view = new_view();
    let page = env.page_with_view(view, cx);
    let other = env.page(1, "https://b.test", cx);
    let (tab, other_tab) = (env.tab_of(&page, cx), env.tab_of(&other, cx));
    let seen = events(&page, cx);
    env.created(0x9000, 0, ProfileId(1), "https://p.test");

    env.host.delete_profile(ProfileId(1));
    assert_eq!(calls("profile_close").len(), 1);
    // profile_close reports each tab, inside the call.
    for tab in [tab, other_tab, 0x9000] {
        (env.callbacks.tab_closed)(env.ctx(), tab, ffi::CLOSED_BY_APP);
    }
    assert!(superview(view).is_null());
    assert!(env.dispatch.tabs.borrow().is_empty());
    cx.run_until_parked();
    assert!(seen.borrow().is_empty());
    cx.update(|_| drop((page, other)));
    cx.executor().advance_clock(ADOPTION_EXPIRY + Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(calls("tab_close"), Vec::<String>::new(), "Chrome owns those tabs' end now");
}

#[gpui::test]
fn every_opened_profile_gets_its_prefs_pinned_off(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.page(1, "https://a.test", cx);
    env.page(1, "https://b.test", cx);
    let calls = calls("pref_set_bool");
    assert_eq!(calls.len(), baa_privacy::PREFS_OFF.len(), "once per profile, not per page");
    assert!(calls[0].ends_with("alternate_error_pages.enabled false"));
}

#[gpui::test]
fn ephemeral_profiles_are_not_opened_to_find_no_extensions(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let ephemeral = env.host.ephemeral_profile();
    assert_eq!(env.host.extensions(ephemeral), []);
    cx.update(|cx| env.host.change_extension(ephemeral, "x", ExtensionChange::Remove, cx, Box::new(|_, _| {})));
    assert_eq!(calls("profile_open_ephemeral"), Vec::<String>::new());
}
