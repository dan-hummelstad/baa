//! Mail without its views (PLAN-mail.md §4): accounts, sync, the store, MIME, sending and
//! verification codes. No gpui; the views are `baa-ui/src/mail`.

mod account;
mod backend;
mod codes;
mod compose;
mod mime;
pub mod oauth;
pub mod provider;
mod secrets;
mod send;
mod store;
mod sync;
#[cfg(feature = "test-support")]
pub mod testing;
mod thread;

use std::{
    collections::HashMap,
    ops::Range,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime},
};

use baa_model::SpaceId;
use serde::{Deserialize, Serialize};
use store::Store;

pub use account::{check_port, Account, NewAccount, NewSignIn, Protocol, Server, Servers, SignIn, Tls};
pub use backend::{
    fake::{Fake, SentMessage},
    Backend, Changes, Flags, Op, RemoteFolder, RemoteMessage,
};
pub use compose::parse_addresses;
#[cfg(target_os = "macos")]
pub use secrets::Keychain;
pub use secrets::{MemorySecrets, Secret, Secrets};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FolderId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub u64);

/// A conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ThreadId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DraftId(pub u64);

/// Why a mail call failed. The kinds are the cases the UI tells apart; `Display` is the words
/// it shows. Never holds a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The server refused the user name or password.
    SignIn,
    /// The server takes no password for this account: it wants an app password, or its own
    /// sign-in ("basic authentication is off").
    BasicAuthOff,
    /// Not one of the standard mail ports (PLAN-mail.md §9).
    Port(u16),
    /// The server offered no TLS, or TLS failed. Nothing goes on in plain text.
    NoTls,
    /// The stored sign-in no longer works; the account's mail stays.
    SignInAgain,
    /// The server could not be reached, or the connection to it was lost.
    Offline,
    /// This build has no OAuth client for the provider.
    NotSetUp,
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::SignIn => f.write_str("The server did not accept the user name or password"),
            Error::BasicAuthOff => f.write_str("The server does not take this password: use an app password, or the provider's own sign-in"),
            Error::Port(port) => write!(f, "Port {port} is not a standard mail port, and baa uses only those"),
            Error::NoTls => f.write_str("The server offered no secure connection, so baa did not connect"),
            Error::SignInAgain => f.write_str("Sign in to this account again"),
            Error::Offline => f.write_str("The mail server could not be reached"),
            Error::NotSetUp => f.write_str("Sign-in with this provider is not set up in this build"),
            Error::Other(what) => f.write_str(what),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// A Space that may have mail: any but an incognito one (PLAN-mail.md §9). It is made only
/// by [`MailSpace::of`], and every [`Mail`] call that names a Space takes one, so an
/// ephemeral Space reaches no account, no code, mail.db or the Keychain. The calls that
/// take an id instead take one that such a call returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MailSpace(SpaceId);

impl MailSpace {
    /// `None` for an ephemeral Space.
    pub fn of(space: &baa_model::Space) -> Option<MailSpace> {
        (!space.ephemeral).then_some(MailSpace(space.id))
    }

    pub fn id(self) -> SpaceId {
        self.0
    }
}

/// What a folder is for, from its server's special-use marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub id: FolderId,
    pub account: AccountId,
    pub name: String,
    pub role: Role,
    pub unread: u32,
}

/// What a list shows; [`Mail::rows`] and [`Mail::count`] read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub space: MailSpace,
    /// `None`: the inboxes of the Space's accounts as one list ("All inboxes").
    pub folder: Option<FolderId>,
    /// The scope chip: this account's mail only.
    pub account: Option<AccountId>,
    pub filter: Filter,
    /// Empty: no search.
    pub search: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Unread,
    Flagged,
}

/// One conversation in a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub thread: ThreadId,
    /// Whose dot the row carries.
    pub account: AccountId,
    /// Who it is from, as the list names them.
    pub from: String,
    pub subject: String,
    pub snippet: String,
    /// Unix seconds of its newest message.
    pub date: u64,
    pub unread: bool,
    pub flagged: bool,
    pub attachments: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    /// Empty when the message gave none.
    pub name: String,
    pub email: String,
}

/// One message of a conversation, without its content ([`Mail::body`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: MessageId,
    pub account: AccountId,
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub subject: String,
    /// Unix seconds.
    pub date: u64,
    pub unread: bool,
    pub flagged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    /// What the message's page is given: the HTML part with its `cid:` images inlined, or
    /// the text part escaped. Untrusted: it reaches Chromium only as a document
    /// (PLAN-mail.md §6).
    pub html: String,
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub has_remote_images: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// Which part of its message it is, for [`Mail::save_attachment`].
    pub part: usize,
    /// As the sender named it: untrusted, and never used as a path.
    pub name: String,
    pub mime: String,
    pub size: u64,
    /// A type the download tray warns about (an app, an installer, a script, a disk image):
    /// the UI warns the same way before it saves one.
    pub dangerous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    /// `None` until [`Mail::save_draft`] has saved it once.
    pub id: Option<DraftId>,
    pub from: AccountId,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: String,
    pub html: String,
    pub text: String,
    pub attachments: Vec<PathBuf>,
    pub in_reply_to: Option<MessageId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Read(bool),
    Flag(bool),
    Move(FolderId),
    Archive,
    Delete,
}

/// What mail showed in a Space, for the next Space switch or launch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewState {
    /// `None`: "All inboxes".
    pub folder: Option<FolderId>,
    /// The scope chip.
    pub account: Option<AccountId>,
    pub selected: Option<ThreadId>,
    /// The open draft tabs, left to right.
    pub drafts: Vec<DraftId>,
    /// The draft tab in front; `None`: the list's own tab.
    pub front: Option<DraftId>,
    /// The list's first row on screen.
    pub scroll: usize,
}

/// A verification code from a just-arrived message (PLAN-mail.md §7). No `Debug`: a code,
/// its sender or an address must not reach a log.
#[derive(Clone, PartialEq, Eq)]
pub struct Code {
    pub code: String,
    /// The sender's address.
    pub sender: String,
    pub account: AccountId,
    /// The host an origin-bound code names (`@host #code`): it is for that site only.
    pub bound_host: Option<String>,
    /// The account's own provider says DMARC failed: never offer it.
    pub dmarc_failed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    Syncing,
    Synced,
    Offline,
    /// The server refused the stored sign-in; the mail already here stays.
    SignInAgain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The account's mail changed in mail.db: read the lists again.
    Changed { account: AccountId },
    Status { account: AccountId, state: SyncState },
    Sent(DraftId),
    /// `what` is for the user. It may name a host or an address, so it is logged at debug
    /// level only; it never holds a secret.
    Failed { account: AccountId, what: String },
}

/// Makes the backend of an account, signed in with its secret from the `Secrets` given.
/// [`Mail::open`] gives the real one; a test gives a [`Fake`] through [`Mail::open_with`].
pub type Connect = Box<dyn Fn(&Account, &dyn Secrets) -> Result<Box<dyn Backend>> + Send + Sync>;

/// One a process, shared by its windows. `Send + Sync`.
pub struct Mail {
    /// `None`: in memory.
    dir: Option<PathBuf>,
    /// Made when mail.db is there or first written, so a Mail that is never used leaves no
    /// file behind.
    store: OnceLock<Store>,
    shared: Arc<sync::Shared>,
    /// The account threads started so far. A thread stops when its entry goes.
    threads: Mutex<HashMap<AccountId, sync::AccountThread>>,
    /// When each account last took a [`Mail::check_now`].
    checked: Mutex<HashMap<AccountId, Instant>>,
}

impl Mail {
    /// Starts nothing: an account's thread starts when the account is first needed.
    pub fn open(dir: &Path, secrets: Box<dyn Secrets>, notify: Box<dyn Fn(Event) + Send + Sync>) -> Result<Mail> {
        Self::open_with(Some(dir), secrets, notify, backend::imap::connector(dir.to_owned()))
    }

    /// `dir: None` keeps mail.db in memory, for tests. A mail.db written by a newer baa is an
    /// error, and is left as it is.
    pub fn open_with(
        dir: Option<&Path>,
        secrets: Box<dyn Secrets>,
        notify: Box<dyn Fn(Event) + Send + Sync>,
        connect: Connect,
    ) -> Result<Mail> {
        let mail = Mail {
            dir: dir.map(Path::to_owned),
            store: OnceLock::new(),
            shared: Arc::new(sync::Shared { connect, secrets, notify }),
            threads: Mutex::default(),
            checked: Mutex::default(),
        };
        if dir.is_none_or(|dir| dir.join("mail.db").exists()) {
            mail.store()?;
        }
        Ok(mail)
    }

    /// mail.db, made now if it is not there yet. An account thread takes
    /// [`Store::another`] of it.
    pub(crate) fn store(&self) -> Result<&Store> {
        if let Some(store) = self.store.get() {
            return Ok(store);
        }
        let _ = self.store.set(Store::open(self.dir.as_deref())?);
        Ok(self.store.get().expect("set above"))
    }

    /// A read: empty while there is no mail.db, or when it cannot be read.
    fn read<T: Default>(&self, read: impl FnOnce(&Store) -> Result<T>) -> T {
        self.store.get().and_then(|store| read(store).ok()).unwrap_or_default()
    }

    /// The Space's own accounts and those of every Space. Their threads start here, the
    /// first time a view asks for them, so mail syncs once mail is shown.
    pub fn accounts(&self, space: MailSpace) -> Vec<Account> {
        let accounts = self.read(|s| s.accounts(Some(space)));
        for account in &accounts {
            self.thread(account.id);
        }
        accounts
    }

    /// Tests the sign-in first, and stores nothing if it fails: the secret is tried from
    /// memory and reaches [`Secrets`] only once the server took it.
    pub fn add_account(&self, new: NewAccount) -> Result<AccountId> {
        new.servers.imap.check(Protocol::Imap)?;
        new.servers.smtp.check(Protocol::Smtp)?;
        let (kind, secret) = match &new.sign_in {
            NewSignIn::Password(password) => (Secret::Password, password.clone()),
            NewSignIn::Google(tokens) => (Secret::RefreshToken, tokens.refresh.clone()),
        };
        let store = self.store()?;
        let account = Account {
            id: store.next_account_id()?,
            space: new.space.map(MailSpace::id),
            name: new.name,
            address: new.address,
            codes: true,
            provider: new.provider,
            servers: new.servers,
            sign_in: match new.sign_in {
                NewSignIn::Password(_) => SignIn::Password,
                NewSignIn::Google(_) => SignIn::Google,
            },
        };
        let trial = MemorySecrets::default();
        trial.set(account.id, kind, &secret)?;
        drop((self.shared.connect)(&account, &trial)?);
        self.shared.secrets.set(account.id, kind, &secret)?;
        if let Err(error) = store.put_account(&account) {
            self.shared.secrets.delete(account.id);
            return Err(error);
        }
        self.thread(account.id);
        Ok(account.id)
    }

    /// The account with its mail, queued changes, outbox and secrets.
    pub fn remove_account(&self, id: AccountId) {
        // ponytail: the thread stops at its next step; a sync it is in the middle of may
        // still write the account's folders once. The ceiling is a check of the account row
        // inside the store's writes.
        self.threads.lock().unwrap().remove(&id);
        if let Some(store) = self.store.get() {
            let _ = store.delete_account(id);
        }
        self.shared.secrets.delete(id);
    }

    /// Removes the accounts that belong to `space` alone, with their mail and secrets, and
    /// returns them. Accounts of every Space stay.
    pub fn remove_accounts_of(&self, space: MailSpace) -> Vec<Account> {
        let own: Vec<Account> = self.read(|s| s.accounts(Some(space))).into_iter().filter(|a| a.space == Some(space.id())).collect();
        for account in &own {
            self.remove_account(account.id);
        }
        own
    }

    /// Sends `job` to the account's thread, started now if it is not running.
    fn job(&self, account: AccountId, job: sync::Job) {
        if let Some(jobs) = self.thread(account) {
            let _ = jobs.send(job);
        }
    }

    fn thread(&self, id: AccountId) -> Option<std::sync::mpsc::Sender<sync::Job>> {
        let mut threads = self.threads.lock().unwrap();
        if let Some(thread) = threads.get(&id).filter(|t| t.running()) {
            return Some(thread.jobs.clone());
        }
        let store = self.store.get()?;
        let account = store.account(id).ok()??;
        let thread = sync::AccountThread::start(account, store.another().ok()?, self.shared.clone());
        let jobs = thread.jobs.clone();
        threads.insert(id, thread);
        Some(jobs)
    }

    /// The other Spaces that have accounts of their own, with their unread counts.
    pub fn other_spaces(&self, than: MailSpace) -> Vec<(SpaceId, u32)> {
        self.read(|s| s.other_spaces(than))
    }

    pub fn folders(&self, space: MailSpace) -> Vec<Folder> {
        self.read(|s| s.folders(space))
    }

    /// One row a conversation, newest first.
    pub fn rows(&self, view: &View, range: Range<usize>) -> Vec<Row> {
        self.read(|s| s.rows(view, range))
    }

    pub fn count(&self, view: &View) -> usize {
        self.read(|s| s.count(view))
    }

    /// Oldest first.
    pub fn conversation(&self, id: ThreadId) -> Vec<Message> {
        self.read(|s| s.conversation(id))
    }

    /// `None`: not here yet. It is asked for, and [`Event::Changed`] follows.
    pub fn body(&self, id: MessageId) -> Option<Body> {
        let body = self.read(|s| s.raw(id)).map(|raw| mime::body(&raw));
        if body.is_none() {
            if let Some(at) = self.read(|s| s.locate(id)) {
                self.job(at.account, sync::Job::Body(id));
            }
        }
        body
    }

    pub fn search(&self, space: MailSpace, text: &str) -> View {
        View { space, folder: None, account: None, filter: Filter::All, search: text.into() }
    }

    /// Applied to mail.db at once, then to the server. Delete moves to Trash, and deletes for
    /// good what is in Trash already or where the account has no Trash; Archive and Move do
    /// nothing where there is no such folder of the message's account.
    pub fn act(&self, on: &[MessageId], action: Action) {
        let Some(store) = self.store.get() else { return };
        let mut woken = Vec::new();
        for &id in on {
            let Ok(Some(at)) = store.locate(id) else { continue };
            let role = |role| store.folder_by_role(at.account, role).ok().flatten().filter(|f| f.id != at.folder);
            let moved = |to: Option<store::StoredFolder>| to.map(|to| store.remove_message(id).map(|()| Op::Move(to.path)));
            let op = match action {
                Action::Read(on) => Some(store.set_seen_flagged(id, Some(on), None).map(|()| Op::Seen(on))),
                Action::Flag(on) => Some(store.set_seen_flagged(id, None, Some(on)).map(|()| Op::Flagged(on))),
                Action::Move(to) => moved(store.folder(to).ok().flatten().filter(|f| f.account == at.account && f.id != at.folder)),
                Action::Archive => moved(role(Role::Archive)),
                Action::Delete => moved(role(Role::Trash)).or_else(|| Some(store.remove_message(id).map(|()| Op::Delete))),
            };
            let queued = op.is_some_and(|op| op.and_then(|op| store.push_op(at.account, &at.path, &at.uid, &op)).is_ok());
            if queued && !woken.contains(&at.account) {
                woken.push(at.account);
            }
        }
        for account in woken {
            self.job(account, sync::Job::Wake);
        }
    }

    /// Private mode: a draft is of an account, which only a [`MailSpace`] has.
    pub fn save_draft(&self, draft: &Draft) -> Result<DraftId> {
        self.store()?.save_draft(draft)
    }

    pub fn draft(&self, id: DraftId) -> Option<Draft> {
        self.read(|s| s.draft(id))
    }

    pub fn discard_draft(&self, id: DraftId) {
        if let Some(store) = self.store.get() {
            let _ = store.discard_draft(id);
        }
    }

    /// A reply to message `to`, from the account it came to. `None` while it is not in
    /// mail.db. The quote is empty until its body is here.
    pub fn reply(&self, to: MessageId, all: bool) -> Option<Draft> {
        let (account, original) = self.original(to)?;
        let me = self.read(|s| s.account(account))?.address;
        Some(compose::reply(&original, to, account, &me, all))
    }

    pub fn forward(&self, id: MessageId) -> Option<Draft> {
        let (account, original) = self.original(id)?;
        Some(compose::forward(&original, account))
    }

    fn original(&self, id: MessageId) -> Option<(AccountId, compose::Original)> {
        let (account, e) = self.read(|s| s.envelope(id))?;
        let body = self.body(id);
        let original = compose::Original {
            from: e.from,
            reply_to: e.reply_to,
            to: e.to,
            cc: e.cc,
            subject: e.subject,
            date: e.date,
            text: body.as_ref().map(|b| b.text.clone()).unwrap_or_default(),
            html: body.map(|b| b.html).unwrap_or_default(),
        };
        Some((account, original))
    }

    /// Into the outbox, which the account's thread sends. [`Event::Sent`] or
    /// [`Event::Failed`] follows.
    pub fn send(&self, draft: DraftId) {
        let Some(saved) = self.read(|s| s.draft(draft)) else { return };
        let queued = self.store().and_then(|store| {
            let (raw, from, to) = compose::outgoing(store, &saved, SystemTime::now())?;
            store.push_outbox(saved.from, Some(draft), &from, &to, &raw)
        });
        match queued {
            Ok(_) => self.job(saved.from, sync::Job::Wake),
            Err(error) => (self.shared.notify)(Event::Failed { account: saved.from, what: error.to_string() }),
        }
    }

    /// Into `dir`, under the last component of the attachment's name; a file already there
    /// is never replaced. The file is quarantined as a download is. Returns where it is.
    pub fn save_attachment(&self, of: MessageId, part: usize, dir: &Path) -> Result<PathBuf> {
        let raw = self.read(|s| s.raw(of)).ok_or_else(|| Error::Other("The message is not downloaded yet".into()))?;
        let (name, bytes) = mime::attachment(&raw, part).ok_or_else(|| Error::Other("The message has no such attachment".into()))?;
        mime::save(dir, &mime::file_name(&name), &bytes).map_err(|e| Error::Other(format!("The attachment cannot be saved: {e}")))
    }

    pub fn view_state(&self, space: MailSpace) -> ViewState {
        self.read(|s| s.view_state(space))
    }

    /// Private mode: it takes a [`MailSpace`], which an ephemeral Space cannot make.
    pub fn set_view_state(&self, space: MailSpace, state: &ViewState) {
        if let Ok(store) = self.store() {
            let _ = store.set_view_state(space, state);
        }
    }

    /// The user chose "always for this sender": remote images of mail from `sender` load.
    pub fn trusts_images(&self, sender: &str) -> bool {
        self.read(|s| s.local(&images_key(sender))).is_some()
    }

    /// Private mode: a sender reaches this only from a message of an account, and an
    /// incognito Space has none (`MailSpace`).
    pub fn trust_images(&self, sender: &str) {
        if let Ok(store) = self.store() {
            let _ = store.set_local(&images_key(sender), "1");
        }
    }

    /// Asks the Space's accounts for one INBOX check now, at most one every 10 s an account,
    /// so a page cannot make baa hammer a server (PLAN-mail.md §7).
    pub fn check_now(&self, space: MailSpace) {
        for account in self.accounts(space) {
            {
                let mut checked = self.checked.lock().unwrap();
                if checked.get(&account.id).is_some_and(|at| at.elapsed() < Duration::from_secs(10)) {
                    continue;
                }
                checked.insert(account.id, Instant::now());
            }
            self.job(account.id, sync::Job::Check);
        }
    }

    /// The codes in mail of the Space's accounts that offer codes, from messages that came in
    /// during the last 10 minutes and at `since` or later, newest first. Which may be offered
    /// on a page is [`Code::offered_on`].
    pub fn codes(&self, space: MailSpace, since: SystemTime) -> Vec<Code> {
        let secs = |t: SystemTime| t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let after = secs(since).max(secs(SystemTime::now()) - 600);
        let offering: Vec<AccountId> = self.accounts(space).into_iter().filter(|a| a.codes).map(|a| a.id).collect();
        codes::found(self.read(|s| s.code_candidates(space, after)).into_iter().filter(|m| offering.contains(&m.account)))
    }
}

fn images_key(sender: &str) -> String {
    format!("images:{}", sender.trim().to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::{account, header, inbox, new, space, temp_dir};

    fn memory() -> Mail {
        let connect: Connect = Box::new(|_, _| Ok(Box::new(Fake::default())));
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
}
