use super::*;

#[gpui::test]
fn extensions_serve_the_toolbar_and_the_page_from_one_list(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    EXTENSIONS.with(|list| {
        list.borrow_mut().extend([
            FakeExtension { id: "bbb", name: "beta", permissions: "Read x\nChange y", site_access: 3, popup: "chrome-extension://bbb/p.html", enabled: true, unpacked: false, needs_approval: false },
            FakeExtension { id: "aaa", name: "Alpha", permissions: "", site_access: 0, popup: "", enabled: false, unpacked: false, needs_approval: true },
            FakeExtension { id: "ccc", name: "charlie", permissions: "Z", site_access: 2, popup: "", enabled: true, unpacked: true, needs_approval: false },
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
    assert_eq!(details.iter().map(|e| e.unpacked).collect::<Vec<_>>(), [false, false, true]);
    assert_eq!(details.iter().map(|e| e.needs_approval).collect::<Vec<_>>(), [true, false, false]);

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
    for change in [ExtensionChange::Enable(false), ExtensionChange::Remove, ExtensionChange::SiteAccess(SiteAccess::OnSpecificSites), ExtensionChange::Reload] {
        let sink = result.clone();
        cx.update(|cx| env.host.change_extension(ProfileId(1), "bbb", change, cx, Box::new(move |v, _| sink.set(v))));
        assert!(result.get());
    }
    assert_eq!(
        calls("profile_extension_change"),
        ["profile_extension_change bbb 0 0", "profile_extension_change bbb 1 0", "profile_extension_change bbb 2 2", "profile_extension_change bbb 3 0"]
    );
}

fn changes(env: &Env, cx: &mut TestAppContext) -> Rc<RefCell<Vec<ProfileId>>> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let source = env.host.extensions_changed();
    cx.update(|cx| cx.subscribe(&source, move |_, profile: &ProfileId, _| sink.borrow_mut().push(*profile)).detach());
    seen
}

#[gpui::test]
fn a_change_of_a_spaces_extensions_reaches_the_ui_with_its_profile(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.page(1, "https://a.test", cx);
    env.page(2, "https://b.test", cx);
    let seen = changes(&env, cx);
    let (one, two) = (env.host.profile_handle(ProfileId(1)), env.host.profile_handle(ProfileId(2)));
    for handle in [two, one] {
        (env.callbacks.extensions_changed)(env.ctx(), handle);
    }
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [ProfileId(2), ProfileId(1)]);
}

#[gpui::test]
fn incognito_and_unknown_profiles_report_no_extension_change(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let private = env.host.ephemeral_profile();
    env.page(private.0, "https://a.test", cx);
    let seen = changes(&env, cx);
    (env.callbacks.extensions_changed)(env.ctx(), env.host.profile_handle(private));
    (env.callbacks.extensions_changed)(env.ctx(), fresh());
    cx.run_until_parked();
    assert!(seen.borrow().is_empty());
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

fn action(id: &'static CStr, badge_rgba: u32, text_rgba: u32) -> ffi::Action {
    ffi::Action { extension_id: id.as_ptr(), badge_text: c"3".as_ptr(), title: c"Tip".as_ptr(), badge_rgba, text_rgba, enabled: true, has_popup: false }
}

#[gpui::test]
fn actions_are_read_per_page_or_as_defaults_with_alpha_0_as_no_colour(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    ACTIONS.with(|list| list.borrow_mut().extend([action(c"aaa", 0xff0000ff, 0x11223300), action(c"bbb", 0x12345600, 0xffffff80)]));
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);

    let defaults = env.host.extension_actions(ProfileId(1), None);
    let on_page = cx.read(|cx| env.host.extension_actions(ProfileId(1), Some(page.read(cx))));
    assert_eq!(calls("profile_extension_actions"), ["profile_extension_actions 0".to_string(), format!("profile_extension_actions {tab}")]);
    assert_eq!(defaults, on_page);
    assert_eq!(
        defaults[0],
        crate::ExtensionAction {
            extension_id: "aaa".into(),
            badge_text: "3".into(),
            title: "Tip".into(),
            badge_rgba: Some(0xff0000ff),
            text_rgba: None,
            enabled: true,
        }
    );
    assert_eq!((defaults[1].badge_rgba, defaults[1].text_rgba), (None, Some(0xffffff80)));

    (env.callbacks.tab_closed)(env.ctx(), tab, ffi::CLOSED_BY_CHROME);
    let closed = cx.read(|cx| env.host.extension_actions(ProfileId(1), Some(page.read(cx))));
    assert_eq!(closed, []);
    assert_eq!(calls("profile_extension_actions").len(), 2, "a closed page does not read the defaults");
}

fn action_changes(env: &Env, cx: &mut TestAppContext) -> Rc<RefCell<Vec<crate::ExtensionActionChanged>>> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let source = env.host.extension_actions_changed();
    cx.update(|cx| cx.subscribe(&source, move |_, changed: &crate::ExtensionActionChanged, _| sink.borrow_mut().push(changed.clone())).detach());
    seen
}

#[gpui::test]
fn an_action_change_inside_an_engine_call_reaches_the_ui_later_without_reentering(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = action_changes(&env, cx);
    ACTION_INSIDE.with(|a| a.set(env.ctx()));
    env.host.extension_actions(ProfileId(1), None);
    let engine: Vec<String> = CALLS.with(|c| c.borrow().iter().skip_while(|c| !c.starts_with("profile_extension_actions")).cloned().collect());
    assert_eq!(engine, ["profile_extension_actions 0", "profile_extension_actions returns"]);
    assert!(seen.borrow().is_empty());

    // The caller may hold the cells it reads.
    {
        let _held = (env.dispatch.tabs.borrow_mut(), env.dispatch.profile_ids.borrow_mut());
        (env.callbacks.extension_action)(env.ctx(), env.host.profile_handle(ProfileId(1)), c"bbb".as_ptr(), tab);
        (env.callbacks.extension_action)(env.ctx(), env.host.profile_handle(ProfileId(1)), c"ccc".as_ptr(), 9999);
        (env.callbacks.extension_action)(env.ctx(), fresh(), c"ddd".as_ptr(), 0);
    }
    cx.run_until_parked();
    let changed = |id: &str, page| crate::ExtensionActionChanged { profile: ProfileId(1), extension_id: id.into(), page };
    assert_eq!(*seen.borrow(), [changed("aaa", None), changed("bbb", Some(page.entity_id()))]);
}

#[gpui::test]
fn commands_are_listed_and_rebound_with_mac_modifiers(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    COMMANDS.with(|list| {
        list.borrow_mut().push(ffi::Command {
            extension_id: c"aaa".as_ptr(),
            name: c"toggle".as_ptr(),
            description: c"Toggle it".as_ptr(),
            key: c"Comma".as_ptr(),
            modifiers: ffi::MOD_CMD | ffi::MOD_SHIFT,
            action: false,
            global: true,
        })
    });
    let commands = env.host.extension_commands(ProfileId(1));
    let cmd_shift = gpui::Modifiers { platform: true, shift: true, ..Default::default() };
    assert_eq!(
        commands,
        [crate::ExtensionCommand {
            extension_id: "aaa".into(),
            name: "toggle".into(),
            description: "Toggle it".into(),
            key: "Comma".into(),
            modifiers: cmd_shift,
            action: false,
        }]
    );

    let ctrl_alt_fn = gpui::Modifiers { control: true, alt: true, function: true, ..Default::default() };
    assert!(env.host.set_extension_shortcut(ProfileId(1), "aaa", "toggle", "K", ctrl_alt_fn));
    assert!(env.host.set_extension_shortcut(ProfileId(1), "aaa", "toggle", "", cmd_shift));
    assert_eq!(calls("extension_set_shortcut"), ["extension_set_shortcut aaa toggle \"K\" 6", "extension_set_shortcut aaa toggle \"\" 0"]);
}

fn popups_opened(env: &Env, cx: &mut TestAppContext) -> Rc<RefCell<Vec<crate::ExtensionPopupOpened>>> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let source = env.host.extension_popups();
    cx.update(|cx| cx.subscribe(&source, move |_, opened: &crate::ExtensionPopupOpened, _| sink.borrow_mut().push(opened.clone())).detach());
    seen
}

#[gpui::test]
fn a_popup_the_engine_opened_is_taken_by_create_popup(cx: &mut TestAppContext) {
    let env = Rc::new(Env::new(cx));
    let handle = env.host.profile_handle(ProfileId(1));
    let popup_tab = fresh_tab();
    let shown = Rc::new(RefCell::new(None));
    let (sink, ui) = (shown.clone(), env.clone());
    let source = env.host.extension_popups();
    let subscription = cx.update(|cx| {
        cx.subscribe(&source, move |_, opened: &crate::ExtensionPopupOpened, cx| {
            let url = format!("chrome-extension://{}/p.html", opened.extension_id);
            let page = ui.window.update(cx, |_, window, cx| ui.host.create_popup(opened.profile, &url, window, cx)).unwrap();
            *sink.borrow_mut() = Some(page);
        })
    });
    (env.callbacks.resized)(env.ctx(), popup_tab, 320, 240);
    (env.callbacks.extension_popup_opened)(env.ctx(), handle, c"bbb".as_ptr(), popup_tab);
    cx.run_until_parked();
    let popup = shown.borrow().clone().expect("the UI heard of it");
    assert_eq!(env.tab_of(&popup, cx), popup_tab);
    assert_eq!(calls("extension_popup"), Vec::<String>::new(), "adopted, not opened again");
    assert_eq!(calls("tab_close"), Vec::<String>::new());

    let seen = events(&popup, cx);
    (env.callbacks.resized)(env.ctx(), popup_tab, 300, 200);
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [PageEvent::PreferredSize(gpui::size(gpui::px(300.), gpui::px(200.)))]);
    drop(subscription);
}

#[gpui::test]
fn a_size_reported_before_the_popup_page_exists_reaches_it(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let handle = env.host.profile_handle(ProfileId(1));
    let popup_tab = fresh_tab();
    (env.callbacks.resized)(env.ctx(), popup_tab, 320, 240);
    (env.callbacks.extension_popup_opened)(env.ctx(), handle, c"bbb".as_ptr(), popup_tab);
    let popup = env
        .window
        .update(cx, |_, window, cx| env.host.create_popup(ProfileId(1), "chrome-extension://bbb/p.html", window, cx))
        .unwrap();
    let seen = events(&popup, cx);
    cx.run_until_parked();
    assert_eq!(env.tab_of(&popup, cx), popup_tab);
    assert!(seen.borrow().contains(&PageEvent::PreferredSize(gpui::size(gpui::px(320.), gpui::px(240.)))));
}

#[gpui::test]
fn a_popup_the_ui_does_not_take_is_closed_and_one_already_gone_is_not_announced(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let handle = env.host.profile_handle(ProfileId(1));
    let seen = popups_opened(&env, cx);
    let (kept, gone) = (fresh_tab(), fresh_tab());
    (env.callbacks.extension_popup_opened)(env.ctx(), handle, c"bbb".as_ptr(), kept);
    (env.callbacks.extension_popup_opened)(env.ctx(), handle, c"ccc".as_ptr(), gone);
    (env.callbacks.tab_closed)(env.ctx(), gone, ffi::CLOSED_BY_CHROME);
    (env.callbacks.extension_popup_opened)(env.ctx(), fresh(), c"ddd".as_ptr(), 9999);
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [crate::ExtensionPopupOpened { profile: ProfileId(1), extension_id: "bbb".into() }]);
    assert_eq!(calls("tab_close"), [format!("tab_close {kept}"), "tab_close 9999".to_string()]);
}
