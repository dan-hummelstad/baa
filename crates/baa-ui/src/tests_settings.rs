//! The settings window, `baa://` routing and the command bar's page autofill,
//! driven through real clicks and keystrokes against `MockHost`.

use baa_model::{Permission, PermissionState, Store, Zone};
use baa_web::{ClearData, ExtensionChange, ExtensionInfo};
use gpui::{size, px, Entity, Modifiers, TestAppContext, VisualTestContext, WindowHandle};

use crate::{
    actions::*,
    mock::MockHost,
    settings::{rows, SettingsView, Tab},
    Browser,
};

type Settings = WindowHandle<SettingsView<MockHost>>;

fn browser(cx: &mut TestAppContext) -> (Entity<Browser<MockHost>>, &mut VisualTestContext) {
    cx.update(crate::init);
    cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx))
}

/// ⌘, in the browser, then the new window's context.
fn open_settings(b: &Entity<Browser<MockHost>>, vcx: &mut VisualTestContext) -> Settings {
    vcx.dispatch_action(OpenSettings);
    vcx.run_until_parked();
    b.read_with(vcx, |b, _| b.settings_window).expect("the settings window opened")
}

/// A settings window tall enough to lay every row out, so any can be clicked.
fn tall(h: Settings, cx: &TestAppContext) -> VisualTestContext {
    let v = VisualTestContext::from_window(h.into(), cx);
    v.simulate_resize(size(px(900.), px(4000.)));
    v.run_until_parked();
    v
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing is drawn as {selector}"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

fn drawn(cx: &mut VisualTestContext, selector: &'static str) -> bool {
    cx.run_until_parked();
    cx.debug_bounds(selector).is_some()
}

fn tab(h: Settings, cx: &mut VisualTestContext) -> Tab {
    h.update(cx, |v, _, _| v.tab).unwrap()
}

fn settings_windows(cx: &TestAppContext) -> usize {
    cx.windows().into_iter().filter(|w| w.downcast::<SettingsView<MockHost>>().is_some()).count()
}

#[gpui::test]
fn the_window_opens_once_focuses_and_closes_with_escape_or_cmd_w(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    let h = open_settings(&b, vcx);
    assert_eq!(settings_windows(cx), 1);
    let mut s = VisualTestContext::from_window(h.into(), cx);
    assert_eq!(tab(h, &mut s), Tab::General);

    // Again from the browser: the same window, and a tab it is moved to.
    let vcx = &mut VisualTestContext::from_window(b.read_with(&s, |b, _| b.window_handle()), cx);
    vcx.dispatch_action(OpenSettings);
    vcx.run_until_parked();
    assert_eq!(settings_windows(cx), 1, "one settings window per app");

    // Esc clears the search first, then closes.
    let mut s = VisualTestContext::from_window(h.into(), cx);
    s.simulate_input("zoom");
    assert_eq!(h.update(&mut s, |v, _, _| v.results()[0].id).unwrap(), "zoom");
    s.simulate_keystrokes("escape");
    assert!(h.update(&mut s, |v, _, _| !v.searching()).unwrap());
    assert_eq!(settings_windows(cx), 1, "the first Esc only cleared the text");
    s.simulate_keystrokes("escape");
    assert_eq!(settings_windows(cx), 0);

    // ⌘W closes too, and the window can come back.
    let vcx = &mut VisualTestContext::from_window(b.read_with(cx, |b, _| b.window_handle()), cx);
    let h = open_settings(&b, vcx);
    let mut s = VisualTestContext::from_window(h.into(), cx);
    s.simulate_keystrokes("cmd-w");
    assert_eq!(settings_windows(cx), 0);
}

#[gpui::test]
fn tabs_switch_by_click_and_by_control_tab(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    let h = open_settings(&b, vcx);
    let mut s = tall(h, cx);
    click(&mut s, "tab:Privacy");
    assert_eq!(tab(h, &mut s), Tab::Privacy);
    assert!(drawn(&mut s, "row:cookies") && !drawn(&mut s, "row:zoom"));
    s.simulate_keystrokes("ctrl-tab");
    assert_eq!(tab(h, &mut s), Tab::Sites);
    s.simulate_keystrokes("ctrl-shift-tab ctrl-shift-tab");
    assert_eq!(tab(h, &mut s), Tab::Search);
    // The pill is on its way to the new tab, and the body crossfades in.
    assert!(h.update(&mut s, |v, _, _| v.motion.target(("settings-pill", 0), crate::motion::Feel::Glide)).unwrap().is_some());
}

#[gpui::test]
fn search_finds_rows_and_jumps_to_them(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    let h = open_settings(&b, vcx);
    let mut s = tall(h, cx);
    s.simulate_input("zoom");
    s.run_until_parked();
    assert!(drawn(&mut s, "result:zoom"), "Default page zoom is listed");
    assert_eq!(rows::find("zoom").map(|r| r.tab), Some(Tab::Appearance));
    s.simulate_keystrokes("enter");
    assert_eq!(tab(h, &mut s), Tab::Appearance);
    assert!(drawn(&mut s, "row:zoom"));
    assert!(h.update(&mut s, |v, _, _| !v.searching()).unwrap());

    s.simulate_input("cooki");
    s.simulate_keystrokes("down down");
    assert_eq!(h.update(&mut s, |v, _, _| v.selected).unwrap(), 2);
    click(&mut s, "result:cookies");
    assert_eq!(tab(h, &mut s), Tab::Privacy);
}

#[gpui::test]
fn a_toggle_saves_the_setting_and_pushes_it_to_the_engine(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    let h = open_settings(&b, vcx);
    let mut s = tall(h, cx);
    click(&mut s, "tab:Search");
    assert!(b.read_with(&s, |b, _| b.settings.search_suggest));
    click(&mut s, "pref-suggest");
    b.read_with(&s, |b, _| {
        assert!(!b.settings.search_suggest);
        assert!(!b.store().unwrap().load_settings().unwrap().search_suggest, "saved");
        assert!(!b.host.applied.borrow().as_ref().unwrap().search_suggest, "pushed");
    });
    // A segment of a global web preference.
    click(&mut s, "tab:Privacy");
    click(&mut s, "pref-cookies:2");
    b.read_with(&s, |b, _| {
        assert_eq!(b.host.applied.borrow().as_ref().unwrap().web.cookies, baa_model::Cookies::BlockAll);
        assert_eq!(b.store().unwrap().load_settings().unwrap().web.cookies, baa_model::Cookies::BlockAll);
    });
}

#[gpui::test]
fn settings_are_applied_at_start_up(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    b.read_with(vcx, |b, _| assert_eq!(b.host.applied.borrow().as_ref(), Some(&b.settings)));
}

#[gpui::test]
fn site_rules_are_edited_in_the_chosen_space(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    let work = b.update_in(vcx, |b, window, cx| {
        b.new_space(window, cx);
        b.session.active_space().unwrap()
    });
    let work_profile = b.read_with(vcx, |b, _| b.session.space(work).unwrap().profile);
    b.update(vcx, |b, _| {
        *b.host.rules.borrow_mut() = vec![
            (Permission::Popups, "*".into(), PermissionState::Block),
            (Permission::Popups, "[*.]a.com".into(), PermissionState::Allow),
        ];
    });
    let h = open_settings(&b, vcx);
    let mut s = tall(h, cx);
    click(&mut s, "tab:Sites");
    assert!(drawn(&mut s, "rule:site.popups:[*.]a.com") && drawn(&mut s, "row:site.popups"), "both are drawn");

    // The Space picker: Work is the second.
    click(&mut s, "space-picker:1");
    click(&mut s, "rule:site.popups:Block");
    let sets = b.read_with(&s, |b, _| b.host.rule_sets.borrow().clone());
    assert_eq!(sets, [(work_profile, Permission::Popups, "[*.]a.com".to_string(), Some(PermissionState::Block))]);
    click(&mut s, "rule-remove:site.popups:[*.]a.com");
    let last = b.read_with(&s, |b, _| b.host.rule_sets.borrow().last().cloned()).unwrap();
    assert_eq!(last, (work_profile, Permission::Popups, "[*.]a.com".to_string(), None));
    assert!(!drawn(&mut s, "rule:site.popups:Allow"), "the exception is gone");

    // A new exception: typed bare, it covers subdomains.
    click(&mut s, "add-rule:site.popups");
    s.simulate_input("b.org");
    click(&mut s, "add-state:site.popups:Allow");
    let last = b.read_with(&s, |b, _| b.host.rule_sets.borrow().last().cloned()).unwrap();
    assert_eq!(last, (work_profile, Permission::Popups, "[*.]b.org".to_string(), Some(PermissionState::Allow)));

    // The default is global: it lands in baa-model, not in one profile.
    click(&mut s, "site.popups:0");
    b.read_with(&s, |b, _| {
        assert_eq!(b.settings.web.site_defaults, [(Permission::Popups, PermissionState::Allow)]);
    });
}

#[gpui::test]
fn clearing_data_asks_each_stored_space_and_forgets_recent_history(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    b.update_in(vcx, |b, window, cx| {
        b.new_space(window, cx);
        b.new_incognito_space(window, cx);
    });
    let now = baa_model::now() as i64;
    b.update(vcx, |b, _| {
        let store = b.store().unwrap();
        store.record_visit("https://old.example", "", now - 7200).unwrap();
        store.record_visit("https://new.example", "", now - 60).unwrap();
    });
    let profiles = b.read_with(vcx, |b, _| b.session.spaces().iter().filter(|s| !s.ephemeral).map(|s| s.profile).collect::<Vec<_>>());
    assert_eq!(profiles.len(), 2);
    let h = open_settings(&b, vcx);
    let mut s = tall(h, cx);
    click(&mut s, "tab:Privacy");
    click(&mut s, "clear-open");
    assert!(drawn(&mut s, "clear-sheet"));
    click(&mut s, "clear-cache"); // history and site data stay ticked, cache off
    click(&mut s, "clear-spaces:1");
    click(&mut s, "clear-confirm");

    b.read_with(&s, |b, _| {
        let clears = b.host.clears.borrow();
        assert_eq!(clears.len(), 2, "one per stored Space, none for incognito");
        assert_eq!(clears.iter().map(|c| c.0).collect::<Vec<_>>(), profiles);
        let want = ClearData { history: true, downloads: false, site_data: true, cache: false };
        assert!(clears.iter().all(|c| c.2 == want));
        let ago = std::time::SystemTime::now().duration_since(clears[0].1.unwrap()).unwrap();
        assert!((3590..3610).contains(&ago.as_secs()), "last hour, not {ago:?}");
        let left: Vec<String> = b.store().unwrap().recent_history(10).unwrap().into_iter().map(|h| h.url).collect();
        assert_eq!(left, ["https://old.example"]);
    });
    assert!(!drawn(&mut s, "clear-sheet"), "the sheet closes");
}

#[gpui::test]
fn developer_mode_loads_unpacked_extensions_and_reloads_them(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    let packed = ExtensionInfo { id: "store".into(), name: "From the store".into(), enabled: true, ..Default::default() };
    let unpacked = ExtensionInfo { id: "dev".into(), name: "Mine".into(), enabled: true, unpacked: true, ..Default::default() };
    b.update(vcx, |b, _| *b.host.extension_details.borrow_mut() = vec![packed, unpacked]);
    let h = open_settings(&b, vcx);
    let mut s = tall(h, cx);
    click(&mut s, "tab:Extensions");
    assert!(drawn(&mut s, "ext:store") && drawn(&mut s, "ext:dev"));
    assert!(!drawn(&mut s, "ext-load") && !drawn(&mut s, "ext-reload"), "developer mode is off");

    click(&mut s, "pref-dev-mode");
    assert!(b.read_with(&s, |b, _| b.settings.web.developer_mode));
    assert!(b.read_with(&s, |b, _| b.host.applied.borrow().as_ref().unwrap().web.developer_mode));
    assert!(drawn(&mut s, "ext-load"));
    click(&mut s, "ext-reload");
    b.read_with(&s, |b, _| {
        assert_eq!(*b.host.extension_changes.borrow(), [("dev".to_string(), ExtensionChange::Reload)], "only the unpacked one reloads")
    });

    // The folder picker is the platform's; what it returns goes to the chosen Space.
    let profile = b.read_with(&s, |b, _| b.session.space(b.session.active_space().unwrap()).unwrap().profile);
    h.update(&mut s, |v, _, cx| v.load_unpacked_from(std::path::Path::new("/tmp/ext"), cx)).unwrap();
    s.run_until_parked();
    b.read_with(&s, |b, _| assert_eq!(*b.host.unpacked.borrow(), [(profile, "/tmp/ext".into())]));
    b.read_with(&s, |b, _| *b.host.unpacked_result.borrow_mut() = Err("Manifest file is missing".into()));
    h.update(&mut s, |v, _, cx| v.load_unpacked_from(std::path::Path::new("/tmp/bad"), cx)).unwrap();
    s.run_until_parked();
    b.read_with(&s, |b, _| assert_eq!(b.extensions_page.error.as_deref(), Some("Manifest file is missing")));
}

#[gpui::test]
fn extensions_switch_off_and_leave_the_toolbar(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    let info = ExtensionInfo { id: "ubo".into(), name: "uBO Lite".into(), enabled: true, ..Default::default() };
    b.update(vcx, |b, _| *b.host.extension_details.borrow_mut() = vec![info]);
    b.update_in(vcx, |b, window, cx| b.open_url("chrome://extensions/?id=ubo", window, cx));
    vcx.run_until_parked();
    let h = b.read_with(vcx, |b, _| b.settings_window).unwrap();
    let mut s = tall(h, cx);
    assert_eq!(tab(h, &mut s), Tab::Extensions);
    b.read_with(&s, |b, _| assert_eq!(b.extensions_page.open.as_deref(), Some("ubo")));
    click(&mut s, "ext-on");
    b.read_with(&s, |b, _| {
        assert_eq!(*b.host.extension_changes.borrow(), [("ubo".to_string(), ExtensionChange::Enable(false))]);
        assert!(!b.extension_enabled("ubo"));
    });
}

#[gpui::test]
fn baa_addresses_open_the_window_or_a_web_ui_tab(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    let open = |b: &Entity<Browser<MockHost>>, url: &str, vcx: &mut VisualTestContext| {
        b.update_in(vcx, |b, window, cx| b.open_url(url, window, cx));
        vcx.run_until_parked();
    };
    open(&b, "baa://settings/privacy", vcx);
    let h = b.read_with(vcx, |b, _| b.settings_window).unwrap();
    assert!(b.read_with(vcx, |b, _| b.host.created.borrow().is_empty()), "no page for it");
    assert_eq!(tab(h, vcx), Tab::Privacy);

    open(&b, "chrome://settings/content/popups", vcx);
    assert_eq!(tab(h, vcx), Tab::Sites);
    assert_eq!(vcx.update(|_, cx| cx.windows().len()), 1 + 1, "the browser and one settings window");

    open(&b, "baa://extensions/?id=x", vcx);
    assert_eq!(tab(h, vcx), Tab::Extensions);
    b.read_with(vcx, |b, _| assert_eq!(b.extensions_page.open.as_deref(), Some("x")));

    open(&b, "baa://flags", vcx);
    open(&b, "baa://settings/fonts", vcx);
    b.read_with(vcx, |b, _| {
        let made: Vec<String> = b.host.created.borrow().iter().map(|(_, u)| u.clone()).collect();
        assert_eq!(made, ["baa://flags", "baa://settings/fonts"]);
    });
    assert_eq!(urls(&b, vcx), ["baa://settings/fonts", "baa://flags"]);
}

fn urls(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) -> Vec<String> {
    b.read_with(cx, |b, _| {
        let s = b.session();
        s.tabs(s.active_space().unwrap(), Zone::Today).map(|t| t.url.clone()).collect()
    })
}

#[gpui::test]
fn the_command_bar_completes_baa_addresses_and_finds_settings(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    vcx.simulate_keystrokes("secondary-t");
    vcx.simulate_input("baa://se");
    let rows = b.read_with(vcx, |b, _| {
        let bar = b.bar.as_ref().unwrap();
        crate::command_bar::rows(&bar.field.text, b.session(), &b.settings, &[])
    });
    match &rows[0].item {
        crate::command_bar::Item::Page(p) => assert_eq!(p.url, "baa://settings"),
        other => panic!("{other:?}"),
    }
    assert!(rows.len() <= 7 && matches!(rows.last().unwrap().item, crate::command_bar::Item::Open(_)), "pages first, then the typed Open");
    vcx.simulate_keystrokes("down tab");
    b.read_with(vcx, |b, _| assert_eq!(b.bar.as_ref().unwrap().field.text, "baa://settings/reset"));
    assert_eq!(b.read_with(vcx, |b, _| b.bar.as_ref().unwrap().selected), 0);

    // Enter on an address opens the tab of the window it names.
    vcx.simulate_keystrokes("cmd-a");
    vcx.simulate_input("baa://extensions");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let h = b.read_with(vcx, |b, _| b.settings_window).unwrap();
    assert_eq!(tab(h, vcx), Tab::Extensions);

    // Plain words find setting rows.
    vcx.simulate_keystrokes("secondary-t");
    vcx.simulate_input("cookies");
    let rows = b.read_with(vcx, |b, _| {
        crate::command_bar::rows("cookies", b.session(), &b.settings, &[])
    });
    let at = rows.iter().position(|r| matches!(&r.item, crate::command_bar::Item::Setting(s) if s.id == "cookies")).expect("the Cookies row is listed");
    assert_eq!(rows[at].detail, "Settings › Privacy & Security");
    vcx.simulate_keystrokes(&"down ".repeat(at));
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(tab(h, vcx), Tab::Privacy);
}

#[gpui::test]
fn the_default_browser_row_asks_macos_only_when_baa_is_not_it(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    let h = open_settings(&b, vcx);
    let mut s = tall(h, cx);
    assert!(drawn(&mut s, "make-default"));
    click(&mut s, "make-default");
    b.read_with(&s, |b, _| assert_eq!(b.host.made_default.get(), 1));
    b.read_with(&s, |b, _| b.host.default_browser.set(true));
    // macOS answered while the window was behind; the next time it is in front the row is read again.
    h.update(&mut s, |v, _, cx| v.on_activate(cx)).unwrap();
    assert!(!drawn(&mut s, "make-default"));
}

#[gpui::test]
fn every_row_in_the_index_is_drawn_in_its_tab(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    let one = ExtensionInfo { id: "e".into(), name: "E".into(), enabled: true, ..Default::default() };
    b.update(vcx, |b, _| *b.host.extension_details.borrow_mut() = vec![one]);
    let h = open_settings(&b, vcx);
    let mut s = tall(h, cx);
    for tab in Tab::ALL {
        h.update(&mut s, |v, _, cx| v.select_tab(tab, cx)).unwrap();
        for row in rows::ROWS.iter().filter(|r| r.tab == tab) {
            let selector: &'static str = Box::leak(format!("row:{}", row.id).into_boxed_str());
            assert!(drawn(&mut s, selector), "{} is not drawn in {tab:?}", row.id);
        }
    }
}

#[gpui::test]
fn reset_puts_defaults_back_and_clears_site_exceptions_on_request(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    b.update(vcx, |b, cx| {
        b.settings.web.site_defaults = vec![(Permission::Popups, PermissionState::Allow)];
        b.settings.reduce_motion = true;
        *b.host.rules.borrow_mut() = vec![(Permission::Camera, "[*.]c.com".into(), PermissionState::Block)];
        b.save_settings(cx);
    });
    let h = open_settings(&b, vcx);
    let mut s = tall(h, cx);
    click(&mut s, "tab:System");
    click(&mut s, "reset");
    click(&mut s, "reset-sites");
    click(&mut s, "reset-confirm");
    b.read_with(&s, |b, _| {
        assert_eq!(b.settings, baa_model::Settings::default());
        assert_eq!(b.host.applied.borrow().as_ref(), Some(&b.settings));
        let sets = b.host.rule_sets.borrow();
        assert!(sets.iter().any(|(_, k, p, st)| *k == Permission::Camera && p == "[*.]c.com" && st.is_none()));
        assert!(sets.iter().any(|(_, k, p, st)| *k == Permission::Popups && p == "*" && st.is_none()));
    });
}

#[gpui::test]
fn the_keymap_row_creates_the_file_and_the_shortcuts_are_listed(cx: &mut TestAppContext) {
    let dir = std::env::temp_dir().join(format!("baa-keymap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    cx.update(|cx| cx.set_global(crate::KeymapPath(dir.join("keymap.json"))));
    let (b, vcx) = browser(cx);
    let h = open_settings(&b, vcx);
    let mut s = tall(h, cx);
    click(&mut s, "tab:Keyboard");
    assert!(drawn(&mut s, "shortcut:New tab or search"), "an action with its keys");
    assert!(drawn(&mut s, "edit-keymap"));
    // The button hands the file to the system's editor, which a test platform lacks.
    crate::settings::ensure_keymap(&dir.join("keymap.json")).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("keymap.json")).unwrap().trim(), "[]");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn launch_settings_shape_the_session_and_links(cx: &mut TestAppContext) {
    // A restored extensions tab has no page any more.
    cx.update(crate::init);
    let store = Store::open(":memory:").unwrap();
    let mut session = baa_model::Session::first_run();
    let sp = session.active_space().unwrap();
    session.open_tab(sp, "https://a.example", 1);
    session.open_tab(sp, "baa://extensions", 2);
    session.open_tab(sp, "baa://extensions-internals", 2);
    let internals = session.tabs(sp, Zone::Today).find(|t| t.url.ends_with("internals")).unwrap().id;
    session.toggle_pin(internals);
    let kept = session.open_tab(sp, "https://b.example", 3);
    session.activate_tab(kept, 3);
    store.save_session(&session).unwrap();
    store.save_settings(&baa_model::Settings { restore_today: false, ..Default::default() }).unwrap();
    let (b, vcx) = cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), store, window, cx));
    assert_eq!(urls(&b, vcx), ["https://b.example"], "extensions page dropped, Today archived but the active tab");
    assert_eq!(urls(&b, vcx).len(), 1);
    let pinned: Vec<String> = b.read_with(vcx, |b, _| b.session().tabs(b.session().active_space().unwrap(), Zone::Pinned).map(|t| t.url.clone()).collect());
    assert_eq!(pinned, ["baa://extensions-internals"], "a Chrome page that only starts with the same letters stays");
    b.read_with(vcx, |b, _| assert_eq!(b.session().archived().len(), 1));

    // "Links that open a new tab: New tab" skips Peek.
    b.update(vcx, |b, cx| {
        b.settings.peek_links = false;
        b.save_settings(cx);
    });
    let page = b.read_with(vcx, |b, _| b.pages.values().next().unwrap().0.clone());
    page.update(vcx, |_, cx| {
        cx.emit(baa_web::PageEvent::OpenRequested { url: "https://c.example".into(), from_user_gesture: true, background: false })
    });
    vcx.run_until_parked();
    assert!(b.read_with(vcx, |b, _| b.peek.is_none()));
    assert_eq!(urls(&b, vcx)[0], "https://c.example");
}

#[gpui::test]
fn a_jump_scrolls_the_row_into_the_body_at_the_real_window_size(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    let h = open_settings(&b, vcx);
    let mut s = VisualTestContext::from_window(h.into(), cx);
    h.update(&mut s, |v, _, cx| v.go_to(Some(Tab::Sites), Some("site.windows"), cx)).unwrap();
    for _ in 0..4 {
        s.run_until_parked();
        h.update(&mut s, |_, window, cx| {
            window.refresh();
            cx.notify();
        })
        .unwrap();
    }
    s.run_until_parked();
    let body = s.debug_bounds("body").expect("body");
    let row = s.debug_bounds("row:site.windows").expect("the row is drawn");
    assert!(row.top() >= body.top() && row.bottom() <= body.bottom(), "{row:?} is outside {body:?}");
}

#[gpui::test]
fn languages_and_dns_apply_on_enter_and_dns_must_be_valid(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    let h = open_settings(&b, vcx);
    let mut s = tall(h, cx);
    click(&mut s, "languages-field");
    s.simulate_input("sv");
    assert_eq!(b.read_with(&s, |b, _| b.settings.web.languages.clone()), None, "not per keystroke");
    s.simulate_keystrokes("enter");
    assert_eq!(b.read_with(&s, |b, _| b.settings.web.languages.clone()).as_deref(), Some("sv"));

    click(&mut s, "tab:Privacy");
    click(&mut s, "pref-dns:4"); // Custom, nothing typed yet
    assert_eq!(b.read_with(&s, |b, _| b.settings.web.secure_dns.clone()), baa_model::SecureDns::Automatic, "never Custom(\"\")");
    s.simulate_input("not a url");
    s.simulate_keystrokes("enter");
    assert_eq!(b.read_with(&s, |b, _| b.settings.web.secure_dns.clone()), baa_model::SecureDns::Automatic);
    s.simulate_keystrokes("cmd-a");
    s.simulate_input("https://dns.example/q");
    s.simulate_keystrokes("enter");
    assert_eq!(b.read_with(&s, |b, _| b.settings.web.secure_dns.clone()), baa_model::SecureDns::Custom("https://dns.example/q".into()));
}

#[gpui::test]
fn a_refused_exception_keeps_the_form_open_with_a_reason(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    b.read_with(vcx, |b, _| b.host.refuse_rules.set(true));
    let h = open_settings(&b, vcx);
    let mut s = tall(h, cx);
    click(&mut s, "tab:Sites");
    click(&mut s, "add-rule:site.camera");
    s.simulate_input("x.org");
    click(&mut s, "add-state:site.camera:Allow");
    assert!(drawn(&mut s, "add-error:site.camera") && drawn(&mut s, "add-form:site.camera"));
}

#[gpui::test]
fn clearing_nothing_calls_nothing(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    let h = open_settings(&b, vcx);
    let mut s = tall(h, cx);
    click(&mut s, "tab:Privacy");
    click(&mut s, "clear-open");
    for id in ["clear-history", "clear-site-data", "clear-cache"] {
        click(&mut s, id);
    }
    click(&mut s, "clear-confirm");
    b.read_with(&s, |b, _| assert!(b.host.clears.borrow().is_empty()));
}

#[gpui::test]
fn the_site_details_address_opens_one_site_and_sets_its_permission(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    b.update_in(vcx, |b, window, cx| b.open_url("chrome://settings/content/siteDetails?site=https://a.com", window, cx));
    vcx.run_until_parked();
    let h = b.read_with(vcx, |b, _| b.settings_window).expect("the settings window opened");
    assert_eq!(tab(h, vcx), Tab::Sites);
    let mut s = tall(h, cx);
    assert!(drawn(&mut s, "site-back") && !drawn(&mut s, "add-rule:site.popups"), "the one-site view, not the list");
    click(&mut s, "site:site.camera:Block");
    b.read_with(&s, |b, _| {
        assert_eq!(b.host.permissions.borrow().last(), Some(&("https://a.com".to_string(), Permission::Camera, PermissionState::Block)));
    });
    click(&mut s, "site-back");
    assert!(drawn(&mut s, "add-rule:site.popups"), "back to the list");
}

#[gpui::test]
fn the_claude_switch_goes_through_the_browser(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    let h = open_settings(&b, vcx);
    let mut s = tall(h, cx);
    h.update(&mut s, |v, _, cx| v.select_tab(Tab::Claude, cx)).unwrap();
    assert!(!b.read_with(&s, |b, _| b.settings.claude.enabled));
    click(&mut s, "pref-claude");
    assert!(b.read_with(&s, |b, _| b.settings.claude.enabled));
}
