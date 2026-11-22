//! Tests of list, on MockHost and an in-memory Mail.

use baa_mail::{testing, AccountId, Action, Filter, Flags, MailSpace, View};
use baa_model::SpaceId;
use gpui::TestAppContext;

use super::{nav::inbox_unread, Field};
use crate::{browser::menus::MenuEntry, tests_mail::*};

fn mail_in(ts: u64, from: &str, subject: &str) -> Vec<u8> {
    testing::message(from, "dan@example.com", subject, ts, "", "Hei")
}

fn view_of(b: &B, cx: &mut gpui::VisualTestContext, space: MailSpace) -> View {
    b.read_with(cx, |b, _| b.mail.as_ref().unwrap().view().unwrap_or(View { space, folder: None, account: None, filter: Filter::All, search: String::new() }))
}

#[gpui::test]
fn two_accounts_make_one_list_and_the_chips_filter_it(cx: &mut TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    let space = mail_space(&b, cx);
    let second = mail.seed_account(&testing::account(2, Some(space), "dan@work.example"));
    mail.seed_message(folders[0], "1", &mail_in(1_700_000_000, "a@x.example", "One"), Flags::default(), false);
    mail.seed_message(second[0], "1", &mail_in(1_700_000_100, "b@x.example", "Two"), Flags::default(), false);
    b.update(cx, |_, cx| cx.notify());
    assert!(drawn(cx, "mail-row-0") && drawn(cx, "mail-row-1"));
    assert_eq!(mail.count(&view_of(&b, cx, space)), 2);

    b.update(cx, |b, cx| b.set_scope(Some(AccountId(2)), cx));
    assert_eq!(mail.count(&view_of(&b, cx, space)), 1);
    assert!(drawn(cx, "mail-row-0") && !drawn(cx, "mail-row-1"));
    assert_eq!(mail.view_state(space).account, Some(AccountId(2)));
}

#[gpui::test]
fn the_fold_lists_each_accounts_folders_a_folder_opens_in_the_mail_view_and_an_account_folds_away(cx: &mut TestAppContext) {
    let (b, cx, mail, _) = with_account(cx);
    let space = mail_space(&b, cx);
    let second = mail.seed_account(&testing::account(2, Some(space), "dan@work.example"));
    b.update(cx, |b, cx| b.set_scope(Some(AccountId(1)), cx));
    assert!(drawn(cx, "nav-account-1") && drawn(cx, "nav-account-2"), "every account, whatever the scope");
    let sent: &'static str = format!("nav-folder-{}", second[2].0).leak();
    let click = |cx: &mut gpui::VisualTestContext, at: &'static str| {
        let at = cx.debug_bounds(at).unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
    };
    click(cx, sent);
    b.read_with(cx, |b, _| {
        let state = &b.mail.as_ref().unwrap().state;
        assert_eq!((state.folder, state.account), (Some(second[2]), None), "the folder, out of the other account's scope");
    });
    click(cx, "nav-account-2");
    assert!(!drawn(cx, sent), "folded away");
    click(cx, "nav-account-2");
    assert!(drawn(cx, sent), "and back");
}

#[gpui::test]
fn the_unread_count_follows_a_read(cx: &mut TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    let space = mail_space(&b, cx);
    let id = mail.seed_message(folders[0], "1", &mail_in(1_700_000_000, "a@x.example", "One"), Flags::default(), false);
    assert_eq!(inbox_unread(&mail.folders(space), None), 1);
    mail.act(&[id], Action::Read(true));
    assert_eq!(inbox_unread(&mail.folders(space), None), 0);
    drop((b, cx));
}

#[gpui::test]
fn ten_thousand_rows_scroll_without_being_built(cx: &mut TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    for i in 0..10_000u64 {
        mail.seed_message(folders[0], &(i + 1).to_string(), &mail_in(1_700_000_000 + i, "a@x.example", "Hi"), Flags::default(), false);
    }
    assert!(drawn(cx, "mail-row-0"));
    assert!(!drawn(cx, "mail-row-9999"));
    b.read_with(cx, |b, _| b.mail.as_ref().unwrap().list.scroll_to_item(9_999, gpui::ScrollStrategy::Top));
    b.update(cx, |_, cx| cx.notify());
    assert!(drawn(cx, "mail-row-9999") && !drawn(cx, "mail-row-0"));
}

#[gpui::test]
fn typing_backspace_in_the_search_field_deletes_a_character_not_a_message(cx: &mut TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    let space = mail_space(&b, cx);
    mail.seed_message(folders[0], "1", &mail_in(1_700_000_000, "a@x.example", "One"), Flags::default(), false);
    b.update_in(cx, |b, w, cx| b.focus_mail_field(Field::Search, w, cx));
    cx.simulate_keystrokes("a b backspace");
    b.read_with(cx, |b, _| assert_eq!(b.mail.as_ref().unwrap().search.text, "a"));
    let all = View { space, folder: None, account: None, filter: Filter::All, search: String::new() };
    assert_eq!(mail.count(&all), 1);
}

#[gpui::test]
fn a_folder_and_the_scope_come_back_after_a_space_switch(cx: &mut TestAppContext) {
    let (b, cx, _mail, folders) = with_account(cx);
    let first: SpaceId = b.read_with(cx, |b, _| b.session.active_space().unwrap());
    b.update(cx, |b, cx| b.open_list(Some(folders[3]), Filter::All, cx));
    cx.dispatch_action(crate::actions::NewSpace);
    cx.simulate_keystrokes("escape");
    b.update_in(cx, |b, w, cx| {
        assert_ne!(b.session.active_space(), Some(first));
        b.activate_space(first, w, cx);
    });
    b.read_with(cx, |b, _| assert_eq!(b.mail.as_ref().unwrap().state.folder, Some(folders[3])));
}

#[gpui::test]
fn the_mail_section_shows_the_add_an_account_card_without_an_account(cx: &mut TestAppContext) {
    let (b, cx) = with_mail(cx);
    b.update_in(cx, |b, window, cx| b.show_mail(true, window, cx));
    assert!(drawn(cx, "mail-add-account") && !drawn(cx, "mail-account-select"));
    let space = mail_space(&b, cx);
    mail(&b, cx).seed_account(&testing::account(1, Some(space), "dan@example.com"));
    b.update(cx, |_, cx| cx.notify());
    assert!(drawn(cx, "mail-account-select") && !drawn(cx, "mail-add-account"));
}

fn scope_menu(b: &B, cx: &mut gpui::VisualTestContext) -> Vec<(String, bool)> {
    b.read_with(cx, |b, _| {
        b.mail_scope_menu()
            .into_iter()
            .map(|e| match e {
                MenuEntry::Item { label, checked, .. } => (label, checked),
                _ => ("---".into(), false),
            })
            .collect()
    })
}

fn run_scope_item(b: &B, cx: &mut gpui::VisualTestContext, at: usize) {
    b.update_in(cx, |b, window, cx| {
        if let Some(MenuEntry::Item { run, .. }) = b.mail_scope_menu().into_iter().nth(at) {
            run(b, window, cx);
        }
    });
}

#[gpui::test]
fn the_account_picker_narrows_the_list_to_one_account_and_back(cx: &mut TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    let space = mail_space(&b, cx);
    let second = mail.seed_account(&testing::account(2, Some(space), "dan@work.example"));
    mail.seed_message(folders[0], "1", &mail_in(1_700_000_000, "a@x.example", "One"), Flags::default(), false);
    mail.seed_message(second[0], "1", &mail_in(1_700_000_100, "b@x.example", "Two"), Flags::default(), false);
    b.update(cx, |_, cx| cx.notify());
    let menu = scope_menu(&b, cx);
    assert_eq!(menu.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["All inboxes", "dan@example.com", "dan@work.example"]);
    assert_eq!(menu.iter().map(|(_, c)| *c).collect::<Vec<_>>(), [true, false, false]);

    run_scope_item(&b, cx, 2);
    b.read_with(cx, |b, _| assert_eq!(b.mail.as_ref().unwrap().state.account, Some(AccountId(2))));
    assert_eq!(mail.count(&view_of(&b, cx, space)), 1);
    assert!(drawn(cx, "mail-account-select") && drawn(cx, "mail-row-0") && !drawn(cx, "mail-row-1"));
    assert_eq!(scope_menu(&b, cx).iter().map(|(_, c)| *c).collect::<Vec<_>>(), [false, false, true]);

    run_scope_item(&b, cx, 0);
    assert_eq!(mail.count(&view_of(&b, cx, space)), 2);
    assert_eq!(scope_menu(&b, cx)[0], ("All inboxes".to_string(), true));

    b.update_in(cx, |b, window, cx| {
        b.host.menu_pick.set(Some(1));
        b.open_menu(crate::browser::MenuKind::MailScope, gpui::point(gpui::px(10.), gpui::px(10.)), window, cx)
    });
    cx.run_until_parked();
    b.read_with(cx, |b, _| {
        assert_eq!(b.host.menus.borrow().last().unwrap().len(), 3);
        assert_eq!(b.mail.as_ref().unwrap().state.account, Some(AccountId(1)));
    });
}

#[gpui::test]
fn go_to_in_the_picker_opens_another_spaces_mail(cx: &mut TestAppContext) {
    let (b, cx, _mail, _folders) = with_account(cx);
    let first: SpaceId = b.read_with(cx, |b, _| b.session.active_space().unwrap());
    cx.dispatch_action(crate::actions::NewSpace);
    cx.simulate_keystrokes("escape");
    let (other, other_space) = (b.read_with(cx, |b, _| b.session.active_space().unwrap()), mail_space(&b, cx));
    assert_ne!(other, first);
    mail(&b, cx).seed_account(&testing::account(2, Some(other_space), "dan@work.example"));
    let name = b.read_with(cx, |b, _| b.session.space(other).unwrap().name.clone());
    b.update_in(cx, |b, w, cx| b.activate_space(first, w, cx));
    b.update_in(cx, |b, w, cx| b.show_mail(true, w, cx));

    let menu = scope_menu(&b, cx);
    assert_eq!(&menu[menu.len() - 2..], [("---".to_string(), false), (format!("Go to {name}"), false)]);
    run_scope_item(&b, cx, menu.len() - 1);
    cx.run_until_parked();
    b.read_with(cx, |b, _| {
        assert_eq!(b.session.active_space(), Some(other));
        assert!(b.mail_in_front());
        assert_eq!(b.library.as_ref().map(|l| l.section), Some(crate::library::Section::Mail));
    });
}

/// `cargo test --release -p baa-ui mail_view_frame_time -- --ignored --nocapture`
#[gpui::test]
#[ignore = "timing, run by hand"]
fn mail_view_frame_time(cx: &mut TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    let space = mail_space(&b, cx);
    let second = mail.seed_account(&testing::account(2, Some(space), "dan@work.example"));
    let seed = |i: u64| {
        let inbox = if i.is_multiple_of(2) { folders[0] } else { second[0] };
        let flags = Flags { seen: !i.is_multiple_of(3), ..Flags::default() };
        mail.seed_message(inbox, &i.to_string(), &mail_in(1_763_800_000 - i * 1_583, &format!("p{}@x.example", i % 97), &format!("Subject number {i}")), flags, false);
    };
    (0..20_000).for_each(seed);
    let mut next = 20_000;
    let mut frames = |cx: &mut gpui::VisualTestContext, n: u64, scroll: bool, sync: bool| {
        let start = std::time::Instant::now();
        for f in 0..n {
            if sync && f.is_multiple_of(10) {
                seed(next);
                next += 1;
            }
            b.update(cx, |b, cx| {
                if scroll {
                    b.mail.as_ref().unwrap().list.scroll_to_item((f * 7) as usize, gpui::ScrollStrategy::Top);
                }
                cx.notify();
            });
            cx.run_until_parked();
        }
        start.elapsed() / n as u32
    };
    frames(cx, 5, false, false);
    let still = frames(cx, 60, false, false);
    let scrolling = frames(cx, 120, true, false);
    let syncing = frames(cx, 120, true, true);
    b.update_in(cx, |b, window, cx| b.show_mail(false, window, cx));
    frames(cx, 5, false, false);
    let without = frames(cx, 60, false, false);
    eprintln!("frame: still {still:?}, scrolling {scrolling:?}, scrolling while syncing {syncing:?}, mail closed {without:?}");
}
