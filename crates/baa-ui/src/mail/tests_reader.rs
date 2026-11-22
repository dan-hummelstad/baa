//! The reading pane and the command row (WP-I), on MockHost and an in-memory Mail.

use baa_mail::{testing::message, Action, Filter, Flags, FolderId, Mail, MessageId, ThreadId, View};
use baa_web::{DocumentKind, PageEvent, WebHost};
use gpui::{Entity, VisualTestContext};

use crate::{mock::MockPage, tests_mail::*};

const HTML: &str = "Content-Type: text/html; charset=utf-8\r\n";

fn deliver(mail: &Mail, inbox: FolderId, uid: &str, from: &str, body: &str, with_body: bool) -> MessageId {
    let raw = message(from, "dan@example.com", "Plans", crate::mail::now_secs(), HTML, body);
    mail.seed_message(inbox, uid, &raw, Flags::default(), with_body)
}

fn thread_of(mail: &Mail, b: &B, cx: &mut VisualTestContext) -> ThreadId {
    let view = b.read_with(cx, |b, _| b.mail.as_ref().unwrap().view().unwrap());
    mail.rows(&view, 0..1)[0].thread
}

fn select(b: &B, cx: &mut VisualTestContext, thread: ThreadId) {
    b.update_in(cx, |b, window, cx| b.select_thread(thread, window, cx));
    cx.run_until_parked();
}

fn page(b: &B, cx: &mut VisualTestContext) -> Option<Entity<MockPage>> {
    b.read_with(cx, |b, _| b.mail.as_ref().unwrap().reader.as_ref().and_then(|r| r.page.clone()))
}

fn documents(b: &B, cx: &mut VisualTestContext) -> Vec<DocumentKind> {
    b.read_with(cx, |b, _| b.host.documents.borrow().iter().map(|(_, k)| *k).collect())
}

#[gpui::test]
fn selecting_a_row_puts_its_body_in_a_message_document_and_reads_it(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    deliver(&mail, folders[0], "1", "Ola <ola@example.com>", "<p>See you at nine</p>", true);
    let thread = thread_of(&mail, &b, cx);
    let view = b.read_with(cx, |b, _| b.mail.as_ref().unwrap().view().unwrap());
    assert_eq!(mail.count(&View { filter: Filter::Unread, ..view.clone() }), 1);

    select(&b, cx, thread);
    assert_eq!(documents(&b, cx), [DocumentKind::Message { remote_images: false }]);
    let set = page(&b, cx).unwrap().read_with(cx, |p, _| p.set_documents.borrow().clone());
    assert_eq!(set.len(), 1);
    assert!(set[0].contains("See you at nine"));
    assert!(drawn(cx, "mail-sender-address"), "the real address is always on screen");
    assert_eq!(mail.count(&View { filter: Filter::Unread, ..view }), 0, "opening it read it");
    assert_eq!(b.read_with(cx, |b, _| b.mail.as_ref().unwrap().state.selected), Some(thread));
    let space = mail_space(&b, cx);
    assert_eq!(mail.view_state(space).selected, Some(thread), "kept for the next launch");
}

#[gpui::test]
fn remote_images_stay_off_until_the_banner_makes_a_new_document(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    deliver(&mail, folders[0], "1", "news@shop.example", "<p>Sale</p><img src=\"https://shop.example/pixel.gif\">", true);
    let thread = thread_of(&mail, &b, cx);
    select(&b, cx, thread);
    let first = page(&b, cx).unwrap();
    assert!(drawn(cx, "mail-images-banner"));

    b.update_in(cx, |b, window, cx| b.load_images(false, window, cx));
    cx.run_until_parked();
    assert_eq!(documents(&b, cx), [DocumentKind::Message { remote_images: false }, DocumentKind::Message { remote_images: true }]);
    assert_ne!(page(&b, cx).unwrap(), first, "a live document's policy is never changed");
    assert!(!drawn(cx, "mail-images-banner"));
    assert!(!mail.trusts_images("news@shop.example"), "this message only");

    b.update_in(cx, |b, window, cx| b.load_images(true, window, cx));
    assert!(mail.trusts_images("news@shop.example"));
    deliver(&mail, folders[0], "2", "news@shop.example", "<p>More</p><img src=\"https://shop.example/b.gif\">", true);
    cx.run_until_parked();
    let thread = thread_of(&mail, &b, cx);
    select(&b, cx, thread);
    assert_eq!(documents(&b, cx).last(), Some(&DocumentKind::Message { remote_images: true }), "always for this sender");
}

#[gpui::test]
fn with_remote_images_set_to_never_there_is_no_banner(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    b.update(cx, |b, _| b.settings.mail.remote_images = baa_model::RemoteImages::Never);
    mail.trust_images("news@shop.example");
    deliver(&mail, folders[0], "1", "news@shop.example", "<img src=\"https://shop.example/pixel.gif\">", true);
    let thread = thread_of(&mail, &b, cx);
    select(&b, cx, thread);
    assert!(!drawn(cx, "mail-images-banner"));
    assert_eq!(documents(&b, cx), [DocumentKind::Message { remote_images: false }], "Never wins over a trusted sender");
}

#[gpui::test]
fn a_link_in_a_message_is_adopted_and_never_loaded(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    deliver(&mail, folders[0], "1", "ola@example.com", "<a href=\"https://evil.example/x\">x</a>", true);
    let thread = thread_of(&mail, &b, cx);
    select(&b, cx, thread);
    let doc = page(&b, cx).unwrap();
    doc.update(cx, |_, cx| cx.emit(PageEvent::TargetUrl("https://evil.example/x".into())));
    assert!(drawn(cx, "mail-link-target"), "where the link goes, shown under the pane");

    doc.update(cx, |_, cx| cx.emit(PageEvent::OpenRequested { url: "https://evil.example/x".into(), from_user_gesture: true, background: false }));
    cx.run_until_parked();
    b.read_with(cx, |b, cx| {
        assert_eq!(*b.host.supplied.borrow(), ["https://evil.example/x"], "Chrome's own tab is adopted");
        assert!(!b.mail_in_front(), "the tab shows, so mail leaves");
        for (page, _) in b.pages.values() {
            assert!(page.read(cx).loads.borrow().is_empty(), "no load_url with a URL from mail");
        }
    });

    doc.update(cx, |_, cx| cx.emit(PageEvent::OpenRequested { url: "https://evil.example/y".into(), from_user_gesture: true, background: true }));
    cx.run_until_parked();
    b.read_with(cx, |b, _| {
        let space = b.session.active_space().unwrap();
        assert!(b.session.space_tabs(space).any(|t| t.url == "https://evil.example/y"), "a tab for later");
        assert_eq!(b.host.supplied.borrow().len(), 1, "made, and adopted, only once it is shown");
    });
}

#[gpui::test]
fn a_body_not_here_yet_shows_loading_and_fills_in_when_it_arrives(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    let raw = message("ola@example.com", "dan@example.com", "Plans", crate::mail::now_secs(), HTML, "<p>Late</p>");
    let id = mail.seed_message(folders[0], "1", &raw, Flags::default(), false);
    let thread = thread_of(&mail, &b, cx);
    select(&b, cx, thread);
    assert!(drawn(cx, "mail-loading") && page(&b, cx).is_none());

    mail.seed_body(id, &raw);
    cx.run_until_parked();
    assert!(!drawn(cx, "mail-loading"));
    let set = page(&b, cx).expect("made when the body came").read_with(cx, |p, _| p.set_documents.borrow().clone());
    assert!(set[0].contains("Late"));
}

#[gpui::test]
fn the_commands_reach_mail(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    deliver(&mail, folders[0], "1", "ola@example.com", "<p>one</p>", true);
    let thread = thread_of(&mail, &b, cx);
    select(&b, cx, thread);
    let act = |b: &B, cx: &mut VisualTestContext, action| b.update(cx, |b, cx| b.act_on_selected(action, cx));
    act(&b, cx, Action::Flag(true));
    assert!(mail.conversation(thread)[0].flagged);
    act(&b, cx, Action::Read(false));
    assert!(mail.conversation(thread)[0].unread);
    act(&b, cx, Action::Archive);
    assert!(mail.conversation(thread).is_empty(), "it left the inbox");
    assert!(b.read_with(cx, |b, _| b.mail.as_ref().unwrap().reader.is_none()));

    deliver(&mail, folders[0], "2", "ola@example.com", "<p>two</p>", true);
    cx.run_until_parked();
    let thread = thread_of(&mail, &b, cx);
    select(&b, cx, thread);
    cx.simulate_keystrokes("backspace");
    let view = b.read_with(cx, |b, _| b.mail.as_ref().unwrap().view().unwrap());
    assert_eq!(mail.count(&view), 0, "⌫ outside a field deletes the conversation");

    deliver(&mail, folders[0], "3", "ola@example.com", "<p>three</p>", true);
    cx.run_until_parked();
    let thread = thread_of(&mail, &b, cx);
    select(&b, cx, thread);
    act(&b, cx, Action::Move(folders[3]));
    assert!(mail.conversation(thread).is_empty());
}

#[gpui::test]
fn reply_and_forward_open_a_draft_of_the_shown_message(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    deliver(&mail, folders[0], "1", "Ola <ola@example.com>", "<p>one</p>", true);
    let thread = thread_of(&mail, &b, cx);
    select(&b, cx, thread);
    cx.simulate_keystrokes("cmd-r");
    cx.run_until_parked();
    b.read_with(cx, |b, _| {
        let ui = b.mail.as_ref().unwrap();
        assert_eq!(ui.drafts.len(), 1);
        assert!(ui.state.front.is_some(), "the reply is in front");
    });
    b.update_in(cx, |b, window, cx| b.forward_selected(window, cx));
    b.read_with(cx, |b, _| assert_eq!(b.mail.as_ref().unwrap().drafts.len(), 2));
}

#[gpui::test]
fn an_incognito_space_shows_none_of_the_mail_views(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    deliver(&mail, folders[0], "1", "ola@example.com", "<p>one</p>", true);
    b.update_in(cx, |b, window, cx| {
        let id = b.session.add_ephemeral_space("Incognito", b.host.ephemeral_profile());
        b.activate_space(id, window, cx);
        b.show_mail(true, window, cx);
        assert!(b.mail.as_ref().unwrap().space.is_none(), "no view state is read for it");
    });
    for view in ["mail-view", "mail-nav", "mail-commands", "mail-tab-inbox"] {
        assert!(!drawn(cx, view), "{view}");
    }
}
