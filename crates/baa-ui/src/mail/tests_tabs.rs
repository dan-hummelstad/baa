//! The mail view's tabs (PLAN-mail.md §5): the list's tab, the preview tab of a narrow view,
//! the draft tabs, and the reading pane of a wide view, on MockHost and an in-memory Mail.

use baa_mail::{testing::message, Flags, FolderId, Mail, Preview, Tab, ThreadId, View};
use gpui::{size, px, Entity, VisualTestContext};

use crate::{mock::MockPage, tests_mail::*};

const HTML: &str = "Content-Type: text/html; charset=utf-8\r\n";

/// A window this wide leaves the mail view under `MAIL_WIDE` beside the open Archive.
const NARROW_WINDOW: f32 = 1000.;

fn deliver(mail: &Mail, inbox: FolderId, uid: &str, subject: &str, body: &str) {
    let raw = message("ola@example.com", "dan@example.com", subject, crate::mail::now_secs() - uid.parse::<u64>().unwrap(), HTML, body);
    mail.seed_message(inbox, uid, &raw, Flags::default(), true);
}

/// The thread on row `n` of the list.
fn thread(mail: &Mail, b: &B, cx: &mut VisualTestContext, n: usize) -> ThreadId {
    let view: View = b.read_with(cx, |b, _| b.mail.as_ref().unwrap().view().unwrap());
    mail.rows(&view, n..n + 1)[0].thread
}

fn select(b: &B, cx: &mut VisualTestContext, thread: ThreadId) {
    b.update_in(cx, |b, window, cx| b.select_thread(thread, window, cx));
    cx.run_until_parked();
}

fn tab(b: &B, cx: &mut VisualTestContext) -> Tab {
    b.read_with(cx, |b, _| b.mail.as_ref().unwrap().state.tab())
}

fn preview(b: &B, cx: &mut VisualTestContext) -> Preview {
    b.read_with(cx, |b, _| b.mail.as_ref().unwrap().state.preview)
}

fn wide(b: &B, cx: &mut VisualTestContext) -> Option<bool> {
    b.read_with(cx, |b, _| b.mail.as_ref().unwrap().wide)
}

fn page(b: &B, cx: &mut VisualTestContext) -> Option<Entity<MockPage>> {
    b.read_with(cx, |b, _| b.mail.as_ref().unwrap().reader.as_ref().and_then(|r| r.page.clone()))
}

fn page_visible(b: &B, cx: &mut VisualTestContext) -> bool {
    page(b, cx).is_some_and(|p| p.read_with(cx, |p, _| p.visible.get()))
}

fn resize(cx: &mut VisualTestContext, width: f32) {
    cx.simulate_resize(size(px(width), px(900.)));
    cx.run_until_parked();
    // The width is read once laid out; the next frame draws the layout it decides.
    drawn(cx, "mail-view");
}

/// A narrow mail view with two conversations, "One" on row 1 and "Two" on row 0.
fn narrow(cx: &mut gpui::TestAppContext) -> (B, &mut VisualTestContext, std::sync::Arc<Mail>, Vec<FolderId>) {
    let (b, cx, mail, folders) = with_account(cx);
    deliver(&mail, folders[0], "2", "One", "<p>one</p>");
    deliver(&mail, folders[0], "1", "Two", "<p>two</p>");
    cx.run_until_parked();
    resize(cx, NARROW_WINDOW);
    assert_eq!(wide(&b, cx), Some(false));
    (b, cx, mail, folders)
}

#[gpui::test]
fn a_wide_mail_view_shows_the_conversation_beside_the_list_and_opens_no_preview_tab(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    deliver(&mail, folders[0], "1", "One", "<p>one</p>");
    cx.run_until_parked();
    drawn(cx, "mail-view");
    assert_eq!(wide(&b, cx), Some(true), "the default test window is wide");
    let t = thread(&mail, &b, cx, 0);
    select(&b, cx, t);
    assert_eq!((tab(&b, cx), preview(&b, cx)), (Tab::List, Preview::Closed));
    assert!(drawn(cx, "mail-row-0") && drawn(cx, "mail-sender-address"), "list and message side by side");
    assert!(!drawn(cx, "mail-tab-preview"));
    assert!(page_visible(&b, cx));
}

#[gpui::test]
fn a_narrow_mail_view_opens_the_conversation_in_a_preview_tab_in_front(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, _) = narrow(cx);
    let t = thread(&mail, &b, cx, 0);
    select(&b, cx, t);
    assert_eq!(tab(&b, cx), Tab::Preview);
    assert!(drawn(cx, "mail-tab-preview") && drawn(cx, "mail-sender-address") && drawn(cx, "mail-back"));
    assert!(!drawn(cx, "mail-row-0"), "the message has the whole view");
    assert!(page_visible(&b, cx));
}
