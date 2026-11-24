//! The code offer (WP-L, PLAN-mail.md §7), on MockHost and an in-memory Mail.

use std::time::Duration;

use baa_mail::{testing::message, Flags, FolderId, Mail};
use baa_model::{DevToolsDock, Pane, TabId};
use baa_web::{PageEvent, WebHost};
use gpui::{px, Bounds, VisualTestContext};

use crate::tests_mail::*;

fn code_mail(mail: &Mail, inbox: FolderId, uid: &str, from: &str) {
    let raw = message(from, "dan@example.com", "Sign in", crate::mail::now_secs(), "", "Your verification code is 123456.\r\n");
    mail.seed_message(inbox, uid, &raw, Flags::default(), true);
}

/// A tab at `url` in front, the mock engine's sites set for the bank.
fn bank_tab(b: &B, cx: &mut VisualTestContext, url: &str) -> TabId {
    b.update_in(cx, |b, window, cx| b.show_mail(false, window, cx));
    b.read_with(cx, |b, _| {
        let mut sites = b.host.sites.borrow_mut();
        sites.insert("login.bank.example".into(), "bank.example".into());
        sites.insert("bank.example".into(), "bank.example".into());
    });
    open(b, cx, url)
}

fn field(b: &B, cx: &mut VisualTestContext, tab: TabId, id: u64, origin: &str, top: &str) {
    let page = b.read_with(cx, |b, _| b.pages[&tab].0.clone());
    let at = Some(Bounds::new(gpui::point(gpui::px(40.), gpui::px(200.)), gpui::size(gpui::px(160.), gpui::px(28.))));
    page.update(cx, |_, cx| cx.emit(PageEvent::CodeField { id, origin: origin.into(), top: top.into(), at }));
    cx.run_until_parked();
}

fn offered(b: &B, cx: &mut VisualTestContext) -> bool {
    b.read_with(cx, |b, _| b.mail.as_ref().is_some_and(|m| m.offers.offer.is_some()))
}

fn filled(b: &B, cx: &mut VisualTestContext, tab: TabId) -> Vec<(u64, String)> {
    b.read_with(cx, |b, cx| b.pages[&tab].0.read(cx).filled.borrow().clone())
}

const BANK: &str = "https://login.bank.example";

#[gpui::test]
fn the_offer_ignores_a_click_in_its_first_half_second_and_enter_always(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    code_mail(&mail, folders[0], "1", "Bank <no-reply@bank.example>");
    let tab = bank_tab(&b, cx, "https://login.bank.example/");
    field(&b, cx, tab, 7, BANK, BANK);
    assert!(drawn(cx, "code-offer-7"), "drawn under the field, its id from the field's");

    b.update(cx, |b, cx| b.accept_offer(cx));
    assert!(filled(&b, cx, tab).is_empty(), "a click meant for the page");
    cx.simulate_keystrokes("enter");
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.simulate_keystrokes("enter");
    assert!(filled(&b, cx, tab).is_empty(), "Enter never accepts");

    b.update(cx, |b, cx| b.accept_offer(cx));
    assert_eq!(filled(&b, cx, tab), [(7, "123456".to_string())]);
    assert!(!offered(&b, cx), "gone once used");
}

#[gpui::test]
fn the_offer_goes_when_the_field_goes_the_tab_navigates_or_two_minutes_pass(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    code_mail(&mail, folders[0], "1", "no-reply@bank.example");
    let tab = bank_tab(&b, cx, "https://login.bank.example/");
    let page = b.read_with(cx, |b, _| b.pages[&tab].0.clone());

    field(&b, cx, tab, 7, BANK, BANK);
    page.update(cx, |_, cx| cx.emit(PageEvent::CodeField { id: 7, origin: BANK.into(), top: BANK.into(), at: None }));
    assert!(!offered(&b, cx));

    field(&b, cx, tab, 8, BANK, BANK);
    page.update(cx, |_, cx| cx.emit(PageEvent::UrlChanged("https://login.bank.example/next".into())));
    assert!(!offered(&b, cx));

    field(&b, cx, tab, 9, BANK, BANK);
    cx.executor().advance_clock(Duration::from_secs(121));
    cx.run_until_parked();
    assert!(!offered(&b, cx));
}

#[gpui::test]
fn nothing_is_offered_in_an_incognito_space_a_background_tab_or_for_a_cross_site_frame(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    code_mail(&mail, folders[0], "1", "no-reply@bank.example");
    let behind = bank_tab(&b, cx, "https://login.bank.example/");
    let front = open(&b, cx, "https://login.bank.example/other");
    field(&b, cx, behind, 1, BANK, BANK);
    assert!(!offered(&b, cx), "a background tab");

    field(&b, cx, front, 2, "https://evil.example", BANK);
    assert!(!offered(&b, cx), "a cross-site frame");
    field(&b, cx, front, 3, "http://login.bank.example", BANK);
    assert!(!offered(&b, cx), "not over https");
    field(&b, cx, front, 4, BANK, BANK);
    assert!(offered(&b, cx), "the top frame of the tab in front");

    b.update_in(cx, |b, window, cx| {
        let id = b.session.add_ephemeral_space("Incognito", b.host.ephemeral_profile());
        b.activate_space(id, window, cx);
    });
    let private = open(&b, cx, "https://login.bank.example/");
    field(&b, cx, private, 6, BANK, BANK);
    assert!(!offered(&b, cx), "an incognito Space");
    assert!(!drawn(cx, "code-offer-6"));
}

#[gpui::test]
fn in_a_split_the_offer_is_under_the_field_of_the_tabs_pane_and_moves_with_docked_devtools(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    code_mail(&mail, folders[0], "1", "no-reply@bank.example");
    let other = bank_tab(&b, cx, "https://other.example/");
    let tab = open(&b, cx, "https://login.bank.example/");
    b.update_in(cx, |b, window, cx| {
        let space = b.session.active_space().unwrap();
        b.session.add_to_split(space, other, 0, baa_model::now());
        b.activate_tab(tab, window, cx);
        assert_eq!(b.panes_on_screen(), [Pane::Tab(other), Pane::Tab(tab)]);
    });
    // Frames only draw when asked under test, and the panes grow in on springs.
    let settle = |cx: &mut VisualTestContext| {
        for _ in 0..40 {
            cx.executor().advance_clock(Duration::from_millis(33));
            cx.update(|window, _| window.refresh());
            cx.run_until_parked();
        }
    };
    settle(cx);
    field(&b, cx, tab, 7, BANK, BANK);
    assert!(drawn(cx, "code-offer-7"), "offered in a split");

    let placed = |b: &B, cx: &mut VisualTestContext| {
        settle(cx);
        let offer = cx.debug_bounds("code-offer-7").expect("drawn");
        let (pane, width) = b.update_in(cx, |b, window, _| (b.tab_area.get(), window.viewport_size().width));
        assert!(offer.left() >= pane.left() && offer.right() <= pane.right(), "within the tab's pane: {offer:?} in {pane:?}");
        assert_eq!(offer.top(), pane.top() + px(232.), "under the field");
        (offer, pane, width)
    };
    let (offer, pane, width) = placed(&b, cx);
    assert!(pane.left() > width / 2. - px(20.), "the right pane: {pane:?}");
    assert_eq!(offer.left(), pane.left() + px(40.));

    b.update_in(cx, |b, window, cx| {
        b.set_devtools_dock(DevToolsDock::Left, window, cx);
        b.toggle_devtools(None, window, cx);
    });
    cx.run_until_parked();
    assert!(offered(&b, cx), "DevTools docking keeps the offer");
    let (_, docked, _) = placed(&b, cx);
    assert!(docked.left() > pane.left(), "DevTools takes the pane's left: {docked:?}");
}

#[gpui::test]
fn codes_off_in_settings_offer_nothing(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    code_mail(&mail, folders[0], "1", "no-reply@bank.example");
    let tab = bank_tab(&b, cx, "https://login.bank.example/");
    b.update(cx, |b, _| b.settings.mail.codes = false);
    field(&b, cx, tab, 1, BANK, BANK);
    assert!(!offered(&b, cx));
}

#[gpui::test]
fn a_private_window_offers_nothing(cx: &mut gpui::TestAppContext) {
    let (_b, cx) = with_mail(cx);
    let h = cx.cx.update(|cx| crate::open_window::<crate::mock::MockHost>(true, cx)).expect("a window opens");
    let pcx = VisualTestContext::from_window(h.into(), &cx.cx).into_mut();
    let p = h.root(pcx).unwrap();
    let tab = open(&p, pcx, "https://login.bank.example/");
    field(&p, pcx, tab, 1, BANK, BANK);
    assert!(p.read_with(pcx, |p, _| p.mail.is_none()) && !drawn(pcx, "code-offer-1"));
}

#[gpui::test]
fn two_fields_within_ten_seconds_cause_one_check(cx: &mut gpui::TestAppContext) {
    let (b, cx, _mail, _) = with_account(cx);
    let tab = bank_tab(&b, cx, "https://login.bank.example/");
    let checks = |b: &B, cx: &mut VisualTestContext| b.read_with(cx, |b, _| b.mail.as_ref().unwrap().offers.checks);
    field(&b, cx, tab, 1, BANK, BANK);
    cx.executor().advance_clock(Duration::from_secs(5));
    field(&b, cx, tab, 2, BANK, BANK);
    assert_eq!(checks(&b, cx), 1);
    cx.executor().advance_clock(Duration::from_secs(6));
    field(&b, cx, tab, 3, BANK, BANK);
    assert_eq!(checks(&b, cx), 2);
}

#[gpui::test]
fn a_code_from_another_site_shows_nothing_and_new_mail_fills_the_offer(cx: &mut gpui::TestAppContext) {
    let (b, cx, mail, folders) = with_account(cx);
    code_mail(&mail, folders[0], "1", "no-reply@shop.example");
    let tab = bank_tab(&b, cx, "https://login.bank.example/");
    field(&b, cx, tab, 7, BANK, BANK);
    assert!(offered(&b, cx) && !drawn(cx, "code-offer-7"), "waiting, with nothing to show");

    code_mail(&mail, folders[0], "2", "no-reply@bank.example");
    cx.run_until_parked();
    assert!(drawn(cx, "code-offer-7"), "the code arrived while the field waited");
}
