use super::*;

/// A `dialog` callback for a JS dialog (Chrome's own fill the other fields).
fn send_dialog(env: &Env, tab: TabId, id: u64, kind: c_int, message: &CStr) {
    let dialog = ffi::Dialog {
        kind,
        message: message.as_ptr(),
        default_text: c"".as_ptr(),
        title: c"".as_ptr(),
        ok_label: c"".as_ptr(),
        cancel_label: c"".as_ptr(),
        check_label: c"".as_ptr(),
    };
    (env.callbacks.dialog)(env.ctx(), tab, id, &dialog);
}

#[gpui::test]
fn dialogs_carry_their_id_to_answer_and_close(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    send_dialog(&env, tab, 7, ffi::DIALOG_CONFIRM, c"Sure?");
    cx.run_until_parked();
    assert_eq!(
        *seen.borrow(),
        [PageEvent::Dialog(Dialog {
            id: 7,
            kind: DialogKind::Confirm,
            message: "Sure?".into(),
            default_text: String::new(),
            title: String::new(),
            ok_label: String::new(),
            cancel_label: String::new(),
            check_label: String::new(),
        })]
    );
    page.read_with(cx, |page, _| page.answer_dialog(7, DialogAnswer { ok: true, text: "yes".into(), ..Default::default() }));
    assert_eq!(calls("answer"), ["answer 7 1 yes  0"]);

    send_dialog(&env, tab, 8, ffi::DIALOG_ALERT, c"Hi");
    (env.callbacks.request_closed)(env.ctx(), tab, 8);
    cx.run_until_parked();
    assert_eq!(seen.borrow().last(), Some(&PageEvent::AskClosed(8)));
}

#[gpui::test]
fn permissions_arrive_in_bit_order_and_are_answered_through_chrome(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    (env.callbacks.permission)(env.ctx(), tab, 5, c"https://a.test".as_ptr(), 0b100_0011);
    (env.callbacks.request_closed)(env.ctx(), tab, 5);
    cx.run_until_parked();
    assert_eq!(
        *seen.borrow(),
        [
            PageEvent::PermissionRequested {
                id: 5,
                origin: "https://a.test".into(),
                kinds: vec![Permission::Camera, Permission::Microphone, Permission::Other],
            },
            PageEvent::AskClosed(5),
        ]
    );
    page.read_with(cx, |page, _| page.answer_permission(6, true));
    assert_eq!(calls("answer"), ["answer 6 1   0"]);
}

#[gpui::test]
fn beforeunload_kinds_and_the_quit_fallback(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    send_dialog(&env, tab, 1, ffi::DIALOG_BEFOREUNLOAD, c"Leave?");
    send_dialog(&env, tab, 2, ffi::DIALOG_BEFOREUNLOAD_RELOAD, c"Reload?");
    cx.run_until_parked();
    let kinds: Vec<_> = seen
        .borrow()
        .iter()
        .map(|e| match e {
            PageEvent::Dialog(d) => d.kind,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(kinds, [DialogKind::BeforeUnload { is_reload: false }, DialogKind::BeforeUnload { is_reload: true }]);

    QUITTING.with(|q| q.set(true));
    seen.borrow_mut().clear();
    send_dialog(&env, tab, 3, ffi::DIALOG_BEFOREUNLOAD, c"Leave?");
    assert_eq!(calls("answer"), Vec::<String>::new(), "not inside Chrome's dialog code");
    cx.run_until_parked();
    assert!(seen.borrow().is_empty(), "nobody is asked while quitting");
    assert_eq!(calls("answer"), ["answer 3 1   0"]);
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
    let unknown = 0xdead0;
    send_dialog(&env, unknown, 11, ffi::DIALOG_ALERT, c"hi");
    send_dialog(&env, unknown, 12, ffi::DIALOG_BEFOREUNLOAD, c"leave?");
    (env.callbacks.permission)(env.ctx(), unknown, 13, c"https://x.test".as_ptr(), 1);
    assert_eq!(calls("answer").len(), 0, "queued, not inline");
    cx.run_until_parked();
    assert_eq!(calls("answer"), ["answer 11 0   0", "answer 12 0   0", "answer 13 0   0"]);
}

#[gpui::test]
fn site_settings_kinds_reach_the_core_as_bits(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.host.set_site_permission(ProfileId(1), "https://a.test", Permission::Cookies, PermissionState::SessionOnly);
    assert_eq!(calls("profile_set_site_permission"), ["profile_set_site_permission https://a.test 1024 3"]);
    SITE_STATE.with(|s| s.set(3));
    assert_eq!(env.host.site_permission(ProfileId(1), "https://a.test", Permission::Cookies), PermissionState::SessionOnly);
}

#[gpui::test]
fn chromes_own_dialogs_carry_their_wording_and_answers(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    let dialog = ffi::Dialog {
        kind: ffi::DIALOG_AUTH,
        message: c"a.test\nSign in".as_ptr(),
        default_text: c"".as_ptr(),
        title: c"Sign in".as_ptr(),
        ok_label: c"Sign in".as_ptr(),
        cancel_label: c"Cancel".as_ptr(),
        check_label: c"".as_ptr(),
    };
    (env.callbacks.dialog)(env.ctx(), tab, 7, &dialog);
    cx.run_until_parked();
    assert!(matches!(&seen.borrow()[0], PageEvent::Dialog(d) if d.kind == DialogKind::Auth && d.title == "Sign in" && d.id == 7));

    page.read_with(cx, |page, _| {
        page.answer_dialog(7, DialogAnswer { ok: true, text: "u".into(), password: "p".into(), checked: false })
    });
    assert_eq!(calls("answer"), ["answer 7 1 u p 0"]);
}
