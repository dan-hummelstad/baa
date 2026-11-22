//! The mail view's tabs (PLAN-mail.md §5): the list's tab, the preview tab of a narrow view,
//! the draft tabs, and the reading pane of a wide view, on MockHost and an in-memory Mail.

use baa_mail::{testing::message, Action, Flags, FolderId, Mail, Preview, Tab, ThreadId, View};
use baa_web::PageEvent;
use gpui::{px, size, Entity, VisualTestContext};

use crate::{mail::Field, mock::MockPage, tests_mail::*};

const HTML: &str = "Content-Type: text/html; charset=utf-8\r\n";

/// A window this wide leaves the mail view under `MAIL_WIDE` beside the open Archive.
const NARROW_WINDOW: f32 = 1000.;

/// A message `age` seconds old, so the list's order is known.
fn deliver(mail: &Mail, inbox: FolderId, age: u64, subject: &str, body: &str) {
    let raw = message("ola@example.com", "dan@example.com", subject, crate::mail::now_secs() - age, HTML, body);
    mail.seed_message(inbox, &age.to_string(), &raw, Flags::default(), true);
}

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

fn selected(b: &B, cx: &mut VisualTestContext) -> Option<ThreadId> {
    b.read_with(cx, |b, _| b.mail.as_ref().unwrap().state.selected)
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

fn documents(b: &B, cx: &mut VisualTestContext) -> usize {
    b.read_with(cx, |b, _| b.host.documents.borrow().len())
}

fn x_of(cx: &mut VisualTestContext, selector: &'static str) -> f32 {
    cx.run_until_parked();
    f32::from(cx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} is drawn")).origin.x)
}

/// Frames until a split's panes have grown in: the width is read once laid out, and the next
/// frame draws the layout it decides (frames only draw when asked under test).
fn settle(cx: &mut VisualTestContext) {
    for _ in 0..40 {
        cx.executor().advance_clock(std::time::Duration::from_millis(33));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
}

fn resize(cx: &mut VisualTestContext, width: f32) {
    cx.simulate_resize(size(px(width), px(900.)));
    settle(cx);
}

/// A narrow mail view with two conversations, "Two" on row 0 and "One" on row 1.
fn narrow(cx: &mut gpui::TestAppContext) -> (B, &mut VisualTestContext, std::sync::Arc<Mail>, Vec<FolderId>) {
    let (b, cx, mail, folders) = with_account(cx);
    deliver(&mail, folders[0], 20, "One", "<p>one</p>");
    deliver(&mail, folders[0], 10, "Two", "<p>two</p>");
    resize(cx, NARROW_WINDOW);
    assert_eq!(wide(&b, cx), Some(false));
    (b, cx, mail, folders)
}

#[gpui::test]
fn a_wide_mail_view_shows_the_conversation_beside_the_list_and_opens_no_preview_tab(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    deliver(&mail, folders[0], 10, "One", "<p>one</p>");
    settle(cx);
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

#[gpui::test]
fn another_conversation_replaces_the_preview_in_place_with_a_new_document(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, _) = narrow(cx);
    let (two, one) = (thread(&mail, &b, cx, 0), thread(&mail, &b, cx, 1));
    select(&b, cx, two);
    let first = page(&b, cx).unwrap();
    b.update_in(cx, |b, window, cx| assert!(b.back_to_list(window, cx)));
    select(&b, cx, one);
    assert_eq!((tab(&b, cx), selected(&b, cx)), (Tab::Preview, Some(one)));
    assert!(x_of(cx, "mail-tab-list") < x_of(cx, "mail-tab-preview"), "its place after the list's tab");
    assert_eq!(documents(&b, cx), 2, "a document is never reused across messages");
    assert_ne!(page(&b, cx).unwrap(), first);
    assert!(page(&b, cx).unwrap().read_with(cx, |p, _| p.set_documents.borrow()[0].contains("one")));
}

#[gpui::test]
fn a_reply_gets_a_draft_tab_that_a_later_preview_never_replaces(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, _) = narrow(cx);
    let (two, one) = (thread(&mail, &b, cx, 0), thread(&mail, &b, cx, 1));
    select(&b, cx, two);
    b.update_in(cx, |b, window, cx| window.focus(&b.mail.as_ref().unwrap().focus, cx));
    cx.simulate_keystrokes("cmd-r");
    cx.run_until_parked();
    let Tab::Draft(draft) = tab(&b, cx) else { panic!("the reply is in front") };
    assert_eq!(preview(&b, cx), Preview::Open, "the preview stays behind it");
    assert!(!page_visible(&b, cx), "the hidden preview's page is hidden");

    b.update(cx, |b, cx| b.show_tab(Tab::List, cx));
    select(&b, cx, one);
    assert_eq!(tab(&b, cx), Tab::Preview);
    b.read_with(cx, |b, _| assert_eq!(b.mail.as_ref().unwrap().drafts.iter().map(|d| d.id).collect::<Vec<_>>(), [draft]));
    let (list, preview_at) = (x_of(cx, "mail-tab-preview").min(x_of(cx, "mail-tab-list")), x_of(cx, "mail-tab-preview"));
    let draft_at = f32::from(cx.debug_bounds(Box::leak(format!("mail-draft-tab-{}", draft.0).into_boxed_str())).expect("the draft's tab").origin.x);
    assert!(list < preview_at && preview_at < draft_at, "the list's tab, the preview's, then the draft's");
}

#[gpui::test]
fn the_list_tab_is_first_and_never_closes(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, _) = narrow(cx);
    let t = thread(&mail, &b, cx, 0);
    select(&b, cx, t);
    b.update_in(cx, |b, window, cx| {
        b.show_tab(Tab::List, cx);
        window.focus(&b.mail.as_ref().unwrap().focus, cx);
    });
    cx.simulate_keystrokes("cmd-w");
    cx.run_until_parked();
    assert_eq!(tab(&b, cx), Tab::List);
    assert!(drawn(cx, "mail-tab-list") && drawn(cx, "mail-tab-preview"), "⌘W on the list's tab closes nothing");
    assert!(x_of(cx, "mail-tab-list") < x_of(cx, "mail-tab-preview"));

    b.update(cx, |b, cx| b.show_tab(Tab::Preview, cx));
    cx.simulate_keystrokes("cmd-w");
    cx.run_until_parked();
    assert_eq!((tab(&b, cx), preview(&b, cx)), (Tab::List, Preview::Closed), "⌘W closes the preview");
    assert!(page(&b, cx).is_none() && !drawn(cx, "mail-tab-preview"));
}

#[gpui::test]
fn the_back_control_cmd_bracket_and_esc_return_to_the_list_with_the_conversation_selected(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, _) = narrow(cx);
    let t = thread(&mail, &b, cx, 1);
    for way in ["the back control", "cmd-[", "escape"] {
        select(&b, cx, t);
        assert_eq!(tab(&b, cx), Tab::Preview, "{way}");
        b.update_in(cx, |b, window, cx| window.focus(&b.mail.as_ref().unwrap().focus, cx));
        match way {
            "the back control" => {
                drawn(cx, "mail-back");
                let at = cx.debug_bounds("mail-back").unwrap().center();
                cx.simulate_click(at, gpui::Modifiers::none());
            }
            keys => cx.simulate_keystrokes(keys),
        }
        cx.run_until_parked();
        assert_eq!((tab(&b, cx), preview(&b, cx), selected(&b, cx)), (Tab::List, Preview::Open, Some(t)), "{way}");
        assert!(drawn(cx, "mail-row-1") && !page_visible(&b, cx), "{way}: the list, and the preview's page hidden");
    }
}

#[gpui::test]
fn esc_in_a_text_field_or_the_message_page_does_not_go_back(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, _) = narrow(cx);
    let t = thread(&mail, &b, cx, 0);
    select(&b, cx, t);
    b.update_in(cx, |b, window, cx| b.focus_mail_field(Field::Search, window, cx));
    cx.simulate_keystrokes("escape");
    assert_eq!(tab(&b, cx), Tab::Preview, "Esc in the search field leaves the field only");

    let doc = page(&b, cx).unwrap();
    b.update_in(cx, |_, window, cx| window.focus(&doc.read(cx).focus.clone(), cx));
    cx.simulate_keystrokes("escape");
    assert_eq!(tab(&b, cx), Tab::Preview, "Esc in the message is the page's");

    b.update_in(cx, |b, window, cx| b.reply_to_selected(false, window, cx));
    let draft = tab(&b, cx);
    assert!(matches!(draft, Tab::Draft(_)));
    b.update_in(cx, |b, window, cx| b.focus_mail_field(Field::Subject, window, cx));
    cx.simulate_keystrokes("escape");
    assert_eq!(tab(&b, cx), draft, "Esc in a draft's field leaves the field only");
}

#[gpui::test]
fn delete_archive_and_move_from_the_preview_return_to_the_list(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = narrow(cx);
    deliver(&mail, folders[0], 5, "Three", "<p>three</p>");
    cx.run_until_parked();
    for action in [Action::Delete, Action::Archive, Action::Move(folders[3])] {
        let t = thread(&mail, &b, cx, 0);
        select(&b, cx, t);
        assert_eq!(tab(&b, cx), Tab::Preview);
        b.update(cx, |b, cx| b.act_on_selected(action, cx));
        cx.run_until_parked();
        assert_eq!((tab(&b, cx), preview(&b, cx), selected(&b, cx)), (Tab::List, Preview::Closed, None), "{action:?}");
        assert!(mail.conversation(t).is_empty(), "{action:?} reached mail");
        assert!(drawn(cx, "mail-tab-list") && !drawn(cx, "mail-tab-preview"));
    }
}

#[gpui::test]
fn the_tab_in_front_comes_back_after_a_space_switch_from_mail_db(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, _) = narrow(cx);
    let t = thread(&mail, &b, cx, 0);
    select(&b, cx, t);
    let space = mail_space(&b, cx);
    let kept = mail.view_state(space);
    assert_eq!((kept.tab(), kept.selected), (Tab::Preview, Some(t)), "kept in mail.db");

    let home = b.update_in(cx, |b, window, cx| {
        let home = b.session.active_space().unwrap();
        let other = b.session.add_space("Work", "briefcase", None);
        b.activate_space(other, window, cx);
        home
    });
    b.read_with(cx, |b, _| assert!(b.mail.as_ref().unwrap().reader.is_none(), "Work has its own view"));
    b.update_in(cx, |b, window, cx| {
        b.activate_space(home, window, cx);
        b.show_mail(true, window, cx);
    });
    settle(cx);
    assert_eq!(tab(&b, cx), Tab::Preview, "read back from mail.db");
    assert_eq!(b.read_with(cx, |b, _| b.mail.as_ref().unwrap().reader.as_ref().map(|r| r.thread)), Some(t));
    assert!(page_visible(&b, cx));
}

#[gpui::test]
fn a_link_in_a_previewed_message_is_adopted_and_never_loaded(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = narrow(cx);
    deliver(&mail, folders[0], 1, "Link", "<a href=\"https://evil.example/x\">x</a>");
    cx.run_until_parked();
    let t = thread(&mail, &b, cx, 0);
    select(&b, cx, t);
    let doc = page(&b, cx).unwrap();
    doc.update(cx, |_, cx| cx.emit(PageEvent::OpenRequested { url: "https://evil.example/x".into(), from_user_gesture: true, background: false }));
    cx.run_until_parked();
    b.read_with(cx, |b, cx| {
        assert_eq!(*b.host.supplied.borrow(), ["https://evil.example/x"], "Chrome's own tab is adopted");
        for (page, _) in b.pages.values() {
            assert!(page.read(cx).loads.borrow().is_empty(), "no load_url with a URL from mail");
        }
    });
}

#[gpui::test]
fn going_wide_folds_the_preview_into_the_reading_pane_with_the_same_document(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, _) = narrow(cx);
    let t = thread(&mail, &b, cx, 0);
    select(&b, cx, t);
    let doc = page(&b, cx).unwrap();
    resize(cx, 1536.);
    assert_eq!(wide(&b, cx), Some(true));
    assert_eq!((tab(&b, cx), preview(&b, cx), selected(&b, cx)), (Tab::List, Preview::Closed, Some(t)));
    assert_eq!(page(&b, cx), Some(doc), "the same message keeps its document");
    assert!(page_visible(&b, cx) && drawn(cx, "mail-row-0") && !drawn(cx, "mail-tab-preview"));
}

#[gpui::test]
fn going_narrow_shows_the_list_with_the_conversation_selected_and_no_preview_tab(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    deliver(&mail, folders[0], 10, "One", "<p>one</p>");
    settle(cx);
    let t = thread(&mail, &b, cx, 0);
    select(&b, cx, t);
    assert!(page_visible(&b, cx));
    resize(cx, NARROW_WINDOW);
    assert_eq!(wide(&b, cx), Some(false));
    assert_eq!((tab(&b, cx), preview(&b, cx), selected(&b, cx)), (Tab::List, Preview::Closed, Some(t)));
    assert!(!page_visible(&b, cx), "no pane to show it in");
    assert!(drawn(cx, "mail-row-0") && !drawn(cx, "mail-tab-preview"));
}

#[gpui::test]
fn a_split_pane_at_half_a_wide_window_is_narrow(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    deliver(&mail, folders[0], 10, "One", "<p>one</p>");
    settle(cx);
    assert_eq!(wide(&b, cx), Some(true));
    open(&b, cx, "https://a.com/");
    b.update_in(cx, |b, window, cx| b.show_mail(true, window, cx));
    cx.dispatch_action(crate::actions::SplitView);
    b.update_in(cx, |b, window, cx| b.show_mail(true, window, cx));
    settle(cx);
    assert_eq!(wide(&b, cx), Some(false), "the mail pane is half the window");
    let t = thread(&mail, &b, cx, 0);
    select(&b, cx, t);
    assert_eq!(tab(&b, cx), Tab::Preview);
}

#[gpui::test]
fn a_tabs_x_closes_it_without_choosing_it(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, _) = narrow(cx);
    let t = thread(&mail, &b, cx, 0);
    select(&b, cx, t);
    b.update_in(cx, |b, window, cx| b.reply_to_selected(false, window, cx));
    let Tab::Draft(draft) = tab(&b, cx) else { panic!("the reply is in front") };
    let x = Box::leak(format!("mail-draft-tab-x-{}", draft.0).into_boxed_str());
    drawn(cx, x);
    let at = cx.debug_bounds(x).unwrap().center();
    cx.simulate_click(at, gpui::Modifiers::none());
    cx.run_until_parked();
    assert_eq!(tab(&b, cx), Tab::Preview, "the tab on its left");
    b.read_with(cx, |b, _| assert!(b.mail.as_ref().unwrap().drafts.is_empty()));

    drawn(cx, "mail-tab-preview-x-0");
    let at = cx.debug_bounds("mail-tab-preview-x-0").unwrap().center();
    cx.simulate_click(at, gpui::Modifiers::none());
    cx.run_until_parked();
    assert_eq!((tab(&b, cx), preview(&b, cx)), (Tab::List, Preview::Closed));
}
