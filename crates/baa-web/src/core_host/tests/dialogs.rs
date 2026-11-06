use super::*;

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
