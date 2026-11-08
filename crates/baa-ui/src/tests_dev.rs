//! Developer tools (E1, E3): environments in the header, ⌥⌘E and the settings editor, and the
//! dev servers folder.

use baa_model::{Env, LiveSource, Project, Store};
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
        b.new_incognito_space(window, cx);
        b.set_dev_servers_folder(true, window, cx);
        b.set_dev_servers_folder(true, window, cx);
    });
    assert_eq!(count(&b, cx), [(false, Some(LiveSource::DevServers))], "one folder, in the saved Space");
    b.update_in(cx, |b, window, cx| b.set_dev_servers_folder(false, window, cx));
    assert!(count(&b, cx).is_empty());
}
