//! Tests of compose, on MockHost and an in-memory Mail.

use std::time::Duration;

use baa_mail::{testing, DraftId, Filter, Flags, Mail, MessageId, View};
use baa_model::SpaceId;
use baa_web::MenuItem;
use gpui::VisualTestContext;
use serde_json::json;

use super::Field;
use crate::{actions::NewSpace, mock::MockPage, tests_mail::*};

fn seed_from_ola(mail: &Mail, inbox: baa_mail::FolderId) -> MessageId {
    let raw = testing::message("Ola <ola@example.com>", "dan@example.com", "Lunch on Friday", 1_700_000_000, "", "Shall we eat at noon?\r\n");
    mail.seed_message(inbox, "1", &raw, Flags::default(), true)
}

fn count(mail: &Mail, space: baa_mail::MailSpace) -> usize {
    mail.count(&View { space, folder: None, account: None, filter: Filter::All, search: String::new() })
}

fn front(b: &B, cx: &mut VisualTestContext) -> Option<DraftId> {
    b.read_with(cx, |b, _| b.mail.as_ref().unwrap().state.front)
}

fn editor(b: &B, cx: &mut VisualTestContext) -> gpui::Entity<MockPage> {
    b.read_with(cx, |b, _| b.mail.as_ref().unwrap().front_draft().unwrap().page.clone().unwrap())
}

/// Answers the body read the composer sent to the editor.
fn answer_body(b: &B, cx: &mut VisualTestContext, html: &str, text: &str) {
    let page = editor(b, cx);
    let (script, done) = page.read_with(cx, |p, _| p.evals.borrow_mut().pop().expect("the body was read"));
    assert!(script.contains("innerHTML") && script.contains("innerText"));
    let body = json!({ "html": html, "text": text });
    cx.update(|_, cx| done(Ok(body), cx));
    cx.run_until_parked();
}

fn new_message(b: &B, cx: &mut VisualTestContext) -> DraftId {
    b.update_in(cx, |b, window, cx| b.new_message(window, cx));
    cx.run_until_parked();
    front(b, cx).expect("a draft is in front")
}

#[gpui::test]
fn a_reply_is_a_new_tab_with_the_quoted_text_and_the_right_recipients(cx: &mut gpui::TestAppContext) {
    cx.executor().allow_parking();
    let (b, cx, mail, folders) = with_account(cx);
    let id = seed_from_ola(&mail, folders[0]);
    b.update_in(cx, |b, window, cx| b.open_draft(mail.reply(id, false).unwrap(), window, cx));
    cx.run_until_parked();

    assert!(drawn(cx, "mail-composer"));
    b.read_with(cx, |b, cx| {
        let ui = b.mail.as_ref().unwrap();
        let c = ui.front_draft().expect("the reply is in front");
        assert_eq!(ui.state.drafts, [c.id]);
        let to: Vec<_> = c.chips(Field::To).iter().map(|a| a.email.as_str()).collect();
        assert_eq!(to, ["ola@example.com"]);
        assert_eq!(c.subject.text, "Re: Lunch on Friday");
        let set = c.page.as_ref().unwrap().read(cx).set_documents.borrow();
        assert_eq!(set.len(), 1);
        assert!(set[0].contains("Shall we eat at noon?"), "the quote is in the editor: {}", set[0]);
        assert_eq!(*b.host.documents.borrow().last().map(|(_, k)| k).unwrap(), baa_web::DocumentKind::Editor);
    });
}

#[gpui::test]
fn autosave_writes_the_draft(cx: &mut gpui::TestAppContext) {
    cx.executor().allow_parking();
    let (b, cx, mail, _) = with_account(cx);
    let id = new_message(&b, cx);
    b.update_in(cx, |b, window, cx| b.focus_mail_field(Field::To, window, cx));
    cx.simulate_input("ola@example.com");
    b.read_with(cx, |b, _| assert!(b.mail.as_ref().unwrap().front_draft().unwrap().dirty));

    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    answer_body(&b, cx, "<p>Hi</p>", "Hi");

    let saved = mail.draft(id).expect("the draft is kept");
    assert_eq!(saved.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ola@example.com"]);
    assert_eq!((saved.html.as_str(), saved.text.as_str()), ("<p>Hi</p>", "Hi"));
    b.read_with(cx, |b, _| assert!(!b.mail.as_ref().unwrap().front_draft().unwrap().dirty, "saved: the dot goes"));
}

#[gpui::test]
fn send_calls_mail_send_and_closes_the_tab(cx: &mut gpui::TestAppContext) {
    cx.executor().allow_parking();
    let (b, cx, _mail, _) = with_account(cx);
    new_message(&b, cx);
    b.update_in(cx, |b, window, cx| b.send_front_draft(window, cx));
    b.read_with(cx, |b, _| {
        assert_eq!(b.mail.as_ref().unwrap().drafts.len(), 1, "no recipient: it stays");
        assert!(b.mail.as_ref().unwrap().front_draft().unwrap().error.is_some());
    });

    b.update_in(cx, |b, window, cx| b.focus_mail_field(Field::To, window, cx));
    cx.simulate_input("ola@example.com,");
    b.update_in(cx, |b, window, cx| b.send_front_draft(window, cx));
    answer_body(&b, cx, "<p>Hello</p>", "Hello");

    b.read_with(cx, |b, _| {
        let ui = b.mail.as_ref().unwrap();
        assert!(ui.drafts.is_empty() && ui.state.drafts.is_empty());
        assert_eq!(ui.state.front, None);
        assert!(b.toast.as_ref().is_some_and(|t| !t.contains("recipient")));
    });
    assert!(!drawn(cx, "mail-composer"));
}

#[gpui::test]
fn closing_a_changed_draft_asks_first(cx: &mut gpui::TestAppContext) {
    cx.executor().allow_parking();
    let (b, cx, _mail, _) = with_account(cx);
    let id = new_message(&b, cx);
    b.update(cx, |b, cx| {
        b.request_close_draft(id, cx);
        assert!(b.mail.as_ref().unwrap().drafts.is_empty(), "nothing changed: it closes at once");
    });

    let id = new_message(&b, cx);
    b.update_in(cx, |b, window, cx| b.focus_mail_field(Field::Subject, window, cx));
    cx.simulate_input("Plans");
    assert!(!drawn(cx, "mail-close-ask"));
    b.update(cx, |b, cx| b.request_close_draft(id, cx));
    assert!(drawn(cx, "mail-close-ask"));
    assert_eq!(front(&b, cx), Some(id), "the tab stays until answered");
    b.read_with(cx, |b, _| assert_eq!(b.mail.as_ref().unwrap().drafts.len(), 1));
}

#[gpui::test]
fn the_from_field_offers_only_the_spaces_accounts(cx: &mut gpui::TestAppContext) {
    cx.executor().allow_parking();
    let (b, cx, mail, _) = with_account(cx);
    let first = b.read_with(cx, |b, _| b.session.active_space().unwrap());
    cx.dispatch_action(NewSpace);
    cx.simulate_keystrokes("escape");
    let other = b.read_with(cx, |b, _| {
        let id: SpaceId = b.session.spaces().iter().map(|s| s.id).find(|s| *s != first).unwrap();
        b.mail_of(id).unwrap().1
    });
    mail.seed_account(&testing::account(2, None, "everywhere@example.com"));
    mail.seed_account(&testing::account(3, Some(other), "elsewhere@example.com"));
    b.update_in(cx, |b, window, cx| {
        b.activate_space(first, window, cx);
        b.show_mail(true, window, cx);
    });
    let id = new_message(&b, cx);

    b.update_in(cx, |b, window, cx| b.open_from_menu(id, window, cx));
    cx.run_until_parked();
    b.read_with(cx, |b, _| {
        let menus = b.host.menus.borrow();
        let labels: Vec<String> = menus
            .last()
            .unwrap()
            .iter()
            .filter_map(|i| match i {
                MenuItem::Action { label, .. } => Some(label.clone()),
                _ => None,
            })
            .collect();
        assert!(labels.iter().any(|l| l.contains("dan@example.com")), "{labels:?}");
        assert!(labels.iter().any(|l| l.contains("everywhere@example.com")), "{labels:?}");
        assert!(!labels.iter().any(|l| l.contains("elsewhere@example.com")), "{labels:?}");
    });
}

#[gpui::test]
fn typing_backspace_in_to_deletes_a_character_not_a_message(cx: &mut gpui::TestAppContext) {
    cx.executor().allow_parking();
    let (b, cx, mail, folders) = with_account(cx);
    let space = mail_space(&b, cx);
    seed_from_ola(&mail, folders[0]);
    let view = View { space, folder: None, account: None, filter: Filter::All, search: String::new() };
    let thread = mail.rows(&view, 0..10)[0].thread;
    b.update(cx, |b, _| b.mail.as_mut().unwrap().state.selected = Some(thread));
    new_message(&b, cx);
    b.update_in(cx, |b, window, cx| b.focus_mail_field(Field::To, window, cx));

    cx.simulate_keystrokes("a b backspace");
    b.read_with(cx, |b, _| {
        let c = b.mail.as_ref().unwrap().front_draft().unwrap();
        assert_eq!(c.to.text, "a");
        assert!(c.chips(Field::To).is_empty());
    });
    assert_eq!(count(&mail, space), 1, "the message is still there");
}
