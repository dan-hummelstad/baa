//! "Use this account in all Spaces?" after a sign-in, and what each answer stores.

use std::time::Duration;

use baa_model::{Account, ProfileId, Settings, Store};
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::{mock::MockHost, Browser};

fn browser_with(store: Store, cx: &mut TestAppContext) -> (Entity<Browser<MockHost>>, &mut VisualTestContext) {
    cx.update(crate::init);
    cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), store, window, cx))
}

fn browser(cx: &mut TestAppContext) -> (Entity<Browser<MockHost>>, &mut VisualTestContext) {
    browser_with(Store::open(":memory:").unwrap(), cx)
}

/// The profiles of the first Space and of a second one.
fn two_profiles(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) -> (ProfileId, ProfileId) {
    b.update_in(cx, |b, window, cx| {
        let first = b.session.active_space().unwrap();
        b.new_space(window, cx);
        b.close_overlays(window, cx);
        let second = b.session.active_space().unwrap();
        b.activate_space(first, window, cx);
        (b.session.space(first).unwrap().profile, b.session.space(second).unwrap().profile)
    })
}

fn sign_in(b: &Entity<Browser<MockHost>>, profile: ProfileId, account: Account, cx: &mut VisualTestContext) {
    b.update(cx, |b, cx| {
        b.host.sign_ins.update(cx, |list, cx| {
            list.0.push((profile, account));
            cx.notify();
        })
    });
    cx.run_until_parked();
}

fn asks(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) -> Vec<(u64, ProfileId, Account)> {
    b.read_with(cx, |b, _| b.account_asks.iter().map(|a| (a.id, a.profile, a.account)).collect())
}

fn saved(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) -> Settings {
    b.read_with(cx, |b, _| b.store.as_ref().unwrap().load_settings().unwrap())
}

#[gpui::test]
fn sharing_waits_out_the_delay_then_shares_from_the_space_that_signed_in(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let (p1, p2) = two_profiles(&b, cx);
    sign_in(&b, p2, Account::Google, cx);
    let [(id, profile, account)] = asks(&b, cx)[..] else { panic!("one ask") };
    assert_eq!((profile, account), (p2, Account::Google));
    assert!(b.read_with(cx, |b, _| b.host.shares.borrow().is_empty()), "nothing is shared before an answer");

    // A click that lands as the ask appears does nothing.
    b.update(cx, |b, cx| b.answer_account(id, true, cx));
    assert_eq!(asks(&b, cx).len(), 1);

    cx.executor().advance_clock(Duration::from_millis(600));
    b.update(cx, |b, cx| b.answer_account(id, true, cx));
    assert!(asks(&b, cx).is_empty());
    assert_eq!(saved(&b, cx).shared_accounts, [Account::Google]);
    assert_eq!(b.read_with(cx, |b, _| b.host.shares.borrow().clone()), [(vec![Account::Google], Some(p2))]);

    // Shared: a sign-in anywhere is not asked about again.
    sign_in(&b, p1, Account::Google, cx);
    assert!(asks(&b, cx).is_empty());

    b.update(cx, |b, cx| b.unshare_account(Account::Google, cx));
    assert!(saved(&b, cx).shared_accounts.is_empty());
    assert_eq!(b.read_with(cx, |b, _| b.host.shares.borrow().last().cloned()), Some((vec![], None)));
}

#[gpui::test]
fn keeping_a_sign_in_to_its_space_is_remembered_for_that_space_only(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let (p1, p2) = two_profiles(&b, cx);
    sign_in(&b, p1, Account::Microsoft, cx);
    // The same sign-in seen again while it is asked: one ask.
    sign_in(&b, p1, Account::Microsoft, cx);
    let [(id, ..)] = asks(&b, cx)[..] else { panic!("one ask") };
    // Keeping needs no wait: it changes nothing.
    b.update(cx, |b, cx| b.answer_account(id, false, cx));
    assert_eq!(saved(&b, cx).kept_accounts, [(p1, Account::Microsoft)]);
    assert!(b.read_with(cx, |b, _| b.host.shares.borrow().is_empty()));

    sign_in(&b, p1, Account::Microsoft, cx);
    assert!(asks(&b, cx).is_empty(), "not asked again in that Space");
    sign_in(&b, p2, Account::Microsoft, cx);
    assert_eq!(asks(&b, cx).len(), 1, "another Space is asked");
}

#[gpui::test]
fn only_the_shown_ask_can_be_answered(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let (p1, p2) = two_profiles(&b, cx);
    sign_in(&b, p1, Account::GitHub, cx);
    sign_in(&b, p2, Account::Google, cx);
    let [(first, ..), (second, ..)] = asks(&b, cx)[..] else { panic!("two asks") };
    cx.executor().advance_clock(Duration::from_millis(600));
    b.update(cx, |b, cx| b.answer_account(second, true, cx));
    assert_eq!(asks(&b, cx).len(), 2, "the second is not shown yet");
    b.update(cx, |b, cx| b.answer_account(first, false, cx));
    // The next one starts its own wait now.
    b.update(cx, |b, cx| b.answer_account(second, true, cx));
    assert_eq!(asks(&b, cx).len(), 1);
    cx.executor().advance_clock(Duration::from_millis(600));
    b.update(cx, |b, cx| b.answer_account(second, true, cx));
    assert_eq!(saved(&b, cx).shared_accounts, [Account::Google]);
}

#[gpui::test]
fn incognito_and_unknown_profiles_are_never_asked_or_stored(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let (p1, _) = two_profiles(&b, cx);
    // No Space of this window has it (an incognito window's ephemeral profile, say).
    sign_in(&b, ProfileId(u64::MAX), Account::Google, cx);
    assert!(asks(&b, cx).is_empty());
    // An incognito window asks nothing, whatever reaches it.
    b.update(cx, |b, cx| {
        b.private = true;
        b.signed_in(vec![(p1, Account::Google)], cx);
        b.private = false;
    });
    assert!(asks(&b, cx).is_empty());
    let s = saved(&b, cx);
    assert!(s.shared_accounts.is_empty() && s.kept_accounts.is_empty());
}

#[gpui::test]
fn shared_accounts_are_handed_to_the_engine_at_launch(cx: &mut TestAppContext) {
    let store = Store::open(":memory:").unwrap();
    store.save_settings(&Settings { shared_accounts: vec![Account::GitHub], ..Default::default() }).unwrap();
    let (b, cx) = browser_with(store, cx);
    assert_eq!(b.read_with(cx, |b, _| b.host.shares.borrow().clone()), [(vec![Account::GitHub], None)]);
}
