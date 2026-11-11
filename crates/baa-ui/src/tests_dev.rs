//! Developer tools (E1–E4): environments in the header, ⌥⌘E and the settings editor, the dev
//! servers folder, the responsive view and the console error badge.

use baa_model::{Env, LiveSource, Project, Store};
use baa_web::{ConsoleLevel, PageEvent};
use gpui::{px, size, Entity, Modifiers, TestAppContext, VisualTestContext};

use crate::{actions::*, mock::MockHost, Browser};

fn browser(cx: &mut TestAppContext) -> (Entity<Browser<MockHost>>, &mut VisualTestContext) {
    cx.update(crate::init);
    cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx))
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing is drawn as {selector}"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

fn shop() -> Vec<Project> {
    let env = |label: &str, origin: &str| Env { label: label.into(), origin: origin.into() };
    vec![Project { name: "shop".into(), envs: vec![env("Local", "http://localhost:3000"), env("Prod", "https://shop.com")] }]
}

#[gpui::test]
fn the_header_names_the_env_and_cycle_moves_the_tab_keeping_path_and_query(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    b.update_in(cx, |b, window, cx| {
        b.settings.environments = shop();
        b.open_url("http://localhost:3000/cart?id=1", window, cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("env-badge").is_some());

    cx.dispatch_action(CycleEnvironment);
    cx.run_until_parked();
    b.read_with(cx, |b, cx| {
        let tab = b.session.tab(b.active_tab().unwrap()).unwrap();
        assert_eq!(tab.url, "https://shop.com/cart?id=1");
        assert_eq!(*b.active_page().unwrap().read(cx).loads.borrow(), ["https://shop.com/cart?id=1"], "the same tab, not a new one");
    });

    b.update_in(cx, |b, window, cx| b.open_url("https://other.com/", window, cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("env-badge").is_none());
    cx.dispatch_action(CycleEnvironment);
    cx.run_until_parked();
    b.read_with(cx, |b, _| assert_eq!(b.session.tab(b.active_tab().unwrap()).unwrap().url, "https://other.com/"));
}

#[gpui::test]
fn the_developer_tab_adds_envs_with_valid_origins_only(cx: &mut TestAppContext) {
    let (b, vcx) = browser(cx);
    vcx.dispatch_action(OpenSettings);
    vcx.run_until_parked();
    let h = b.read_with(vcx, |b, _| b.settings_window).unwrap();
    let mut s = VisualTestContext::from_window(h.into(), cx);
    s.simulate_resize(size(px(900.), px(4000.)));
    click(&mut s, "tab:Developer");
    click(&mut s, "env-project");
    s.simulate_input("shop");
    click(&mut s, "env-label");
    s.simulate_input("Prod");
    click(&mut s, "env-origin");
    s.simulate_input("https://shop.com/cart");
    s.simulate_keystrokes("enter");
    assert!(b.read_with(&s, |b, _| b.settings.environments.is_empty()), "a path is not an origin");
    s.simulate_keystrokes("cmd-a");
    s.simulate_input("shop.com");
    click(&mut s, "env-add");
    let want = vec![Project { name: "shop".into(), envs: vec![Env { label: "Prod".into(), origin: "https://shop.com".into() }] }];
    assert_eq!(b.read_with(&s, |b, _| b.settings.environments.clone()), want);
    click(&mut s, "env-remove:0:0");
    assert!(b.read_with(&s, |b, _| b.settings.environments.is_empty()));
}

#[gpui::test]
fn the_dev_servers_folder_goes_in_a_saved_space_never_an_incognito_one(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let count = |b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext| {
        b.read_with(cx, |b, _| {
            let s = &b.session;
            s.spaces().iter().flat_map(|sp| s.folders(sp.id).map(move |f| (sp.ephemeral, f.live.clone()))).collect::<Vec<_>>()
        })
    };
    b.update_in(cx, |b, window, cx| {
        // An incognito Space beside saved ones, as a session from before incognito windows had.
        let id = b.session.add_ephemeral_space("Incognito", baa_web::WebHost::ephemeral_profile(&*b.host));
        b.activate_space(id, window, cx);
        b.set_dev_servers_folder(true, window, cx);
        b.set_dev_servers_folder(true, window, cx);
    });
    assert_eq!(count(&b, cx), [(false, Some(LiveSource::DevServers))], "one folder, in the saved Space");
    b.update_in(cx, |b, window, cx| b.set_dev_servers_folder(false, window, cx));
    assert!(count(&b, cx).is_empty());
}

#[gpui::test]
fn responsive_view_splits_off_a_phone_pane_that_ends_the_split_when_closed(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    b.update_in(cx, |b, window, cx| b.open_url("http://localhost:3000/cart", window, cx));
    let first = b.read_with(cx, |b, _| b.active_tab().unwrap());
    cx.simulate_keystrokes("secondary-alt-m");
    cx.run_until_parked();
    let phone = b.read_with(cx, |b, cx| {
        let s = b.session();
        let split = s.shown_split(s.active_space().unwrap()).expect("a split").tabs.clone();
        assert_eq!(split.len(), 2);
        assert_eq!(split[0], first);
        let phone = split[1];
        assert_eq!(s.tab(phone).unwrap().url, "http://localhost:3000/cart", "the same address");
        assert!(b.phone_panes.contains(&phone));
        assert_eq!(*b.pages[&phone].0.read(cx).cdp.borrow(), ["watch_console", "emulation Some(Phone)"]);
        assert!(!b.pages[&first].0.read(cx).cdp.borrow().iter().any(|c| c.starts_with("emulation")), "the first pane stays a desktop page");
        phone
    });
    click(cx, "phone-reload");
    assert_eq!(b.read_with(cx, |b, cx| b.pages[&phone].0.read(cx).reloads.get()), 1, "its own reload");

    // ⌥⌘M again closes the phone pane; the split of one dissolves.
    cx.simulate_keystrokes("secondary-alt-m");
    cx.run_until_parked();
    b.read_with(cx, |b, _| {
        assert!(b.session().tab(phone).is_none());
        assert!(b.session().spaces()[0].split.is_none());
        assert!(b.phone_panes.is_empty());
        assert_eq!(b.active_tab(), Some(first));
    });

    // Closing it like any tab does the same.
    cx.dispatch_action(ResponsiveView);
    cx.run_until_parked();
    let phone = b.read_with(cx, |b, _| *b.phone_panes.iter().next().expect("a phone pane"));
    b.update_in(cx, |b, window, cx| b.close_tab_id(phone, window, cx));
    cx.run_until_parked();
    b.read_with(cx, |b, _| {
        assert!(b.session().spaces()[0].split.is_none());
        assert!(b.phone_panes.is_empty());
    });
}

#[gpui::test]
fn responsive_view_needs_a_web_page(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    b.update_in(cx, |b, window, cx| b.open_url("about:blank", window, cx));
    cx.dispatch_action(ResponsiveView);
    cx.run_until_parked();
    b.read_with(cx, |b, _| {
        assert!(b.session().spaces()[0].split.is_none());
        assert!(b.phone_panes.is_empty());
    });
}

#[gpui::test]
fn localhost_tabs_count_console_errors_and_the_badge_opens_devtools(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let error = |text: &str| PageEvent::Console { level: ConsoleLevel::Error, text: text.into() };
    b.update_in(cx, |b, window, cx| b.open_url("https://example.com/", window, cx));
    let (web, web_page) = b.read_with(cx, |b, _| (b.active_tab().unwrap(), b.active_page().unwrap().clone()));
    web_page.update(cx, |_, cx| cx.emit(error("x")));
    b.update_in(cx, |b, window, cx| b.open_url("http://127.0.0.1:5173/", window, cx));
    let (local, page) = b.read_with(cx, |b, _| (b.active_tab().unwrap(), b.active_page().unwrap().clone()));
    page.update(cx, |_, cx| {
        cx.emit(error("boom"));
        cx.emit(PageEvent::Console { level: ConsoleLevel::Warning, text: "meh".into() });
        cx.emit(error("again"));
    });
    cx.run_until_parked();
    b.read_with(cx, |b, cx| {
        assert_eq!(*b.pages[&local].0.read(cx).cdp.borrow(), ["watch_console"]);
        assert!(b.pages[&web].0.read(cx).cdp.borrow().is_empty(), "a remote page is never watched");
        assert_eq!(b.console_errors.get(&local), Some(&2), "errors only");
        assert_eq!(b.console_errors.get(&web), None);
    });

    // The badge opens DevTools on its tab, from another tab.
    b.update_in(cx, |b, window, cx| b.activate_tab(web, window, cx));
    // The new rows grow in first (frames only draw when asked under test).
    for _ in 0..40 {
        cx.executor().advance_clock(std::time::Duration::from_millis(33));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
    click(cx, "console-badge");
    b.read_with(cx, |b, _| {
        assert_eq!(b.active_tab(), Some(local));
        assert!(b.devtools.contains_key(&local));
    });

    // A new load starts the count over.
    page.update(cx, |_, cx| cx.emit(PageEvent::LoadingChanged(true)));
    cx.run_until_parked();
    b.read_with(cx, |b, _| assert_eq!(b.console_errors.get(&local), None));
    assert!(cx.debug_bounds("console-badge").is_none());

    // Off in settings: no new page is watched.
    b.update_in(cx, |b, window, cx| {
        b.settings.console_badge = false;
        b.open_url("http://localhost:8080/", window, cx);
    });
    b.read_with(cx, |b, cx| assert!(b.active_page().unwrap().read(cx).cdp.borrow().is_empty()));
}
