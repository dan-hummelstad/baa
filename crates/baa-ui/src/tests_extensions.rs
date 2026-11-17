//! Extensions' toolbar actions: badges, the activate shortcut, and popups they open themselves.

use baa_model::Store;
use baa_web::{ExtensionAction, ExtensionActionChanged, ExtensionCommand, ExtensionPopupOpened, PageEvent, ProfileId};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};

use crate::{mock::MockHost, Browser};

const POPUP: &str = "chrome-extension://ubo/popup.html";

fn browser(cx: &mut TestAppContext) -> (Entity<Browser<MockHost>>, &mut VisualTestContext) {
    cx.update(crate::init);
    let (b, cx) = cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx));
    let ext = baa_web::Extension { id: "ubo".into(), name: "uBO".into(), icon: None, popup: Some(POPUP.into()), options: None };
    b.update(cx, |b, _| *b.host.extensions.borrow_mut() = vec![ext]);
    open(cx, "a.example");
    (b, cx)
}

fn open(cx: &mut VisualTestContext, url: &str) {
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input(url);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
}

fn action(badge: &str) -> ExtensionAction {
    ExtensionAction {
        extension_id: "ubo".into(),
        badge_text: badge.into(),
        title: "uBO: 3 blocked".into(),
        badge_rgba: Some(0xFF0000FF),
        text_rgba: None,
        enabled: true,
    }
}

fn profile(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) -> ProfileId {
    b.read_with(cx, |b, _| b.active_space().unwrap().profile)
}

fn changed(b: &Entity<Browser<MockHost>>, page: Option<gpui::EntityId>, cx: &mut VisualTestContext) {
    let profile = profile(b, cx);
    b.update(cx, |b, cx| b.host.emit_action_changed(ExtensionActionChanged { profile, extension_id: "ubo".into(), page }, cx));
    cx.run_until_parked();
}

fn badge(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) -> Option<String> {
    b.read_with(cx, |b, _| b.actions.get("ubo").map(|a| a.badge_text.clone()))
}

fn set_actions(b: &Entity<Browser<MockHost>>, page: Option<gpui::EntityId>, badge: &str, cx: &mut VisualTestContext) {
    b.update(cx, |b, _| b.host.actions.borrow_mut().insert(page, vec![action(badge)]));
}

#[gpui::test]
fn a_badge_is_drawn_over_its_icon_and_follows_changes(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    assert!(cx.debug_bounds("badge:ubo").is_none(), "no badge yet");
    set_actions(&b, None, "12", cx);
    changed(&b, None, cx);
    assert_eq!(badge(&b, cx).as_deref(), Some("12"));
    let at = cx.debug_bounds("badge:ubo").expect("drawn");
    assert!(at.size.width > gpui::px(0.));

    set_actions(&b, None, "", cx);
    changed(&b, None, cx);
    assert!(cx.debug_bounds("badge:ubo").is_none(), "an empty badge is none");
}

#[gpui::test]
fn a_page_change_counts_only_for_the_active_page(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let first = b.read_with(cx, |b, _| b.active_page().unwrap().entity_id());
    set_actions(&b, None, "d", cx);
    set_actions(&b, Some(first), "1", cx);
    changed(&b, Some(first), cx);
    assert_eq!(badge(&b, cx).as_deref(), Some("1"));

    open(cx, "b.example");
    assert_eq!(badge(&b, cx).as_deref(), Some("d"), "another tab shows its own");
    set_actions(&b, Some(first), "2", cx);
    changed(&b, Some(first), cx);
    assert_eq!(badge(&b, cx).as_deref(), Some("d"), "a background page's change waits");

    let first_tab = b.read_with(cx, |b, _| *b.pages.iter().find(|(_, (p, _))| p.entity_id() == first).unwrap().0);
    b.update_in(cx, |b, window, cx| b.activate_tab(first_tab, window, cx));
    assert_eq!(badge(&b, cx).as_deref(), Some("2"), "read again on activation");
}

#[gpui::test]
fn another_profiles_change_is_not_read(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    set_actions(&b, None, "x", cx);
    b.update(cx, |b, cx| b.host.emit_action_changed(ExtensionActionChanged { profile: ProfileId(4242), extension_id: "ubo".into(), page: None }, cx));
    cx.run_until_parked();
    assert_eq!(badge(&b, cx), None);
}

#[gpui::test]
fn a_disabled_action_is_still_listed_greyed(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    b.update(cx, |b, _| b.host.actions.borrow_mut().insert(None, vec![ExtensionAction { enabled: false, ..action("3") }]));
    changed(&b, None, cx);
    b.read_with(cx, |b, _| assert!(!b.actions["ubo"].enabled));
    assert!(cx.debug_bounds("badge:ubo").is_some());
}

fn command(key: &str, modifiers: Modifiers, action: bool) -> ExtensionCommand {
    let name = if action { "_execute_action" } else { "toggle" };
    ExtensionCommand { extension_id: "ubo".into(), name: name.into(), description: String::new(), key: key.into(), modifiers, action }
}

fn popups(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) -> Vec<String> {
    b.read_with(cx, |b, _| b.host.popups.borrow().clone())
}

fn listed(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) {
    set_actions(b, None, "", cx);
    changed(b, None, cx);
}

#[gpui::test]
fn the_activate_shortcut_opens_the_popup(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let cmd_shift = Modifiers { platform: true, shift: true, ..Default::default() };
    b.update(cx, |b, _| *b.host.commands.borrow_mut() = vec![command("Y", cmd_shift, true), command("U", cmd_shift, false)]);
    listed(&b, cx);
    cx.simulate_keystrokes("cmd-shift-u");
    assert!(popups(&b, cx).is_empty(), "a named command is the engine's");
    cx.simulate_keystrokes("cmd-shift-y");
    assert_eq!(popups(&b, cx), [POPUP]);
    b.read_with(cx, |b, _| assert!(b.peek.as_ref().is_some_and(|p| p.popup)));

    no_manifest_popup(&b, cx);
    cx.simulate_keystrokes("cmd-shift-y");
    assert_eq!(popups(&b, cx).last().map(String::as_str), Some("chrome-extension://ubo/"), "a click, not Settings");
    b.read_with(cx, |b, _| assert!(b.settings_window.is_none()));
}

#[gpui::test]
fn a_shifted_digit_shortcut_fires(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let cmd_shift = Modifiers { platform: true, shift: true, ..Default::default() };
    b.update(cx, |b, _| *b.host.commands.borrow_mut() = vec![command("1", cmd_shift, true)]);
    listed(&b, cx);
    cx.simulate_keystrokes("cmd-!");
    assert_eq!(popups(&b, cx), [POPUP]);
}

#[gpui::test]
fn the_activate_shortcut_ignores_key_repeat(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    b.update(cx, |b, _| *b.host.commands.borrow_mut() = vec![command("Y", Modifiers::command(), true)]);
    listed(&b, cx);
    let keystroke = gpui::Keystroke::parse("cmd-y").unwrap();
    cx.simulate_event(gpui::KeyDownEvent { keystroke, is_held: true, prefer_character_input: false });
    assert!(popups(&b, cx).is_empty());
}

#[gpui::test]
fn the_activate_shortcut_of_an_extension_without_an_action_does_nothing(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    b.update(cx, |b, _| *b.host.commands.borrow_mut() = vec![command("Y", Modifiers::command(), true)]);
    cx.simulate_keystrokes("cmd-y");
    assert!(popups(&b, cx).is_empty(), "the engine lists no action for a disabled extension");
}

#[gpui::test]
fn an_activate_shortcut_that_is_baas_does_nothing(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    b.update(cx, |b, _| *b.host.commands.borrow_mut() = vec![command("T", Modifiers::command(), true)]);
    listed(&b, cx);
    cx.simulate_keystrokes("cmd-t");
    assert!(popups(&b, cx).is_empty());
    b.read_with(cx, |b, _| assert!(b.bar.is_some(), "baa's own binding ran"));
}

#[gpui::test]
fn the_activate_shortcut_waits_for_a_prompt(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    b.update(cx, |b, _| *b.host.commands.borrow_mut() = vec![command("Y", Modifiers::command(), true)]);
    listed(&b, cx);
    let page = b.read_with(cx, |b, _| b.active_page().unwrap().clone());
    page.update(cx, |_, cx| {
        cx.emit(PageEvent::PermissionRequested { id: 1, origin: "https://a.example".into(), kinds: vec![baa_web::Permission::Camera] })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-y");
    assert!(popups(&b, cx).is_empty());
}

fn popup_opened(b: &Entity<Browser<MockHost>>, profile: ProfileId, cx: &mut VisualTestContext) {
    cx.update(|window, _| window.activate_window());
    b.update(cx, |b, cx| b.host.emit_popup_opened(ExtensionPopupOpened { profile, extension_id: "ubo".into() }, cx));
    cx.run_until_parked();
}

#[gpui::test]
fn a_popup_the_extension_opened_shows_as_its_click_would(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let profile = profile(&b, cx);
    b.update(cx, |b, cx| b.host.emit_popup_opened(ExtensionPopupOpened { profile, extension_id: "ubo".into() }, cx));
    cx.run_until_parked();
    assert!(popups(&b, cx).is_empty(), "a window in the background leaves it");
    popup_opened(&b, ProfileId(4242), cx);
    assert!(popups(&b, cx).is_empty(), "another profile's");
    popup_opened(&b, profile, cx);
    assert_eq!(popups(&b, cx), [POPUP]);
    b.read_with(cx, |b, _| assert!(b.peek.as_ref().is_some_and(|p| p.popup)));

    no_manifest_popup(&b, cx);
    popup_opened(&b, profile, cx);
    assert_eq!(popups(&b, cx).last().map(String::as_str), Some("chrome-extension://ubo/"), "one set by setPopup");
}

fn no_manifest_popup(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) {
    b.update(cx, |b, _| b.extensions.iter_mut().for_each(|e| e.popup = None));
}

#[gpui::test]
fn a_popup_the_extension_opened_never_covers_a_prompt(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let page = b.read_with(cx, |b, _| b.active_page().unwrap().clone());
    let dialog = baa_web::Dialog {
        id: 1,
        kind: baa_web::DialogKind::Alert,
        message: "hi".into(),
        default_text: String::new(),
        title: String::new(),
        ok_label: String::new(),
        cancel_label: String::new(),
        check_label: String::new(),
    };
    page.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog)));
    cx.run_until_parked();
    let profile = profile(&b, cx);
    popup_opened(&b, profile, cx);
    assert!(popups(&b, cx).is_empty());
    b.read_with(cx, |b, _| assert!(b.peek.is_none()));
}

fn settings_on_ubo(cx: &mut TestAppContext, commands: Vec<ExtensionCommand>) -> (Entity<Browser<MockHost>>, VisualTestContext) {
    let (b, vcx) = browser(cx);
    let info = baa_web::ExtensionInfo { id: "ubo".into(), name: "uBO".into(), enabled: true, ..Default::default() };
    b.update(vcx, |b, _| {
        *b.host.extension_details.borrow_mut() = vec![info];
        *b.host.commands.borrow_mut() = commands;
    });
    b.update_in(vcx, |b, window, cx| b.open_url("chrome://extensions/?id=ubo", window, cx));
    vcx.run_until_parked();
    let h = b.read_with(vcx, |b, _| b.settings_window).unwrap();
    let s = VisualTestContext::from_window(h.into(), cx);
    s.simulate_resize(gpui::size(gpui::px(900.), gpui::px(4000.)));
    s.run_until_parked();
    (b, s)
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

fn shortcuts(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) -> Vec<(String, String, String, Modifiers)> {
    b.read_with(cx, |b, _| b.host.shortcuts.borrow().clone())
}

#[gpui::test]
fn a_command_is_listed_then_recorded_and_cleared(cx: &mut TestAppContext) {
    let (b, mut s) = settings_on_ubo(cx, vec![ExtensionCommand { description: "Toggle it".into(), ..command("", Modifiers::none(), false) }]);
    assert!(drawn(&mut s, "cmd-change"), "listed in the details");
    assert!(!drawn(&mut s, "cmd-clear"), "nothing to clear: Not set");
    click(&mut s, "cmd-change");
    b.read_with(&s, |b, _| assert!(b.extensions_page.recording.is_some()));
    s.simulate_keystrokes("cmd-shift-k");
    let cmd_shift = Modifiers { platform: true, shift: true, ..Default::default() };
    assert_eq!(shortcuts(&b, &mut s), [("ubo".into(), "toggle".into(), "K".into(), cmd_shift)]);
    b.read_with(&s, |b, _| assert!(b.extensions_page.recording.is_none()));
    click(&mut s, "cmd-clear");
    assert_eq!(shortcuts(&b, &mut s).last().unwrap(), &("ubo".into(), "toggle".into(), String::new(), Modifiers::none()));
    assert!(!drawn(&mut s, "cmd-clear"));
}

#[gpui::test]
fn a_shifted_digit_is_recorded_as_digit_and_shift(cx: &mut TestAppContext) {
    let (b, mut s) = settings_on_ubo(cx, vec![command("", Modifiers::none(), false)]);
    click(&mut s, "cmd-change");
    s.simulate_keystrokes("cmd-!");
    let cmd_shift = Modifiers { platform: true, shift: true, ..Default::default() };
    assert_eq!(shortcuts(&b, &mut s), [("ubo".into(), "toggle".into(), "1".into(), cmd_shift)]);
}

#[gpui::test]
fn escape_stops_recording_without_a_change(cx: &mut TestAppContext) {
    let (b, mut s) = settings_on_ubo(cx, vec![command("", Modifiers::none(), false)]);
    click(&mut s, "cmd-change");
    s.simulate_keystrokes("escape");
    b.read_with(&s, |b, _| assert!(b.extensions_page.recording.is_none()));
    assert!(shortcuts(&b, &mut s).is_empty());
}

#[gpui::test]
fn a_shortcut_that_is_baas_shows_in_conflict_until_rebound(cx: &mut TestAppContext) {
    let (b, mut s) = settings_on_ubo(cx, vec![command("T", Modifiers::command(), true)]);
    let named = s.update(|_, cx| crate::browser::ext_actions::baa_binding(&gpui::Keystroke::parse("cmd-t").unwrap(), cx));
    assert_eq!(named.as_deref(), Some("New tab or search"));
    assert!(drawn(&mut s, "cmd-note"), "in conflict");
    click(&mut s, "cmd-change");
    s.simulate_keystrokes("cmd-shift-y");
    assert_eq!(shortcuts(&b, &mut s).len(), 1);
    assert!(!drawn(&mut s, "cmd-note"), "rebound away from baa's");
}

#[gpui::test]
fn a_shortcut_the_engine_refuses_says_so(cx: &mut TestAppContext) {
    let (b, mut s) = settings_on_ubo(cx, vec![command("", Modifiers::none(), false)]);
    b.update(&mut s, |b, _| b.host.refuse_shortcuts.set(true));
    click(&mut s, "cmd-change");
    s.simulate_keystrokes("cmd-alt-k");
    b.read_with(&s, |b, _| assert_eq!(b.extensions_page.refused, Some(("ubo".into(), "toggle".into()))));
    assert!(drawn(&mut s, "cmd-note"));
}
