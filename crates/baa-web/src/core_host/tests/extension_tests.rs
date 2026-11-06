use super::*;

#[gpui::test]
fn extensions_serve_the_toolbar_and_the_page_from_one_list(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    EXTENSIONS.with(|list| {
        list.borrow_mut().extend([
            FakeExtension { id: "bbb", name: "beta", permissions: "Read x\nChange y", site_access: 3, popup: "chrome-extension://bbb/p.html", enabled: true, unpacked: false },
            FakeExtension { id: "aaa", name: "Alpha", permissions: "", site_access: 0, popup: "", enabled: false, unpacked: false },
            FakeExtension { id: "ccc", name: "charlie", permissions: "Z", site_access: 2, popup: "", enabled: true, unpacked: true },
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
