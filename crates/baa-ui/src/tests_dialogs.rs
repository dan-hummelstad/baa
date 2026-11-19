//! Chrome's dialogs as sheets: HTTP auth, external protocol, JS dialogs.

use baa_model::Store;
use baa_web::{Dialog, DialogAnswer, DialogKind, PageEvent};
use gpui::{Entity, InputHandler, Keystroke, Modifiers, TestAppContext, VisualTestContext};

use crate::{lil_baa::LilBaa, mock::{MockHost, MockPage}, prompts::{wording, ACTIVATION_DELAY}, Browser};

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

#[test]
fn a_js_dialog_shows_the_title_chrome_sent_not_the_tabs_host() {
    for kind in [DialogKind::Alert, DialogKind::Confirm, DialogKind::Prompt] {
        let mut d = dialog(12, kind);
        d.title = "evil.example says".into();
        assert_eq!(wording(&d, "https://a.example/").0, "evil.example says");
    }
    let d = dialog(13, DialogKind::Alert);
    assert_eq!(wording(&d, "https://user@a.example:8443/").0, "a.example:8443 says");
    assert_eq!(wording(&dialog(10, DialogKind::Confirm), "https://a.example/").0, "a.example says", "a confirm with no title too");
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

/// A prompt's text, its fields and its warnings stay readable in the light, dark and ASCII looks.
#[test]
fn prompts_stay_legible_in_every_look() {
    use crate::theme::surface::{BG, INK, MUTED, RAISED, WARN};
    // WCAG contrast of two RRGGBBAA colours (alpha ignored: the surfaces are near opaque).
    let contrast = |a: u32, b: u32| {
        let lum = |c: u32| {
            let c = gpui::rgba(c);
            let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
            0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b) + 0.05
        };
        lum(a).max(lum(b)) / lum(a).min(lum(b))
    };
    for look in 0..3 {
        for under in [BG, RAISED] {
            assert!(contrast(INK[look], under[look]) > 7., "text, look {look}");
            assert!(contrast(MUTED[look], under[look]) > 3., "muted text, look {look}");
            assert!(contrast(WARN[look], under[look]) > 4.5, "a warning, look {look}");
        }
    }
}

fn ime(b: &Entity<Browser<MockHost>>) -> impl InputHandler {
    crate::input::handler(b, Browser::input_field, Browser::on_input_key)
}

#[gpui::test]
fn enter_while_composing_a_prompt_answer_commits_it_and_does_not_answer(cx: &mut TestAppContext) {
    let (b, p, cx) = page(cx);
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog(30, DialogKind::Prompt))));
    cx.run_until_parked();
    let mut ime = ime(&b);
    cx.update(|window, cx| {
        assert!(window.is_action_available(&crate::actions::Undo, cx), "a prompt's answer field has Undo");
        ime.replace_and_mark_text_in_range(None, "にほん", Some(3..3), window, cx);
        assert_eq!(ime.marked_text_range(window, cx), Some(0..3));
    });
    cx.simulate_keystrokes("enter");
    cx.update(|window, cx| {
        let b = b.read(cx);
        assert_eq!(b.active_dialog().unwrap().field.text, "にほん");
        assert!(b.input_focus.is_focused(window), "the prompt keeps the keyboard");
        assert_eq!(ime.marked_text_range(window, cx), None);
    });
    cx.simulate_keystrokes("enter");
    assert!(answered(&p, cx).is_empty(), "OK still waits out the activation delay");
    cx.executor().advance_clock(ACTIVATION_DELAY);
    cx.update(|window, cx| ime.replace_and_mark_text_in_range(None, "ご", None, window, cx));
    cx.simulate_keystrokes("enter");
    assert!(answered(&p, cx).is_empty());
    cx.simulate_keystrokes("enter");
    let want = DialogAnswer { ok: true, text: "にほんご".into(), ..Default::default() };
    assert_eq!(answered(&p, cx), [(30, want)]);
}

#[gpui::test]
fn the_input_method_reaches_no_risky_dialog_and_no_password(cx: &mut TestAppContext) {
    let (b, p, cx) = page(cx);
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog(31, DialogKind::ExternalProtocol))));
    cx.run_until_parked();
    cx.executor().advance_clock(ACTIVATION_DELAY);
    let mut ime = ime(&b);
    cx.update(|window, cx| {
        assert!(!window.is_action_available(&crate::actions::Undo, cx), "a field-less prompt gets no Undo");
        assert!(!ime.accepts_text_input(window, cx));
        ime.replace_and_mark_text_in_range(None, "x", None, window, cx);
        ime.replace_text_in_range(None, "\n", window, cx);
    });
    assert!(answered(&p, cx).is_empty());
    cx.simulate_keystrokes("escape");

    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog(32, DialogKind::Auth))));
    cx.run_until_parked();
    cx.simulate_keystrokes("tab");
    cx.simulate_input("pw");
    cx.update(|window, cx| {
        assert!(!ime.accepts_text_input(window, cx), "a password is typed without the input method");
        ime.replace_and_mark_text_in_range(None, "¨", None, window, cx);
        assert_eq!(ime.marked_text_range(window, cx), None, "never shown composing in the clear");
        ime.replace_text_in_range(None, "ö", window, cx);
        assert_eq!(ime.text_for_range(0..3, &mut None, window, cx), None);
        assert_eq!(b.read(cx).active_dialog().unwrap().password.text, "pwö");
    });
}

fn app_link(id: u64) -> Dialog {
    Dialog {
        title: "Open Hachiman?".into(),
        message: "https://a.example wants to open this application.".into(),
        ok_label: "Open Hachiman".into(),
        cancel_label: "Cancel".into(),
        ..dialog(id, DialogKind::ExternalProtocol)
    }
}

fn lil_baa(cx: &mut TestAppContext) -> (Entity<LilBaa<MockHost>>, Entity<MockPage>, gpui::AnyWindowHandle) {
    cx.update(crate::init);
    let (b, vcx) = cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx));
    b.update(vcx, |b, cx| b.open_external("https://a.example", cx));
    vcx.run_until_parked();
    let lil = cx.windows().into_iter().find_map(|w| w.downcast::<LilBaa<MockHost>>()).expect("a lil-baa window");
    let page = lil.update(cx, |lil, _, _| lil.page.clone().unwrap()).unwrap();
    (lil.entity(cx).unwrap(), page, lil.into())
}

fn ask_focused(lil: &Entity<LilBaa<MockHost>>, p: &Entity<MockPage>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        let on_ask = lil.read(cx).ask_focus.is_focused(window);
        assert!(!(on_ask && p.read(cx).focus.is_focused(window)));
        on_ask
    })
}

#[gpui::test]
fn lil_baa_asks_to_open_an_app_with_every_safeguard(cx: &mut TestAppContext) {
    let (lil, p, window) = lil_baa(cx);
    let cx = &mut VisualTestContext::from_window(window, cx);
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(app_link(40))));
    cx.run_until_parked();
    let shown = cx.update(|_, cx| lil.read(cx).asks.dialog.as_ref().map(|d| wording(&d.dialog, "https://a.example/")));
    let (title, message, ok, _) = shown.expect("drawn, not answered");
    assert_eq!((title.as_str(), message.as_str(), ok.as_str()), ("Open Hachiman?", "https://a.example wants to open this application.", "Open Hachiman"));
    assert!(cx.debug_bounds("page-dialog").is_some());
    assert!(ask_focused(&lil, &p, cx), "the prompt has the keyboard");
    p.update(cx, |_, cx| cx.emit(PageEvent::Focused));
    cx.run_until_parked();
    assert!(ask_focused(&lil, &p, cx), "a click into the page does not take it");

    let ok = cx.debug_bounds("dialog-ok").expect("Open is drawn").center();
    cx.simulate_click(ok, Modifiers::none());
    assert!(answered(&p, cx).is_empty(), "a click in the first 500 ms was aimed at something else");
    cx.executor().advance_clock(ACTIVATION_DELAY);
    cx.simulate_event(gpui::KeyDownEvent { keystroke: Keystroke::parse("escape").unwrap(), is_held: true, prefer_character_input: false });
    cx.simulate_keystrokes("enter");
    assert!(answered(&p, cx).is_empty(), "Enter never opens an app, a held Escape never cancels");
    cx.simulate_click(ok, Modifiers::none());
    assert_eq!(answered(&p, cx), [(40, DialogAnswer { ok: true, ..Default::default() })]);
    assert!(!ask_focused(&lil, &p, cx), "the page has the keyboard back");

    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(app_link(41))));
    cx.run_until_parked();
    let cancel = cx.debug_bounds("dialog-cancel").expect("Cancel is drawn").center();
    cx.simulate_click(cancel, Modifiers::none());
    assert_eq!(answered(&p, cx)[1], (41, DialogAnswer::default()));
}

#[gpui::test]
fn lil_baa_shows_an_alert_and_a_permission_instead_of_answering_them(cx: &mut TestAppContext) {
    let (_, p, window) = lil_baa(cx);
    let cx = &mut VisualTestContext::from_window(window, cx);
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog(42, DialogKind::Alert))));
    cx.run_until_parked();
    assert!(answered(&p, cx).is_empty());
    assert!(cx.debug_bounds("dialog-ok").is_some());
    cx.executor().advance_clock(ACTIVATION_DELAY);
    cx.simulate_keystrokes("enter");
    assert_eq!(answered(&p, cx), [(42, DialogAnswer { ok: true, ..Default::default() })]);

    p.update(cx, |_, cx| {
        cx.emit(PageEvent::PermissionRequested { id: 7, origin: "https://a.example".into(), kinds: vec![baa_web::Permission::Camera] })
    });
    cx.run_until_parked();
    let allow = cx.debug_bounds("perm-allow").expect("the banner is drawn").center();
    assert!(p.read_with(cx, |p, _| p.answers.borrow().is_empty()), "not denied for want of room");
    cx.simulate_click(allow, Modifiers::none());
    assert!(p.read_with(cx, |p, _| p.answers.borrow().is_empty()), "Allow waits out the activation delay");
    cx.executor().advance_clock(ACTIVATION_DELAY);
    cx.simulate_click(allow, Modifiers::none());
    p.read_with(cx, |p, _| assert_eq!(*p.answers.borrow(), ["permission 7 true"]));
}

#[gpui::test]
fn a_sign_in_sheet_fits_lil_baa_at_its_smallest(cx: &mut TestAppContext) {
    let (_, p, window) = lil_baa(cx);
    let cx = &mut VisualTestContext::from_window(window, cx);
    let min = cx.update(|_, cx| crate::lil_baa::window_options(cx).window_min_size.expect("a minimum size"));
    cx.simulate_resize(min);
    let mut d = dialog(43, DialogKind::Auth);
    d.message = "https://a.example requires a username and password. ".repeat(20);
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(d)));
    cx.run_until_parked();
    let window = cx.update(|window, _| window.viewport_size());
    for part in ["page-dialog", "dialog-ok", "dialog-cancel"] {
        let b = cx.debug_bounds(part).unwrap_or_else(|| panic!("{part} is drawn"));
        assert!(b.bottom() <= window.height && b.right() <= window.width && b.top() >= gpui::px(0.), "{part} at {b:?} in {window:?}");
    }
}

#[gpui::test]
fn an_app_link_in_a_peek_asks_on_the_card_and_the_empty_card_goes(cx: &mut TestAppContext) {
    let (b, p, cx) = page(cx);
    let peek = |url: &str, cx: &mut VisualTestContext| {
        p.update(cx, |_, cx| cx.emit(PageEvent::OpenRequested { url: url.into(), from_user_gesture: true, background: false }));
        cx.run_until_parked();
        b.read_with(cx, |b, _| b.peek.as_ref().unwrap().page.clone())
    };
    let link = peek("app.hachiman://open", cx);
    link.update(cx, |_, cx| cx.emit(PageEvent::Dialog(app_link(50))));
    cx.run_until_parked();
    assert!(answered(&link, cx).is_empty(), "not cancelled for being in a Peek");
    cx.update(|window, cx| assert!(b.read(cx).input_focus.is_focused(window)));
    let (card, sheet) = (cx.debug_bounds("peek-card").unwrap(), cx.debug_bounds("page-dialog").expect("drawn"));
    assert!(card.contains(&sheet.center()) && card.left() < sheet.left() && sheet.right() < card.right(), "on the Peek card, not under its scrim");
    cx.executor().advance_clock(ACTIVATION_DELAY);
    cx.simulate_keystrokes("enter");
    assert!(answered(&link, cx).is_empty());
    cx.simulate_keystrokes("escape");
    assert_eq!(answered(&link, cx), [(50, DialogAnswer::default())]);
    b.read_with(cx, |b, _| assert!(b.peek.is_none(), "no empty card stays"));

    let link = peek("zoommtg://meet", cx);
    link.update(cx, |_, cx| cx.emit(PageEvent::Dialog(app_link(51))));
    cx.run_until_parked();
    cx.executor().advance_clock(ACTIVATION_DELAY);
    let ok = cx.debug_bounds("dialog-ok").unwrap().center();
    cx.simulate_click(ok, Modifiers::none());
    assert_eq!(answered(&link, cx), [(51, DialogAnswer { ok: true, ..Default::default() })]);
    b.read_with(cx, |b, _| assert!(b.peek.is_none()));

    let site = peek("https://zoom.example/j/1", cx);
    site.update(cx, |_, cx| cx.emit(PageEvent::Dialog(app_link(52))));
    cx.run_until_parked();
    cx.executor().advance_clock(ACTIVATION_DELAY);
    cx.simulate_click(ok, Modifiers::none());
    assert_eq!(answered(&site, cx).len(), 1);
    b.read_with(cx, |b, _| assert!(b.peek.is_some(), "a Peek of a site stays"));
    assert!(answered(&p, cx).is_empty(), "the page under the Peek was asked nothing");
}

#[gpui::test]
fn a_dialog_moved_out_of_a_peek_waits_out_the_delay_again(cx: &mut TestAppContext) {
    let (b, p, cx) = page(cx);
    p.update(cx, |_, cx| cx.emit(PageEvent::OpenRequested { url: "https://peek.example".into(), from_user_gesture: true, background: false }));
    cx.run_until_parked();
    let peek = b.read_with(cx, |b, _| b.peek.as_ref().unwrap().page.clone());
    peek.update(cx, |_, cx| cx.emit(PageEvent::Dialog(dialog(60, DialogKind::Confirm))));
    cx.run_until_parked();
    cx.executor().advance_clock(std::time::Duration::from_millis(600));
    b.update_in(cx, |b, window, cx| b.expand_peek(window, cx));
    cx.run_until_parked();
    let ok = cx.debug_bounds("dialog-ok").expect("the dialog came along").center();
    cx.simulate_click(ok, Modifiers::none());
    assert!(answered(&peek, cx).is_empty(), "a click aimed at the Peek's controls");
    cx.executor().advance_clock(ACTIVATION_DELAY);
    cx.simulate_click(ok, Modifiers::none());
    assert_eq!(answered(&peek, cx), [(60, DialogAnswer { ok: true, ..Default::default() })]);
}

#[gpui::test]
fn a_dialog_moved_out_of_lil_baa_waits_out_the_delay_again_over_an_open_peek(cx: &mut TestAppContext) {
    let (lil, p, window) = lil_baa(cx);
    let main = cx.windows().into_iter().find_map(|w| w.downcast::<Browser<MockHost>>()).expect("the browser window");
    let b = main.entity(cx).unwrap();
    let main = &mut VisualTestContext::from_window(main.into(), cx);
    b.update_in(main, |b, window, cx| {
        let space = b.session.active_space().unwrap();
        b.open_peek(space, "https://peek.example", window, cx);
    });
    main.run_until_parked();
    p.update(main, |_, cx| cx.emit(PageEvent::Dialog(dialog(61, DialogKind::Confirm))));
    main.run_until_parked();
    main.executor().advance_clock(std::time::Duration::from_millis(600));
    window.update(main, |_, window, cx| lil.update(cx, |lil, cx| lil.open_in_browser(window, cx))).unwrap();
    main.run_until_parked();
    main.update(|window, cx| {
        assert!(b.read(cx).peek.is_none(), "the Peek would cover the moved dialog");
        assert!(b.read(cx).input_focus.is_focused(window), "the dialog has the keyboard");
    });
    let ok = main.debug_bounds("dialog-ok").expect("the dialog came along").center();
    main.simulate_click(ok, Modifiers::none());
    assert!(answered(&p, main).is_empty(), "a click aimed at the Open in button");
    main.executor().advance_clock(ACTIVATION_DELAY);
    main.simulate_click(ok, Modifiers::none());
    assert_eq!(answered(&p, main), [(61, DialogAnswer { ok: true, ..Default::default() })]);
}
