//! Settings › Mail and adding an account (WP-K), on `MockHost` and an in-memory `Mail`.

use baa_mail::{provider::Provider, Connect, Error, Fake, Mail, MemorySecrets};
use baa_model::Store;
use baa_web::WebHost;
use gpui::{px, size, Modifiers, TestAppContext, VisualTestContext, WindowHandle};

use crate::{
    mail::setup::Setup,
    mock::MockHost,
    mail::setup::MailField,
    settings::{Field, SettingsView},
    tests_mail::{mail, mail_space, store_with_mail, B},
    Browser,
};

type Settings = WindowHandle<SettingsView<MockHost>>;

/// `crate::tests_mail::window`, with the connector given: it stands for the mail server.
fn window(cx: &mut TestAppContext, store: Store, connect: Connect) -> (B, &mut VisualTestContext) {
    cx.update(crate::init);
    cx.executor().allow_parking();
    cx.update(|cx| crate::start_mail(cx, |notify| Mail::open_with(None, Box::new(MemorySecrets::default()), notify, connect)))
        .unwrap();
    cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), store, window, cx))
}

fn accepting() -> Connect {
    Box::new(|_, _| Ok(Box::new(Fake::default())))
}

fn refusing() -> Connect {
    Box::new(|_, _| Err(Error::SignIn))
}

/// Settings › Mail through the door's entry point, in a window tall enough to click every row.
fn open_mail_settings(b: &B, vcx: &mut VisualTestContext) -> Settings {
    b.update(vcx, |b, cx| b.open_mail_settings(cx));
    vcx.run_until_parked();
    b.read_with(vcx, |b, _| b.settings_window).expect("the settings window opened")
}

fn tall(h: Settings, cx: &TestAppContext) -> VisualTestContext {
    let v = VisualTestContext::from_window(h.into(), cx);
    v.simulate_resize(size(px(900.), px(4000.)));
    v.run_until_parked();
    v
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing is drawn as {selector}"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

fn drawn(cx: &mut VisualTestContext, selector: &'static str) -> bool {
    cx.run_until_parked();
    cx.debug_bounds(selector).is_some()
}

fn type_into(cx: &mut VisualTestContext, selector: &'static str, text: &str) {
    click(cx, selector);
    cx.simulate_input(text);
}

fn setup<R>(h: Settings, cx: &mut VisualTestContext, read: impl FnOnce(&Setup) -> R) -> R {
    h.update(cx, |v, _, _| read(&v.mail_setup)).unwrap()
}

/// The form open, filled in for a Fastmail account, and the password typed.
fn filled_form(s: &mut VisualTestContext) {
    click(s, "mail-add");
    type_into(s, "mail-name", "Dan");
    type_into(s, "mail-address", "dan@fastmail.com");
    type_into(s, "mail-password", "hunter2");
}

#[gpui::test]
fn the_app_password_path_ends_in_add_account(cx: &mut TestAppContext) {
    let (b, vcx) = window(cx, store_with_mail(true), accepting());
    let h = open_mail_settings(&b, vcx);
    let mut s = tall(h, cx);
    filled_form(&mut s);
    assert_eq!(setup(h, &mut s, |f| f.provider), Provider::Fastmail, "the address chose its preset");

    click(&mut s, "mail-add-account");
    let (mail, space) = (mail(&b, &mut s), mail_space(&b, &mut s));
    let accounts = mail.accounts(space);
    assert_eq!(accounts.len(), 1);
    let account = &accounts[0];
    assert_eq!((account.name.as_str(), account.address.as_str(), account.provider), ("Dan", "dan@fastmail.com", Provider::Fastmail));
    assert_eq!(account.space, Some(space.id()));
    assert_eq!(account.servers, Provider::Fastmail.servers("dan@fastmail.com").unwrap());
    assert!(!setup(h, &mut s, |f| f.open), "the form closed");
    assert!(drawn(&mut s, "mail-account:1") && !drawn(&mut s, "mail-add-account"), "the list shows it");
}

#[gpui::test]
fn every_space_is_stored_as_no_space(cx: &mut TestAppContext) {
    let (b, vcx) = window(cx, store_with_mail(true), accepting());
    let h = open_mail_settings(&b, vcx);
    let mut s = tall(h, cx);
    filled_form(&mut s);
    click(&mut s, "mail-where:1");
    click(&mut s, "mail-add-account");
    let (mail, space) = (mail(&b, &mut s), mail_space(&b, &mut s));
    let accounts = mail.accounts(space);
    assert_eq!(accounts.len(), 1, "an account of every Space is the Space's too, listed once");
    assert_eq!(accounts[0].space, None);
    assert!(drawn(&mut s, "mail-account:1"));
}

#[gpui::test]
fn removing_an_account_asks_first(cx: &mut TestAppContext) {
    let (b, vcx) = window(cx, store_with_mail(true), accepting());
    let (mail, space) = (mail(&b, vcx), mail_space(&b, vcx));
    mail.seed_account(&baa_mail::testing::account(1, Some(space), "dan@example.com"));
    let h = open_mail_settings(&b, vcx);
    let mut s = tall(h, cx);
    assert!(drawn(&mut s, "mail-account:1"));

    click(&mut s, "mail-remove");
    assert!(drawn(&mut s, "mail-remove-yes") && drawn(&mut s, "mail-remove-cancel"));
    assert_eq!(mail.accounts(space).len(), 1, "asking removes nothing");
    click(&mut s, "mail-remove-cancel");
    assert!(drawn(&mut s, "mail-remove") && !drawn(&mut s, "mail-remove-yes"));

    click(&mut s, "mail-remove");
    click(&mut s, "mail-remove-yes");
    assert!(mail.accounts(space).is_empty());
    assert!(!drawn(&mut s, "mail-account:1"));
}

#[gpui::test]
fn a_failed_sign_in_shows_the_servers_reason(cx: &mut TestAppContext) {
    let (b, vcx) = window(cx, store_with_mail(true), refusing());
    let h = open_mail_settings(&b, vcx);
    let mut s = tall(h, cx);
    filled_form(&mut s);
    click(&mut s, "mail-add-account");

    assert_eq!(setup(h, &mut s, |f| f.error.clone()), Some(Error::SignIn.to_string()));
    assert!(drawn(&mut s, "mail-error"));
    assert!(setup(h, &mut s, |f| f.open && !f.busy), "the form stays, ready to try again");
    let space = mail_space(&b, &mut s);
    assert!(mail(&b, &mut s).accounts(space).is_empty());
}

#[gpui::test]
fn a_refused_smtp_sign_in_shows_in_the_form_and_adds_nothing(cx: &mut TestAppContext) {
    let smtp_refusing: Connect = Box::new(|_, _| {
        let server = Fake::default();
        server.refuse_send(Error::SignIn);
        Ok(Box::new(server))
    });
    let (b, vcx) = window(cx, store_with_mail(true), smtp_refusing);
    let h = open_mail_settings(&b, vcx);
    let mut s = tall(h, cx);
    filled_form(&mut s);
    click(&mut s, "mail-add-account");

    let error = setup(h, &mut s, |f| f.error.clone()).expect("the SMTP refusal shows");
    assert!(error.starts_with("Outgoing mail (SMTP)") && error.ends_with(&Error::SignIn.to_string()), "{error}");
    assert!(drawn(&mut s, "mail-error"));
    assert!(setup(h, &mut s, |f| f.open && !f.busy));
    let space = mail_space(&b, &mut s);
    assert!(mail(&b, &mut s).accounts(space).is_empty());
}

#[gpui::test]
fn google_without_a_client_says_it_is_not_set_up(cx: &mut TestAppContext) {
    let (b, vcx) = window(cx, store_with_mail(true), accepting());
    let h = open_mail_settings(&b, vcx);
    let mut s = tall(h, cx);
    click(&mut s, "mail-add");
    let tabs = b.read_with(&s, |b, _| b.host.created.borrow().len());
    click(&mut s, "mail-provider:1");
    assert!(matches!(setup(h, &mut s, |f| f.google.as_ref().map(|g| g.as_ref().err().cloned())), Some(Some(Error::NotSetUp))));

    type_into(&mut s, "mail-name", "Dan");
    type_into(&mut s, "mail-address", "dan@gmail.com");
    click(&mut s, "mail-google");
    assert_eq!(b.read_with(&s, |b, _| b.host.created.borrow().len()), tabs, "the button opens no tab");
    assert!(setup(h, &mut s, |f| !f.busy && f.error.is_none()));
}

#[gpui::test]
fn the_password_is_masked_and_never_in_an_element_id(cx: &mut TestAppContext) {
    let (b, vcx) = window(cx, store_with_mail(true), refusing());
    let h = open_mail_settings(&b, vcx);
    let mut s = tall(h, cx);
    filled_form(&mut s);
    let password = |s: &mut VisualTestContext| h.update(s, |v, _, _| (v.fields[&Field::Mail(MailField::Password)].masked, v.text(Field::Mail(MailField::Password)).to_owned())).unwrap();
    assert_eq!(password(&mut s), (true, "hunter2".to_owned()));

    click(&mut s, "mail-add-account");
    assert_eq!(password(&mut s), (true, String::new()), "the call took it out of the field, which is still masked");
}

#[gpui::test]
fn with_mail_off_or_in_a_private_window_the_tab_offers_no_accounts(cx: &mut TestAppContext) {
    let (b, vcx) = window(cx, store_with_mail(false), accepting());
    let h = open_mail_settings(&b, vcx);
    let mut s = tall(h, cx);
    assert!(drawn(&mut s, "row:mail_enabled") && drawn(&mut s, "row:mail_accounts"));
    assert!(!drawn(&mut s, "mail-add"), "no way to add an account with mail off");

    click(&mut s, "pref-mail");
    assert!(drawn(&mut s, "mail-add"), "turning mail on shows it");
    click(&mut s, "mail-add");
    assert!(drawn(&mut s, "mail-add-account"));

    b.update_in(&mut s, |b, window, cx| {
        let id = b.session.add_ephemeral_space("Incognito", b.host.ephemeral_profile());
        b.activate_space(id, window, cx);
    });
    s.run_until_parked();
    assert!(!drawn(&mut s, "mail-add-account") && !drawn(&mut s, "mail-form:provider"), "an incognito Space is offered nothing");
}

fn offline() -> Connect {
    Box::new(|_, _| Err(Error::Offline))
}

#[gpui::test]
fn an_accounts_codes_switch_is_kept_and_greys_with_the_global_one(cx: &mut TestAppContext) {
    let (b, vcx) = window(cx, store_with_mail(true), offline());
    let (mail, space) = (mail(&b, vcx), mail_space(&b, vcx));
    mail.seed_account(&baa_mail::testing::account(1, Some(space), "dan@example.com"));
    let h = open_mail_settings(&b, vcx);
    let mut s = tall(h, cx);
    click(&mut s, "mail-account-codes:1:on");
    assert!(!mail.accounts(space)[0].codes, "the switch calls the setter");

    h.update(&mut s, |_, window, _| window.remove_window()).unwrap();
    s.run_until_parked();
    let h = open_mail_settings(&b, &mut s);
    let mut s = tall(h, cx);
    assert!(drawn(&mut s, "mail-account-codes:1:off"), "reopened, the row shows what is stored");

    click(&mut s, "pref-mail-codes");
    click(&mut s, "mail-account-codes:1:off");
    assert!(!mail.accounts(space)[0].codes, "greyed with codes off in settings: a click does nothing");
}

#[gpui::test]
fn removing_an_account_takes_its_message_and_drafts_off_the_mail_view(cx: &mut TestAppContext) {
    let (b, vcx) = window(cx, store_with_mail(true), offline());
    let (mail, space) = (mail(&b, vcx), mail_space(&b, vcx));
    let folders = mail.seed_account(&baa_mail::testing::account(1, Some(space), "dan@example.com"));
    let raw = baa_mail::testing::message("ola@example.com", "dan@example.com", "Plans", crate::mail::now_secs(), "", "Hei");
    mail.seed_message(folders[0], "1", &raw, baa_mail::Flags::default(), true);
    b.update_in(vcx, |b, window, cx| {
        b.show_mail(true, window, cx);
        let view = b.mail.as_ref().unwrap().view().unwrap();
        let thread = b.mail.as_ref().unwrap().mail.rows(&view, 0..1)[0].thread;
        b.select_thread(thread, window, cx);
        b.new_message(window, cx);
        b.show_tab(baa_mail::Tab::List, cx);
        let ui = b.mail.as_ref().unwrap();
        assert!(ui.reader.is_some() && ui.drafts.len() == 1);
    });
    let h = open_mail_settings(&b, vcx);
    let mut s = tall(h, cx);
    click(&mut s, "mail-remove");
    click(&mut s, "mail-remove-yes");
    b.read_with(&s, |b, _| {
        let ui = b.mail.as_ref().unwrap();
        assert!(ui.reader.is_none() && ui.state.selected.is_none(), "its message left the reading pane");
        assert!(ui.drafts.is_empty() && ui.state.drafts.is_empty(), "its drafts' tabs closed");
    });
}

#[gpui::test]
fn the_google_sign_in_tab_leaves_no_trace(cx: &mut TestAppContext) {
    let (b, cx) = window(cx, store_with_mail(true), accepting());
    let callback = "http://127.0.0.1:5555/?state=s&code=one-time";
    b.update_in(cx, |b, window, cx| {
        let space = b.session.active_space().unwrap();
        b.session.open_tab(space, "https://example.com/", 1);
        b.session.open_tab(space, callback, 2);
        b.store().unwrap().record_visit(callback, "", 2).unwrap();
        b.drop_google_sign_in(space, "http://127.0.0.1:5555/", window, cx);
        let urls: Vec<&str> = b.session.space_tabs(space).map(|t| t.url.as_str()).collect();
        assert_eq!(urls, ["https://example.com/"]);
        assert!(b.session.archived().is_empty(), "dropped, not archived");
        assert!(b.store().unwrap().recent_history(10).unwrap().iter().all(|h| h.url != callback), "the code is not in history");
    });
}
