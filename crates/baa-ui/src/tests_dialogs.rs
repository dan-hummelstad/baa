//! Chrome's dialogs as sheets: HTTP auth, external protocol, JS dialogs.

use baa_model::Store;
use baa_web::{Dialog, DialogAnswer, DialogKind, PageEvent};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};

use crate::{mock::{MockHost, MockPage}, Browser};

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
    cx.simulate_keystrokes("enter");
    assert_eq!(answered(&p, cx), [(6, DialogAnswer { ok: true, checked: true, ..Default::default() })]);
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
    // The title is drawn, not stored; the sheet being up with no Chrome title is the check.
    b.read_with(cx, |b, _| assert!(b.active_dialog().unwrap().dialog.title.is_empty()));
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
