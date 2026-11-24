//! Mail without its views (PLAN-mail.md §4): accounts, sync, the store, MIME, sending and
//! verification codes. No gpui; the views are `baa-ui/src/mail`.

mod account;
mod backend;
mod categories;
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
    sync::{atomic::AtomicBool, Arc, Mutex, OnceLock},
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
pub use categories::{model_status, Category, ModelStatus};
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
    /// The inbox's pills (PLAN-mail.md §17); `All` is All Mail.
    Category(Category),
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
    /// Its newest message's.
    pub category: Category,
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
    /// The draft tab in front; `None`: the list's tab or the preview's. Kept as it was so a
    /// build from before the preview tab reads this row.
    pub front: Option<DraftId>,
    /// The preview tab, which shows `selected` in a narrow mail view.
    pub preview: Preview,
    /// The list's first row on screen.
    pub scroll: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preview {
    #[default]
    Closed,
    Open,
    InFront,
}

/// The mail view's tab in front: the list's (first, always there), the preview's, or a draft's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    List,
    Preview,
    Draft(DraftId),
}

impl ViewState {
    pub fn tab(&self) -> Tab {
        match (self.front, self.preview) {
            (Some(draft), _) => Tab::Draft(draft),
            (None, Preview::InFront) => Tab::Preview,
            (None, _) => Tab::List,
        }
    }

    /// Puts `tab` in front; a preview tab put behind stays open.
    pub fn set_tab(&mut self, tab: Tab) {
        self.front = match tab {
            Tab::Draft(draft) => Some(draft),
            _ => None,
        };
        self.preview = match (tab, self.preview) {
            (Tab::Preview, _) => Preview::InFront,
            (_, Preview::Closed) => Preview::Closed,
            _ => Preview::Open,
        };
    }

    pub fn close_preview(&mut self) {
        if self.tab() == Tab::Preview {
            self.set_tab(Tab::List);
        }
        self.preview = Preview::Closed;
    }

    /// A wide mail view has no preview tab: its conversation shows in the reading pane beside
    /// the list.
    pub fn fit(&mut self, wide: bool) {
        if wide {
            self.close_preview();
        }
    }
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
    /// When each account last took a [`Mail::check_now`], and each folder a
    /// [`Mail::check_folder`].
    checked: Mutex<HashMap<AccountId, Instant>>,
    folders_checked: Mutex<HashMap<FolderId, Instant>>,
}

impl Mail {
    /// Starts nothing: an account's thread starts when the account is first needed.
    pub fn open(dir: &Path, secrets: Box<dyn Secrets>, notify: Box<dyn Fn(Event) + Send + Sync>) -> Result<Mail> {
        let mut mail = Self::open_with(Some(dir), secrets, notify, backend::imap::connector(dir.to_owned()))?;
        if let Some(shared) = Arc::get_mut(&mut mail.shared) {
            shared.model = categories::apple_model();
        }
        Ok(mail)
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
            shared: Arc::new(sync::Shared {
                connect,
                secrets,
                notify,
                model: None,
                categorizer: OnceLock::new(),
                categories: AtomicBool::new(true),
            }),
            threads: Mutex::default(),
            checked: Mutex::default(),
            folders_checked: Mutex::default(),
        };
        if dir.is_none_or(|dir| dir.join("mail.db").exists()) {
            let store = mail.store()?;
            if let Some(dir) = dir {
                sweep_forwards(&dir.join(FORWARDS), store);
            }
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

    /// Tests the sign-in first, at the IMAP server and then the SMTP server (which is sent
    /// nothing), and stores nothing if either fails: the secret is tried from memory and
    /// reaches [`Secrets`] only once both servers took it.
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
        let mut server = (self.shared.connect)(&account, &trial)?;
        server.check_send().map_err(|e| Error::Other(format!("Outgoing mail (SMTP): {e}")))?;
        drop(server);
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

    pub fn unread(&self, view: &View) -> usize {
        self.read(|s| s.unread(view))
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

    /// With the original's attachments when its body is here.
    pub fn forward(&self, id: MessageId) -> Option<Draft> {
        let (account, original) = self.original(id)?;
        let mut draft = compose::forward(&original, account);
        draft.attachments = self.copy_attachments(id);
        Some(draft)
    }

    /// The message's attachments as files baa writes itself, for a draft to attach: in a new
    /// 0700 folder under `forwards`, each under its name's last component and quarantined,
    /// as a saved attachment is. Empty in memory, before the body is here, or if writing
    /// fails.
    // ponytail: the copies stay until the next launch, which removes those no saved draft
    // attaches; the ceiling is removing them when their draft is sent or discarded.
    fn copy_attachments(&self, id: MessageId) -> Vec<PathBuf> {
        use std::os::unix::fs::DirBuilderExt;
        let (Some(dir), Some(raw)) = (self.dir.as_deref(), self.read(|s| s.raw(id))) else { return Vec::new() };
        let parts = mime::attachments(&raw);
        if parts.is_empty() {
            return Vec::new();
        }
        static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let folder = dir.join(FORWARDS).join(format!("{nanos}-{n}"));
        let written = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir.join(FORWARDS))
            .and_then(|()| std::fs::DirBuilder::new().mode(0o700).create(&folder))
            .and_then(|()| parts.iter().map(|(name, bytes)| mime::save(&folder, &mime::file_name(name), bytes)).collect());
        written.unwrap_or_else(|e| {
            log::warn!("forward: the attachments could not be copied: {e}");
            let _ = std::fs::remove_dir_all(&folder);
            Vec::new()
        })
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

    /// The account's "Offer verification codes" switch; an unknown id is left alone. Its
    /// thread keeps running: [`Mail::codes`] reads the switch from mail.db each time.
    pub fn set_codes(&self, account: AccountId, on: bool) {
        let Some(store) = self.store.get() else { return };
        if let Ok(Some(found)) = store.account(account) {
            let _ = store.put_account(&Account { codes: on, ..found });
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
            if due(&self.checked, account.id) {
                self.job(account.id, sync::Job::Check);
            }
        }
    }

    /// Asks the folder's account to sync it now, as the views do when the user opens it; at
    /// most once every 10 s a folder. [`Event::Changed`] follows if it changed.
    pub fn check_folder(&self, folder: FolderId) {
        let Some(account) = self.read(|s| s.folder(folder)).map(|f| f.account) else { return };
        if due(&self.folders_checked, folder) {
            self.job(account, sync::Job::Folder(folder));
        }
    }

    /// The codes in mail of the Space's accounts that offer codes, from messages that came in
    /// during the last 10 minutes and at `since` or later, newest first. Which may be offered
    /// on a page is [`Code::offered_on`].
    pub fn codes(&self, space: MailSpace, since: SystemTime) -> Vec<Code> {
        let secs = |t: SystemTime| t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let after = secs(since).max(secs(SystemTime::now()) - codes::WINDOW_SECS);
        let offering: Vec<AccountId> = self.accounts(space).into_iter().filter(|a| a.codes).map(|a| a.id).collect();
        codes::found(self.read(|s| s.code_candidates(space, after)).into_iter().filter(|m| offering.contains(&m.account)))
    }
}

/// Whether `key` may be checked now (not within the last 10 s); if so, it is marked checked.
fn due<K: Eq + std::hash::Hash>(checked: &Mutex<HashMap<K, Instant>>, key: K) -> bool {
    let mut checked = checked.lock().unwrap();
    if checked.get(&key).is_some_and(|at| at.elapsed() < Duration::from_secs(10)) {
        return false;
    }
    checked.insert(key, Instant::now());
    true
}

const FORWARDS: &str = "forwards";

/// Removes the forward folders that no saved draft attaches a file from. A draft that cannot
/// be read keeps them all.
fn sweep_forwards(forwards: &Path, store: &Store) {
    let (Ok(entries), Ok(kept)) = (std::fs::read_dir(forwards), store.draft_attachments()) else { return };
    for path in entries.flatten().map(|e| e.path()) {
        if !kept.iter().any(|k| k.starts_with(&path)) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

fn images_key(sender: &str) -> String {
    format!("images:{}", sender.trim().to_lowercase())
}

#[cfg(test)]
mod tests;
