//! Chrome's dialogs as sheets: HTTP auth, external protocol, JS dialogs.

use baa_model::Store;
use baa_web::{Dialog, DialogAnswer, DialogKind, PageEvent};
use gpui::{Entity, Keystroke, Modifiers, TestAppContext, VisualTestContext};

use crate::{mock::{MockHost, MockPage}, prompts::{wording, ACTIVATION_DELAY}, Browser};

fn dialog(id: u64, kind: DialogKind) -> Dialog {
    Dialog {
        id,
        kind,
        message: "msg".into(),
        default_text: String::new(),
        title: String::new(),
        ok_label: String::new(),
        cancel_label: String::new(),
        check_label: String::new(),
    }
}

fn page(cx: &mut TestAppContext) -> (Entity<Browser<MockHost>>, Entity<MockPage>, &mut VisualTestContext) {
    cx.update(crate::init);
    let (b, cx) = cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx));
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("a.example");
    cx.simulate_keystrokes("enter");
    let p = b.read_with(cx, |b, _| b.pages.get(&b.active_tab().unwrap()).unwrap().0.clone());
    (b, p, cx)
}

fn answered(p: &Entity<MockPage>, cx: &mut VisualTestContext) -> Vec<(u64, DialogAnswer)> {
    p.read_with(cx, |p, _| p.answered.borrow().clone())
}

#[gpui::test]
fn auth_takes_a_username_and_a_masked_password(cx: &mut TestAppContext) {
    let (b, p, cx) = page(cx);
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog(5, DialogKind::Auth))));
    cx.run_until_parked();
    cx.simulate_input("u");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("p");
    cx.executor().advance_clock(ACTIVATION_DELAY);
    b.read_with(cx, |b, _| {
        let d = b.active_dialog().unwrap();
        assert!(d.password.masked && !d.field.masked);
        assert_eq!((d.field.text.as_str(), d.password.text.as_str()), ("u", "p"));
    });
    cx.simulate_keystrokes("enter");
    let want = DialogAnswer { ok: true, text: "u".into(), password: "p".into(), checked: false };
    assert_eq!(answered(&p, cx), [(5, want)]);
}

#[gpui::test]
fn the_checkbox_answers_with_its_state(cx: &mut TestAppContext) {
    let (b, p, cx) = page(cx);
    let mut d = dialog(6, DialogKind::ExternalProtocol);
    d.check_label = "Always allow".into();
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(d)));
    cx.run_until_parked();
    let bounds = cx.debug_bounds("dialog-check").expect("the checkbox is drawn");
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    b.read_with(cx, |b, _| assert!(b.active_dialog().unwrap().checked));
    cx.executor().advance_clock(ACTIVATION_DELAY);
    // Opening an app is never accepted from the keyboard.
    cx.simulate_keystrokes("enter");
    assert!(answered(&p, cx).is_empty());
    let ok = cx.debug_bounds("dialog-ok").expect("OK is drawn");
    cx.simulate_click(ok.center(), Modifiers::none());
    assert_eq!(answered(&p, cx), [(6, DialogAnswer { ok: true, checked: true, ..Default::default() })]);
}

#[gpui::test]
fn enter_meant_for_an_alert_does_not_open_the_app_that_replaces_it(cx: &mut TestAppContext) {
    let (b, p, cx) = page(cx);
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog(20, DialogKind::Alert))));
    cx.run_until_parked();
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog(21, DialogKind::ExternalProtocol))));
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.executor().advance_clock(ACTIVATION_DELAY);
    cx.simulate_keystrokes("enter");
    assert!(answered(&p, cx).is_empty());
    b.read_with(cx, |b, _| assert_eq!(b.active_dialog().unwrap().dialog.id, 21));
    cx.simulate_keystrokes("escape");
    assert_eq!(answered(&p, cx), [(21, DialogAnswer::default())]);
}

#[gpui::test]
fn ok_waits_out_the_activation_delay(cx: &mut TestAppContext) {
    let (b, p, cx) = page(cx);
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog(22, DialogKind::Confirm))));
    cx.run_until_parked();
    let ok = cx.debug_bounds("dialog-ok").expect("OK is drawn");
    cx.simulate_click(ok.center(), Modifiers::none());
    assert!(answered(&p, cx).is_empty(), "a click right after it appears was aimed at something else");
    b.read_with(cx, |b, _| assert!(b.active_dialog().is_some()));
    cx.executor().advance_clock(ACTIVATION_DELAY);
    cx.simulate_click(ok.center(), Modifiers::none());
    assert_eq!(answered(&p, cx), [(22, DialogAnswer { ok: true, ..Default::default() })]);
}

#[gpui::test]
fn a_press_on_one_dialog_does_not_complete_on_its_replacement(cx: &mut TestAppContext) {
    let (_, p, cx) = page(cx);
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog(24, DialogKind::Confirm))));
    cx.run_until_parked();
    cx.executor().advance_clock(ACTIVATION_DELAY);
    let ok = cx.debug_bounds("dialog-ok").expect("OK is drawn").center();
    cx.simulate_mouse_down(ok, gpui::MouseButton::Left, Modifiers::none());
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog(25, DialogKind::Confirm))));
    cx.run_until_parked();
    cx.executor().advance_clock(ACTIVATION_DELAY);
    cx.simulate_mouse_up(ok, gpui::MouseButton::Left, Modifiers::none());
    assert!(answered(&p, cx).is_empty());
}

#[gpui::test]
fn a_held_enter_does_not_answer(cx: &mut TestAppContext) {
    let (b, p, cx) = page(cx);
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog(23, DialogKind::Confirm))));
    cx.run_until_parked();
    cx.executor().advance_clock(ACTIVATION_DELAY);
    cx.simulate_event(gpui::KeyDownEvent { keystroke: Keystroke::parse("enter").unwrap(), is_held: true, prefer_character_input: false });
    assert!(answered(&p, cx).is_empty());
    b.read_with(cx, |b, _| assert!(b.active_dialog().is_some()));
    cx.simulate_keystrokes("enter");
    assert_eq!(answered(&p, cx), [(23, DialogAnswer { ok: true, ..Default::default() })]);
}

#[gpui::test]
fn ask_closed_removes_only_its_own_sheet(cx: &mut TestAppContext) {
    let (b, p, cx) = page(cx);
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog(7, DialogKind::Unresponsive))));
    cx.run_until_parked();
    p.update(cx, |_, cx| cx.emit(PageEvent::AskClosed(8)));
    cx.run_until_parked();
    b.read_with(cx, |b, _| assert!(b.active_dialog().is_some()));
    p.update(cx, |_, cx| cx.emit(PageEvent::AskClosed(7)));
    cx.run_until_parked();
    b.read_with(cx, |b, _| assert!(b.active_dialog().is_none()));
    assert!(answered(&p, cx).is_empty());
}

#[gpui::test]
fn unresponsive_escape_waits(cx: &mut TestAppContext) {
    let (_, p, cx) = page(cx);
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog(9, DialogKind::Unresponsive))));
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    assert_eq!(answered(&p, cx), [(9, DialogAnswer::default())]);
}

#[gpui::test]
fn js_confirm_still_names_the_site(cx: &mut TestAppContext) {
    let (b, p, cx) = page(cx);
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog(10, DialogKind::Confirm))));
    cx.run_until_parked();
    b.read_with(cx, |b, _| assert!(b.active_dialog().unwrap().dialog.title.is_empty()));
    assert_eq!(wording(&dialog(10, DialogKind::Confirm), "https://a.example/").0, "a.example says");
}

#[test]
fn a_js_dialog_shows_the_title_chrome_sent_not_the_tabs_host() {
    for kind in [DialogKind::Alert, DialogKind::Confirm, DialogKind::Prompt] {
        let mut d = dialog(12, kind);
        d.title = "evil.example says".into();
        assert_eq!(wording(&d, "https://a.example/").0, "evil.example says");
    }
    let d = dialog(13, DialogKind::Alert);
    assert_eq!(wording(&d, "https://user@a.example:8443/").0, "a.example:8443 says");
}

#[gpui::test]
fn closing_a_peek_leaves_the_keyboard_on_the_base_tabs_dialog(cx: &mut TestAppContext) {
    let (b, p, cx) = page(cx);
    p.update(cx, |_, cx| {
        cx.emit(PageEvent::OpenRequested { url: "https://peek.example".into(), from_user_gesture: true, background: false })
    });
    cx.run_until_parked();
    let peek = b.read_with(cx, |b, _| b.peek.as_ref().unwrap().page.clone());
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog(11, DialogKind::Auth))));
    cx.run_until_parked();
    peek.update(cx, |_, cx| cx.emit(PageEvent::Closed));
    cx.run_until_parked();
    cx.update(|window, cx| {
        let b = b.read(cx);
        assert!(b.peek.is_none());
        assert!(b.input_focus.is_focused(window), "the dialog keeps the keyboard");
        assert!(!p.read(cx).focus.is_focused(window), "not the page under it");
    });
}
