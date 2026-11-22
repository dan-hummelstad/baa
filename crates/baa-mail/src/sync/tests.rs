use std::{
    collections::{HashSet, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst},
        mpsc::{channel, Receiver},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use crate::{
    parse_addresses,
    provider::Provider,
    store::tests::{account, header, space},
    Account, AccountId, Action, Backend, Changes, Connect, Draft, Error, Event, Fake, Filter, Flags, MailSpace, MemorySecrets, MessageId,
    NewAccount, NewSignIn, Op, RemoteFolder, Result, Role, Secret, Secrets, SyncState, View,
};

/// Well in the past, so this mail is outside the window sync fetches bodies for (codes).
const DATE: &str = "Mon, 02 Nov 2026 10:00:00 +0000";

/// What the test can look at: the server, whether it can be reached, and what was asked of it.
struct Net {
    fake: Fake,
    up: AtomicBool,
    connects: AtomicUsize,
    /// The next connections fail with these, then connecting works while `up`.
    refusals: Mutex<VecDeque<Error>>,
    send_error: Mutex<Option<Error>>,
    inbox_asked: AtomicUsize,
    archive_asked: AtomicUsize,
}

impl Net {
    fn new() -> Arc<Net> {
        let fake = Fake::default();
        for (path, role) in [("INBOX", Role::Inbox), ("Sent", Role::Sent), ("Trash", Role::Trash), ("Archive", Role::Archive)] {
            fake.add_folder(path, role);
        }
        Arc::new(Net {
            fake,
            up: AtomicBool::new(true),
            connects: AtomicUsize::new(0),
            refusals: Mutex::default(),
            send_error: Mutex::default(),
            inbox_asked: AtomicUsize::new(0),
            archive_asked: AtomicUsize::new(0),
        })
    }

    fn connector(self: &Arc<Self>) -> Connect {
        let net = self.clone();
        Box::new(move |_, _| {
            net.connects.fetch_add(1, SeqCst);
            if let Some(error) = net.refusals.lock().unwrap().pop_front() {
                return Err(error);
            }
            if !net.up.load(SeqCst) {
                return Err(Error::Offline);
            }
            Ok(Box::new(Link { net: net.clone(), fake: net.fake.clone() }))
        })
    }

    fn deliver(&self, subject: &str) -> String {
        self.fake.deliver("INBOX", &raw(subject), Flags::default())
    }

    /// Subject and flags of what the server holds in `folder`.
    fn held(&self, folder: &str) -> Vec<(String, Flags)> {
        let new = self.fake.clone().changes(folder, None).unwrap().new;
        new.into_iter()
            .map(|m| {
                let header = String::from_utf8_lossy(&m.header).into_owned();
                let subject = header.lines().find_map(|l| l.strip_prefix("Subject: ")).unwrap().to_owned();
                (subject, m.flags)
            })
            .collect()
    }

    fn subjects(&self, folder: &str) -> Vec<String> {
        self.held(folder).into_iter().map(|(s, _)| s).collect()
    }
}

/// The fake server behind a connection that can drop.
struct Link {
    net: Arc<Net>,
    fake: Fake,
}

impl Link {
    fn up(&self) -> Result<()> {
        if self.net.up.load(SeqCst) {
            Ok(())
        } else {
            Err(Error::Offline)
        }
    }
}

impl Backend for Link {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>> {
        self.up()?;
        self.fake.folders()
    }

    fn changes(&mut self, folder: &str, cursor: Option<&str>) -> Result<Changes> {
        self.up()?;
        match folder {
            "INBOX" => self.net.inbox_asked.fetch_add(1, SeqCst),
            "Archive" => self.net.archive_asked.fetch_add(1, SeqCst),
            _ => 0,
        };
        self.fake.changes(folder, cursor)
    }

    fn fetch_body(&mut self, folder: &str, uid: &str) -> Result<Vec<u8>> {
        self.up()?;
        self.fake.fetch_body(folder, uid)
    }

    fn apply(&mut self, folder: &str, uid: &str, op: &Op) -> Result<()> {
        self.up()?;
        self.fake.apply(folder, uid, op)
    }

    fn wait(&mut self, folder: &str, timeout: Duration) -> Result<bool> {
        self.up()?;
        self.fake.wait(folder, timeout)
    }

    fn append(&mut self, folder: &str, raw: &[u8], flags: Flags) -> Result<()> {
        self.up()?;
        self.fake.append(folder, raw, flags)
    }

    fn send(&mut self, from: &str, to: &[String], raw: &[u8]) -> Result<()> {
        self.up()?;
        if let Some(error) = self.net.send_error.lock().unwrap().clone() {
            return Err(error);
        }
        self.fake.send(from, to, raw)
    }
}

/// The test keeps a handle on the secrets the `Mail` owns.
struct Kept(Arc<MemorySecrets>);

impl Secrets for Kept {
    fn get(&self, account: AccountId, kind: Secret) -> Option<String> {
        self.0.get(account, kind)
    }

    fn set(&self, account: AccountId, kind: Secret, value: &str) -> Result<()> {
        self.0.set(account, kind, value)
    }

    fn delete(&self, account: AccountId) {
        self.0.delete(account)
    }
}

struct Rig {
    mail: crate::Mail,
    rx: Receiver<Event>,
    secrets: Arc<MemorySecrets>,
    net: Arc<Net>,
    space: MailSpace,
}

impl Rig {
    fn new(net: Arc<Net>, connect: Connect) -> Rig {
        let (tx, rx) = channel();
        let secrets = Arc::<MemorySecrets>::default();
        let notify = Box::new(move |event| {
            let _ = tx.send(event);
        });
        let mail = crate::Mail::open_with(None, Box::new(Kept(secrets.clone())), notify, connect).unwrap();
        Rig { mail, rx, secrets, net, space: space(1) }
    }

    fn standard() -> Rig {
        let net = Net::new();
        Rig::new(net.clone(), net.connector())
    }

    /// Keeps `account` in mail.db and starts its thread.
    fn start(&self, account: Account) {
        self.mail.store().unwrap().put_account(&account).unwrap();
        self.mail.accounts(self.space);
    }

    fn start_default(&self) {
        self.start(account(1, Some(self.space)));
    }

    fn synced(&self) {
        until(&self.rx, status(SyncState::Synced));
    }

    fn inboxes(&self) -> View {
        View { space: self.space, folder: None, account: None, filter: Filter::All, search: String::new() }
    }

    fn folder(&self, role: Role) -> crate::FolderId {
        self.mail.folders(self.space).into_iter().find(|f| f.role == role).unwrap().id
    }

    fn message(&self, role: Role, subject: &str) -> MessageId {
        let view = View { folder: Some(self.folder(role)), ..self.inboxes() };
        let row = self.mail.rows(&view, 0..50).into_iter().find(|r| r.subject == subject).unwrap();
        self.mail.conversation(row.thread)[0].id
    }
}

fn raw(subject: &str) -> Vec<u8> {
    [header(&format!("{subject}@x"), subject, DATE, ""), format!("Text of {subject}\r\n").into_bytes()].concat()
}

fn until(rx: &Receiver<Event>, mut pred: impl FnMut(&Event) -> bool) -> Event {
    let end = Instant::now() + Duration::from_secs(5);
    let mut seen = Vec::new();
    loop {
        match rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
            Ok(event) if pred(&event) => return event,
            Ok(event) => seen.push(event),
            Err(_) => panic!("no such event in 5 s; saw {seen:?}"),
        }
    }
}

fn eventually(what: &str, mut holds: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(2);
    while !holds() {
        assert!(Instant::now() < end, "not within 2 s: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn status(state: SyncState) -> impl FnMut(&Event) -> bool {
    move |e| matches!(e, Event::Status { state: s, .. } if *s == state)
}

fn changed(e: &Event) -> bool {
    matches!(e, Event::Changed { .. })
}

fn draft(from: AccountId) -> Draft {
    Draft {
        id: None,
        from,
        to: parse_addresses("Ola <ola@example.com>"),
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: "Hei".into(),
        html: "<p>Hei</p>".into(),
        text: "Hei".into(),
        attachments: Vec::new(),
        in_reply_to: None,
    }
}

fn new_account(space: Option<MailSpace>, address: &str, password: &str) -> NewAccount {
    NewAccount {
        space,
        name: "Dan".into(),
        address: address.into(),
        provider: Provider::Fastmail,
        servers: Provider::Fastmail.servers(address).unwrap(),
        sign_in: NewSignIn::Password(password.into()),
    }
}

#[test]
fn a_first_sync_fills_the_store_and_says_so() {
    let rig = Rig::standard();
    rig.net.deliver("one");
    rig.net.deliver("two");
    rig.start_default();
    rig.synced();
    assert_eq!(rig.mail.count(&rig.inboxes()), 2);
    let mut roles: Vec<_> = rig.mail.folders(rig.space).into_iter().map(|f| f.role as u8).collect();
    roles.sort();
    let mut want = [Role::Inbox, Role::Sent, Role::Trash, Role::Archive].map(|r| r as u8);
    want.sort();
    assert_eq!(roles, want);
}

#[test]
fn new_mail_during_idle_raises_changed() {
    let rig = Rig::standard();
    rig.net.deliver("one");
    rig.net.deliver("two");
    rig.start_default();
    rig.synced();
    rig.net.deliver("three");
    until(&rig.rx, changed);
    assert_eq!(rig.mail.count(&rig.inboxes()), 3);
}

#[test]
fn a_body_is_fetched_on_demand() {
    let rig = Rig::standard();
    rig.net.deliver("one");
    rig.start_default();
    rig.synced();
    let id = rig.message(Role::Inbox, "one");
    assert!(rig.mail.body(id).is_none());
    until(&rig.rx, changed);
    assert!(rig.mail.body(id).unwrap().text.contains("Text of one"));
}

fn code_mail(subject: &str, date: &str) -> Vec<u8> {
    [header(&format!("{subject}@x"), subject, date, ""), b"Your verification code is 482913.\r\n".to_vec()].concat()
}

fn codes_found(rig: &Rig) -> Vec<String> {
    let since = std::time::SystemTime::now() - Duration::from_secs(60);
    rig.mail.codes(rig.space, since).into_iter().map(|c| c.code).collect()
}

#[test]
fn a_code_in_the_body_of_new_mail_is_found_without_anyone_opening_it() {
    let now = mail_parser::DateTime::from_timestamp(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64);
    // In an account's first sync: the user focused a code field before ever opening mail.
    let rig = Rig::standard();
    rig.net.fake.deliver("INBOX", &code_mail("Sign in", &now.to_rfc822()), Flags::default());
    rig.start_default();
    rig.synced();
    assert_eq!(codes_found(&rig), ["482913"]);

    let rig = Rig::standard();
    rig.start_default();
    rig.synced();
    assert!(codes_found(&rig).is_empty());
    rig.net.fake.deliver("INBOX", &code_mail("Sign in", &now.to_rfc822()), Flags::default());
    until(&rig.rx, changed);
    assert_eq!(codes_found(&rig), ["482913"]);
}

#[test]
fn a_sync_does_not_fetch_the_bodies_of_older_mail() {
    let rig = Rig::standard();
    rig.net.fake.deliver("INBOX", &code_mail("Old", DATE), Flags::default());
    rig.start_default();
    rig.synced();
    let id = rig.message(Role::Inbox, "Old");
    assert!(rig.mail.store().unwrap().raw(id).unwrap().is_none());
}

#[test]
fn actions_reach_the_server() {
    let rig = Rig::standard();
    for subject in ["read", "archive", "delete"] {
        rig.net.deliver(subject);
    }
    rig.net.fake.deliver("Trash", &raw("trashed"), Flags::default());
    rig.start_default();
    rig.synced();

    let ids = [(Role::Inbox, "read"), (Role::Inbox, "archive"), (Role::Inbox, "delete"), (Role::Trash, "trashed")].map(|(r, s)| rig.message(r, s));
    rig.mail.act(&ids[0..1], Action::Read(true));
    rig.mail.act(&ids[1..2], Action::Archive);
    rig.mail.act(&ids[2..3], Action::Delete);
    rig.mail.act(&ids[3..4], Action::Delete);

    let seen = Flags { seen: true, ..Flags::default() };
    eventually("the server heard every action", || {
        rig.net.held("INBOX").iter().map(|(s, f)| (s.as_str(), *f)).collect::<Vec<_>>() == [("read", seen)]
            && rig.net.subjects("Archive") == ["archive"]
            && rig.net.subjects("Trash") == ["delete"]
    });
}

#[test]
fn a_move_made_offline_is_replayed_once_after_reconnecting() {
    let rig = Rig::standard();
    rig.net.deliver("moved");
    rig.net.deliver("stays");
    rig.start_default();
    rig.synced();

    rig.net.up.store(false, SeqCst);
    until(&rig.rx, status(SyncState::Offline));
    let id = rig.message(Role::Inbox, "moved");
    rig.mail.act(&[id], Action::Move(rig.folder(Role::Archive)));
    assert_eq!(rig.mail.count(&rig.inboxes()), 1, "mail.db changed at once");
    assert_eq!(rig.net.subjects("INBOX"), ["moved", "stays"], "the server has not heard");

    rig.net.up.store(true, SeqCst);
    rig.synced();
    rig.net.deliver("later");
    until(&rig.rx, changed);
    assert_eq!(rig.net.subjects("Archive"), ["moved"]);
    assert_eq!(rig.net.subjects("INBOX"), ["stays", "later"]);
    assert!(rig.mail.store().unwrap().pending_ops(AccountId(1)).unwrap().is_empty());
    assert_eq!(rig.mail.count(&rig.inboxes()), 2);
}

#[test]
fn a_dropped_connection_reconnects_with_back_off() {
    let rig = Rig::standard();
    rig.net.refusals.lock().unwrap().extend([Error::Offline, Error::Offline, Error::Offline]);
    rig.start_default();
    let mut offline = 0;
    until(&rig.rx, |e| {
        offline += status(SyncState::Offline)(e) as usize;
        status(SyncState::Synced)(e)
    });
    assert_eq!((offline, rig.net.connects.load(SeqCst)), (3, 4));
}

#[test]
fn a_refused_sign_in_asks_to_sign_in_again_and_keeps_the_mail() {
    let rig = Rig::standard();
    rig.net.deliver("one");
    rig.start_default();
    rig.synced();

    rig.net.refusals.lock().unwrap().push_back(Error::SignInAgain);
    rig.net.up.store(false, SeqCst);
    until(&rig.rx, status(SyncState::SignInAgain));
    let tried = rig.net.connects.load(SeqCst);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(rig.net.connects.load(SeqCst), tried, "it waits for the user");
    assert_eq!(rig.mail.count(&rig.inboxes()), 1);

    rig.net.up.store(true, SeqCst);
    rig.mail.check_now(rig.space);
    rig.synced();
    assert_eq!(rig.net.connects.load(SeqCst), tried + 1);
}

fn sent_through(provider: Provider) -> (Vec<crate::SentMessage>, usize) {
    let rig = Rig::standard();
    let address = "dan1@example.com";
    rig.start(Account { provider, servers: provider.servers(address).unwrap(), ..account(1, Some(rig.space)) });
    rig.synced();
    let id = rig.mail.save_draft(&draft(AccountId(1))).unwrap();
    rig.mail.send(id);
    assert_eq!(until(&rig.rx, |e| matches!(e, Event::Sent(_))), Event::Sent(id));
    (rig.net.fake.sent(), rig.net.subjects("Sent").len())
}

#[test]
fn a_sent_message_goes_out_and_is_filed_in_sent_but_not_on_gmail() {
    let (sent, filed) = sent_through(Provider::Fastmail);
    assert_eq!((sent.len(), filed), (1, 1));
    assert_eq!(sent[0].from, "dan1@example.com");
    assert_eq!(sent[0].to, ["ola@example.com"]);

    let (sent, filed) = sent_through(Provider::Gmail);
    assert_eq!((sent.len(), filed), (1, 0), "Gmail files it itself");
    assert_eq!(sent[0].to, ["ola@example.com"]);
}

#[test]
fn a_failed_send_stays_in_the_outbox_and_raises_failed() {
    let rig = Rig::standard();
    *rig.net.send_error.lock().unwrap() = Some(Error::Other("550 no such user".into()));
    rig.start_default();
    rig.synced();
    let id = rig.mail.save_draft(&draft(AccountId(1))).unwrap();
    rig.mail.send(id);
    let Event::Failed { what, .. } = until(&rig.rx, |e| matches!(e, Event::Failed { .. })) else { unreachable!() };
    assert!(what.contains("550 no such user"), "{what}");
    std::thread::sleep(Duration::from_millis(100));
    assert!(!rig.rx.try_iter().any(|e| matches!(e, Event::Sent(_))));
    assert!(rig.mail.draft(id).is_some());
    assert_eq!(rig.mail.store().unwrap().outbox(AccountId(1)).unwrap().len(), 1);
}

#[test]
fn add_account_tries_the_sign_in_before_anything_is_kept() {
    let refused = Rig::new(Net::new(), Box::new(|_, _| Err(Error::SignIn)));
    assert_eq!(refused.mail.add_account(new_account(Some(refused.space), "dan@example.com", "wrong")), Err(Error::SignIn));
    assert!(refused.mail.accounts(refused.space).is_empty());
    for id in 1..4 {
        let held = (refused.secrets.get(AccountId(id), Secret::Password), refused.secrets.get(AccountId(id), Secret::RefreshToken));
        assert_eq!(held, (None, None));
    }

    let net = Net::new();
    let connect = net.connector();
    let checking: Connect = Box::new(move |account, secrets| match secrets.get(account.id, Secret::Password).as_deref() {
        Some("hunter2") => connect(account, secrets),
        _ => Err(Error::Other("the trial had no password".into())),
    });
    let ok = Rig::new(net, checking);
    let id = ok.mail.add_account(new_account(Some(ok.space), "dan@example.com", "hunter2")).unwrap();
    assert_eq!(ok.mail.accounts(ok.space).iter().map(|a| a.id).collect::<Vec<_>>(), [id]);
    assert_eq!(ok.secrets.get(id, Secret::Password).as_deref(), Some("hunter2"));

    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let never = Rig::new(Net::new(), Box::new(move |_, _| Err(Error::Other(counted.fetch_add(1, SeqCst).to_string()))));
    let mut new = new_account(Some(never.space), "dan@example.com", "x");
    new.servers.smtp.port = 25;
    assert_eq!(never.mail.add_account(new), Err(Error::Port(25)));
    assert_eq!(calls.load(SeqCst), 0);
}

#[test]
fn removing_an_account_removes_its_secrets_and_rows_and_a_space_keeps_shared_accounts() {
    let rig = Rig::standard();
    rig.net.deliver("one");
    let own = rig.mail.add_account(new_account(Some(rig.space), "own@example.com", "own-secret")).unwrap();
    let shared = rig.mail.add_account(new_account(None, "shared@example.com", "shared-secret")).unwrap();
    let mut done = HashSet::new();
    until(&rig.rx, |e| matches!(e, Event::Status { account, state: SyncState::Synced } if done.insert(*account)) && done.len() == 2);
    let of = |account| View { account: Some(account), ..rig.inboxes() };
    assert_eq!((rig.mail.count(&of(own)), rig.mail.count(&of(shared))), (1, 1));

    let removed = rig.mail.remove_accounts_of(rig.space);
    assert_eq!(removed.iter().map(|a| a.id).collect::<Vec<_>>(), [own]);
    assert_eq!(rig.secrets.get(own, Secret::Password), None);
    assert_eq!(rig.mail.accounts(rig.space).iter().map(|a| a.id).collect::<Vec<_>>(), [shared]);
    assert_eq!((rig.mail.count(&of(own)), rig.mail.count(&of(shared))), (0, 1));
    assert_eq!(rig.secrets.get(shared, Secret::Password).as_deref(), Some("shared-secret"));

    rig.mail.remove_account(shared);
    assert!(rig.mail.accounts(rig.space).is_empty());
    assert_eq!(rig.secrets.get(shared, Secret::Password), None);
    assert_eq!(rig.mail.count(&of(shared)), 0);
}

#[test]
fn check_now_asks_each_account_once_every_ten_seconds() {
    let rig = Rig::standard();
    rig.start_default();
    rig.synced();
    let before = rig.net.inbox_asked.load(SeqCst);
    rig.mail.check_now(rig.space);
    rig.mail.check_now(rig.space);
    eventually("one more INBOX check", || rig.net.inbox_asked.load(SeqCst) > before);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(rig.net.inbox_asked.load(SeqCst), before + 1);
}

#[test]
fn an_opened_folder_syncs_at_once_and_at_most_once_in_ten_seconds() {
    let rig = Rig::standard();
    rig.start_default();
    rig.synced();
    let archive = rig.folder(Role::Archive);
    let view = View { folder: Some(archive), ..rig.inboxes() };
    rig.net.fake.deliver("Archive", &raw("filed"), Flags::default());
    let before = rig.net.archive_asked.load(SeqCst);

    rig.mail.check_folder(archive);
    rig.mail.check_folder(archive);
    until(&rig.rx, changed);
    eventually("the archived message in mail.db", || rig.mail.count(&view) == 1);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(rig.net.archive_asked.load(SeqCst), before + 1, "two calls within 10 s, one sync");

    rig.mail.check_folder(crate::FolderId(9999));
}
