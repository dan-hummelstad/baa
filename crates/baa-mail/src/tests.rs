use super::*;
use crate::store::tests::{account, header, inbox, new, space, temp_dir};

/// Its accounts stay offline: these tests put mail straight into mail.db, and a sync
/// against a server that has none of it would take it out again.
fn memory() -> Mail {
    let connect: Connect = Box::new(|_, _| Err(Error::Offline));
    Mail::open_with(None, Box::new(MemorySecrets::default()), Box::new(|_| {}), connect).unwrap()
}

fn now() -> String {
    mail_parser::DateTime::from_timestamp(SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64).to_rfc822()
}

/// Delivers `raw` to account 1's inbox and keeps its body, as sync will.
fn deliver(mail: &Mail, uid: &str, raw: &[u8]) -> MessageId {
    let store = mail.store().unwrap();
    let folder = store.folder_by_role(AccountId(1), Role::Inbox).unwrap().unwrap().id;
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").map_or(raw.len(), |at| at + 4);
    let id = store.apply_changes(folder, &Changes { new: vec![new(uid, raw[..end].to_vec(), Flags::default())], ..Changes::default() }).unwrap()[0];
    store.put_body(id, raw).unwrap();
    id
}

fn with_account(space: MailSpace) -> Mail {
    let mail = memory();
    let store = mail.store().unwrap();
    store.put_account(&account(1, Some(space))).unwrap();
    inbox(store, AccountId(1));
    mail
}

#[test]
fn an_ephemeral_space_is_never_a_mail_space() {
    let mut session = baa_model::Session::first_run();
    let own = session.active_space().unwrap();
    assert_eq!(MailSpace::of(session.space(own).unwrap()).map(MailSpace::id), Some(own));
    let incognito = session.add_ephemeral_space("Incognito", baa_model::ProfileId(u64::MAX));
    assert_eq!(MailSpace::of(session.space(incognito).unwrap()), None);
}

#[test]
fn open_writes_nothing_until_something_is_kept_and_reads_are_empty_before() {
    fn shared<T: Send + Sync>() {}
    shared::<Mail>();

    let dir = temp_dir("open");
    let mail = Mail::open(&dir, Box::new(MemorySecrets::default()), Box::new(|_| panic!("nothing has happened"))).unwrap();
    assert!(!dir.exists(), "nothing on disk before anything is kept");
    let space = space(1);
    let view = mail.search(space, "invoice");
    assert_eq!(view, View { space, folder: None, account: None, filter: Filter::All, search: "invoice".into() });
    assert!(mail.accounts(space).is_empty() && mail.folders(space).is_empty() && mail.other_spaces(space).is_empty());
    assert!(mail.rows(&view, 0..50).is_empty() && mail.count(&view) == 0);
    assert!(mail.conversation(ThreadId(1)).is_empty() && mail.body(MessageId(1)).is_none());
    assert!(mail.codes(space, SystemTime::now()).is_empty() && mail.draft(DraftId(1)).is_none());
    assert!(mail.save_attachment(MessageId(1), 0, &dir).is_err() && mail.reply(MessageId(1), false).is_none());
    assert!(!dir.exists());

    let state = ViewState { scroll: 3, drafts: vec![DraftId(2)], ..ViewState::default() };
    mail.set_view_state(space, &state);
    assert!(dir.join("mail.db").exists());
    drop(mail);
    let again = Mail::open(&dir, Box::new(MemorySecrets::default()), Box::new(|_| {})).unwrap();
    assert_eq!(again.view_state(space), state, "kept across a restart");
    drop(again);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_view_state_written_before_the_preview_tab_loads_with_its_draft_in_front() {
    let old = r#"{"folder":9,"account":3,"selected":7,"drafts":[2,5],"front":5,"scroll":4}"#;
    let state: ViewState = serde_json::from_str(old).unwrap();
    assert_eq!(state.tab(), Tab::Draft(DraftId(5)));
    assert_eq!((state.selected, state.drafts.as_slice(), state.scroll), (Some(ThreadId(7)), &[DraftId(2), DraftId(5)][..], 4));
    assert_eq!(state.preview, Preview::Closed);
}

#[test]
fn the_preview_tab_stays_open_behind_another_and_folds_away_when_wide() {
    let mut state = ViewState { selected: Some(ThreadId(1)), ..ViewState::default() };
    state.set_tab(Tab::Preview);
    state.set_tab(Tab::Draft(DraftId(2)));
    assert_eq!((state.tab(), state.preview), (Tab::Draft(DraftId(2)), Preview::Open));
    state.set_tab(Tab::List);
    state.fit(false);
    assert_eq!((state.tab(), state.preview), (Tab::List, Preview::Open), "narrow keeps it");
    state.set_tab(Tab::Preview);
    state.fit(true);
    assert_eq!((state.tab(), state.preview, state.selected), (Tab::List, Preview::Closed, Some(ThreadId(1))));
}

#[test]
fn a_sender_trusted_for_images_stays_trusted_by_address_alone() {
    let mail = memory();
    assert!(!mail.trusts_images("news@shop.example"));
    mail.trust_images("News@Shop.example");
    assert!(mail.trusts_images("news@shop.example") && mail.trusts_images(" NEWS@shop.example"));
    assert!(!mail.trusts_images("other@shop.example") && !mail.trusts_images("shop.example"));

    let dir = temp_dir("images");
    let mail = Mail::open(&dir, Box::new(MemorySecrets::default()), Box::new(|_| {})).unwrap();
    assert!(!mail.trusts_images("news@shop.example"));
    assert!(!dir.exists(), "asking writes nothing");
    mail.trust_images("news@shop.example");
    drop(mail);
    let again = Mail::open(&dir, Box::new(MemorySecrets::default()), Box::new(|_| {})).unwrap();
    assert!(again.trusts_images("news@shop.example"), "kept across a restart");
    drop(again);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_account_s_codes_switch_turns_its_codes_off_and_on() {
    let home = space(1);
    // No server: a Fake one would list no folders, and a sync of that empties the inbox.
    let connect: Connect = Box::new(|_, _| Err(Error::Offline));
    let mail = Mail::open_with(None, Box::new(MemorySecrets::default()), Box::new(|_| {}), connect).unwrap();
    mail.store().unwrap().put_account(&account(1, Some(home))).unwrap();
    inbox(mail.store().unwrap(), AccountId(1));
    let raw = format!("From: no-reply@bank.example\r\nTo: dan1@example.com\r\nSubject: Sign in\r\nDate: {}\r\nMessage-ID: <c@x>\r\n\r\nYour verification code is 482913.\r\n", now());
    deliver(&mail, "1", raw.as_bytes());
    let earlier = SystemTime::now() - std::time::Duration::from_secs(60);
    assert_eq!(mail.codes(home, earlier).len(), 1);
    mail.set_codes(AccountId(1), false);
    assert!(!mail.accounts(home)[0].codes);
    assert!(mail.codes(home, earlier).is_empty());
    mail.set_codes(AccountId(1), true);
    assert_eq!(mail.codes(home, earlier).len(), 1);
    mail.set_codes(AccountId(9), false);
    assert_eq!(mail.accounts(home).len(), 1, "an unknown id is left alone");
}

#[test]
fn a_draft_is_kept_found_and_discarded() {
    let mail = with_account(space(1));
    let mut draft = Draft {
        id: None,
        from: AccountId(1),
        to: parse_addresses("Ola <ola@example.com>"),
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: "Hei".into(),
        html: "<p>Hei</p>".into(),
        text: "Hei".into(),
        attachments: vec!["/tmp/a.pdf".into()],
        in_reply_to: None,
    };
    let id = mail.save_draft(&draft).unwrap();
    draft.id = Some(id);
    assert_eq!(mail.draft(id), Some(draft.clone()));
    draft.subject = "Hei igjen".into();
    assert_eq!(mail.save_draft(&draft).unwrap(), id, "saving again keeps its id");
    assert_eq!(mail.draft(id).unwrap().subject, "Hei igjen");
    mail.discard_draft(id);
    assert_eq!(mail.draft(id), None);
}

#[test]
fn a_reply_answers_the_message_and_its_bytes_thread() {
    let mail = with_account(space(1));
    let raw = [header("root@x", "Plans", "Sun, 1 Nov 2026 10:00:00 +0000", "References: <older@x>\r\nCc: dan1@example.com, kari@example.org\r\n"), b"See you.\r\n".to_vec()].concat();
    let id = deliver(&mail, "1", &raw);
    let reply = mail.reply(id, true).unwrap();
    assert_eq!((reply.subject.as_str(), reply.in_reply_to, reply.from), ("Re: Plans", Some(id), AccountId(1)));
    let emails = |list: &[Address]| list.iter().map(|a| a.email.clone()).collect::<Vec<_>>();
    assert_eq!((emails(&reply.to), emails(&reply.cc)), (vec!["ola@example.com".to_owned(), "dan@example.com".into()], vec!["kari@example.org".to_owned()]));
    assert!(reply.text.contains("> See you."));

    let (bytes, from, to) = compose::outgoing(mail.store().unwrap(), &reply, SystemTime::now()).unwrap();
    let sent = mail_parser::MessageParser::new().parse(&bytes).unwrap();
    assert_eq!(sent.in_reply_to().and_then(|l| l.first()), Some("root@x"));
    assert_eq!(sent.references().map(|l| l.iter().collect::<Vec<_>>()), Some(vec!["older@x", "root@x"]));
    assert_eq!((from.as_str(), to.len()), ("dan1@example.com", 3));
    assert_eq!(mail.forward(id).unwrap().subject, "Fwd: Plans");
}

#[test]
fn a_saved_attachment_is_its_last_name_component_never_overwrites_and_is_quarantined() {
    let mail = with_account(space(1));
    let raw = b"From: a@example.com\r\nMessage-ID: <att@x>\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nhi\r\n\
        --b\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"../../.ssh/authorized_keys\"\r\n\r\nkey\r\n--b--\r\n";
    let id = deliver(&mail, "1", raw);
    let body = mail.body(id).unwrap();
    assert_eq!(body.attachments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["../../.ssh/authorized_keys"]);

    let dir = temp_dir("attachment");
    std::fs::create_dir_all(&dir).unwrap();
    let first = mail.save_attachment(id, 0, &dir).unwrap();
    assert_eq!(first, dir.join("authorized_keys"));
    let second = mail.save_attachment(id, 0, &dir).unwrap();
    assert_eq!(second, dir.join("authorized_keys (2)"));
    assert_eq!(std::fs::read(&first).unwrap(), b"key");
    let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names.len(), 2, "nothing written outside, nothing replaced");
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/bin/xattr").arg("-p").arg("com.apple.quarantine").arg(&first).output().unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains(";baa;"), "quarantined");
    }
    assert!(mail.save_attachment(id, 5, &dir).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_forward_carries_the_attachments_as_private_copies_until_no_draft_holds_them() {
    use std::os::unix::fs::PermissionsExt;
    let raw = b"From: a@example.com\r\nMessage-ID: <fwd@x>\r\nSubject: Files\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nhi\r\n\
        --b\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"../../.ssh/report.pdf\"\r\n\r\npdf\r\n\
        --b\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\n\r\nmore\r\n--b--\r\n";
    let in_memory = with_account(space(1));
    let id = deliver(&in_memory, "1", raw);
    assert!(in_memory.forward(id).unwrap().attachments.is_empty(), "nowhere to write in memory");

    let dir = temp_dir("forward");
    let offline: Connect = Box::new(|_, _| Err(Error::Offline));
    let mail = Mail::open_with(Some(&dir), Box::new(MemorySecrets::default()), Box::new(|_| {}), offline).unwrap();
    mail.store().unwrap().put_account(&account(1, Some(space(1)))).unwrap();
    inbox(mail.store().unwrap(), AccountId(1));
    let id = deliver(&mail, "1", raw);

    let draft = mail.forward(id).unwrap();
    let forwards = dir.join("forwards");
    let folder = draft.attachments[0].parent().unwrap().to_owned();
    assert_eq!(folder.parent(), Some(forwards.as_path()));
    assert_eq!(draft.attachments, [folder.join("report.pdf"), folder.join("report (2).pdf")]);
    assert_eq!(std::fs::read(&draft.attachments[1]).unwrap(), b"more");
    assert_eq!(std::fs::metadata(&folder).unwrap().permissions().mode() & 0o777, 0o700);

    let (bytes, _, _) = compose::outgoing(mail.store().unwrap(), &Draft { to: parse_addresses("o@example.com"), ..draft.clone() }, SystemTime::now()).unwrap();
    let sent = mail_parser::MessageParser::new().parse(&bytes).unwrap();
    let names: Vec<_> = sent.attachments().map(|a| a.attachment_name().unwrap().to_owned()).collect();
    assert_eq!(names, ["report.pdf", "report (2).pdf"]);

    mail.save_draft(&draft).unwrap();
    let unsaved = mail.forward(id).unwrap().attachments[0].parent().unwrap().to_owned();
    assert_ne!(unsaved, folder);
    drop(mail);
    let again = Mail::open_with(Some(&dir), Box::new(MemorySecrets::default()), Box::new(|_| {}), Box::new(|_, _| Err(Error::Offline))).unwrap();
    assert!(folder.exists(), "a saved draft attaches it");
    assert!(!unsaved.exists(), "no draft attaches it");
    drop(again);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn hostile_attachment_names_come_out_as_their_last_component() {
    for (sender, saved) in [
        ("../../.ssh/authorized_keys", "authorized_keys"),
        ("..\\..\\evil.bat", "evil.bat"),
        ("nul\0.txt", "nul.txt"),
        (".hidden", "hidden"),
        ("..", "attachment"),
        ("a/b/", "attachment"),
        ("report:2026.pdf", "report_2026.pdf"),
        ("", "attachment"),
    ] {
        assert_eq!(mime::file_name(sender), saved, "{sender:?}");
    }
    assert!(mime::file_name(&"é".repeat(300)).len() <= 200);
}

#[test]
fn codes_come_from_new_mail_of_the_spaces_accounts_and_the_origin_bound_line_wins() {
    let home = space(1);
    let mail = with_account(home);
    let date = now();
    let bound = format!("From: Bank <no-reply@bank.example>\r\nSubject: Sign in\r\nDate: {date}\r\nMessage-ID: <b@x>\r\n\r\nYour verification code is 111111.\r\n\r\n@login.bank.example #222222\r\n");
    let header_code = format!("From: shop@shop.example\r\nSubject: Hi\r\nDate: {date}\r\nOTP-Token: 333333\r\nMessage-ID: <s@x>\r\n\r\nYour verification code is 444444.\r\n");
    let old = "From: old@old.example\r\nSubject: Your verification code is 555555\r\nDate: Sun, 1 Nov 2026 10:00:00 +0000\r\nMessage-ID: <o@x>\r\n\r\n.\r\n";
    let no_code = format!("From: ola@example.com\r\nSubject: Lunch\r\nDate: {date}\r\nMessage-ID: <l@x>\r\n\r\nSee you in 2026.\r\n");
    deliver(&mail, "1", bound.as_bytes());
    deliver(&mail, "2", header_code.as_bytes());
    deliver(&mail, "3", old.as_bytes());
    deliver(&mail, "4", no_code.as_bytes());

    let earlier = SystemTime::now() - std::time::Duration::from_secs(60);
    let codes = mail.codes(home, earlier);
    let got: Vec<(&str, &str, Option<&str>)> = codes.iter().map(|c| (c.code.as_str(), c.sender.as_str(), c.bound_host.as_deref())).collect();
    assert_eq!(got.len(), 2);
    assert!(got.contains(&("222222", "no-reply@bank.example", Some("login.bank.example"))));
    assert!(got.contains(&("333333", "shop@shop.example", None)));
    assert!(codes.iter().all(|c| c.account == AccountId(1) && !c.dmarc_failed));
    assert!(mail.codes(home, SystemTime::now() + std::time::Duration::from_secs(60)).is_empty(), "nothing before `since`");
    assert!(mail.codes(space(2), earlier).is_empty(), "another Space's account");

    let store = mail.store().unwrap();
    store.put_account(&Account { codes: false, ..account(1, Some(home)) }).unwrap();
    assert!(mail.codes(home, earlier).is_empty(), "the account's switch is off");
}

#[test]
fn a_code_is_offered_only_where_its_sender_or_bound_host_matches_and_never_after_a_dmarc_failure() {
    // The engine's registrable domain, for these hosts.
    let site_of = |host: &str| host.rsplitn(3, '.').take(2).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(".");
    let code = |sender: &str, bound: Option<&str>, dmarc_failed| Code {
        code: "123456".into(),
        sender: sender.into(),
        account: AccountId(1),
        bound_host: bound.map(str::to_owned),
        dmarc_failed,
    };
    assert!(code("no-reply@accounts.bank.example", None, false).offered_on("bank.example", &site_of));
    assert!(!code("no-reply@bank-example.evil", None, false).offered_on("bank.example", &site_of), "another site");
    assert!(!code("no-reply@bank.example.evil.example", None, false).offered_on("bank.example", &site_of));
    assert!(code("account-security-noreply@accountprotection.microsoft.com", None, false).offered_on("live.com", &site_of), "an alias");
    assert!(!code("x@live.com", None, false).offered_on("microsoft.com", &site_of), "an alias goes one way");
    assert!(!code("no-reply@bank.example", None, true).offered_on("bank.example", &site_of), "DMARC failed");
    assert!(code("x@mailer.example", Some("login.bank.example"), false).offered_on("bank.example", &site_of), "the bound host decides");
    assert!(!code("x@bank.example", Some("other.example"), false).offered_on("bank.example", &site_of));
    assert!(!code("not an address", None, false).offered_on("bank.example", &site_of));
    assert!(!code("x@", None, false).offered_on("", &site_of));
}

#[test]
fn the_corpus_under_mutation_never_panics_the_code_path() {
    for (_, raw) in mime::corpus() {
        for seed in 0..200 {
            let raw = mime::mutate(&raw, seed);
            let text = mime::plain_text(&raw);
            let token = mime::header_text(&raw, "OTP-Token");
            let e = mime::envelope(&raw);
            let _ = codes::extract(&e.subject, &text, token.as_deref());
        }
    }
}
