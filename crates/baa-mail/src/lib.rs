//! Mail without its views (PLAN-mail.md §4): accounts, sync, the store, MIME, sending and
//! verification codes. No gpui; the views are `baa-ui/src/mail`.
//!
//! So far this is the contract of PLAN-mail.md §8 with nothing behind it: [`Mail`] keeps
//! nothing, opens no file and contacts nobody.

mod account;
mod backend;
mod secrets;

use std::{
    ops::Range,
    path::{Path, PathBuf},
    time::SystemTime,
};

use baa_model::SpaceId;
use serde::{Deserialize, Serialize};

pub use account::{Account, NewAccount};
pub use backend::{
    fake::{Fake, SentMessage},
    Backend, Changes, Flags, Op, RemoteFolder, RemoteMessage,
};
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

/// Why a mail call failed, in words the user can be shown. Never holds a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

const NOT_BUILT: &str = "Mail is not built yet";

/// One a process, shared by its windows. `Send + Sync`.
pub struct Mail(());

impl Mail {
    /// Starts nothing: an account's thread starts when the account is first needed.
    // ponytail: nothing is kept, so nothing is on disk or in the Keychain. WP-A opens
    // `dir`/mail.db, WP-E keeps `secrets`, WP-C calls `notify`.
    pub fn open(_dir: &Path, _secrets: Box<dyn Secrets>, _notify: Box<dyn Fn(Event) + Send + Sync>) -> Result<Mail> {
        Ok(Mail(()))
    }

    /// The Space's own accounts and those of every Space.
    pub fn accounts(&self, _space: MailSpace) -> Vec<Account> {
        Vec::new()
    }

    /// Tests the sign-in first, and stores nothing if it fails.
    pub fn add_account(&self, _new: NewAccount) -> Result<AccountId> {
        Err(Error(NOT_BUILT.into()))
    }

    pub fn remove_account(&self, _id: AccountId) {}

    /// Removes the accounts that belong to `space` alone, with their mail and secrets, and
    /// returns them. Accounts of every Space stay.
    pub fn remove_accounts_of(&self, _space: MailSpace) -> Vec<Account> {
        Vec::new()
    }

    /// The other Spaces that have accounts of their own, with their unread counts.
    pub fn other_spaces(&self, _than: MailSpace) -> Vec<(SpaceId, u32)> {
        Vec::new()
    }

    pub fn folders(&self, _space: MailSpace) -> Vec<Folder> {
        Vec::new()
    }

    /// One row a conversation, newest first.
    pub fn rows(&self, _view: &View, _range: Range<usize>) -> Vec<Row> {
        Vec::new()
    }

    pub fn count(&self, _view: &View) -> usize {
        0
    }

    /// Oldest first.
    pub fn conversation(&self, _id: ThreadId) -> Vec<Message> {
        Vec::new()
    }

    /// `None`: not here yet. It has been asked for, and [`Event::Changed`] follows.
    pub fn body(&self, _id: MessageId) -> Option<Body> {
        None
    }

    pub fn search(&self, space: MailSpace, text: &str) -> View {
        View { space, folder: None, account: None, filter: Filter::All, search: text.into() }
    }

    /// Applied to mail.db at once, then to the server.
    pub fn act(&self, _on: &[MessageId], _action: Action) {}

    pub fn save_draft(&self, _draft: &Draft) -> Result<DraftId> {
        Err(Error(NOT_BUILT.into()))
    }

    /// [`Event::Sent`] or [`Event::Failed`] follows.
    pub fn send(&self, _draft: DraftId) {}

    /// Into `dir`, under the last component of the attachment's name. Returns where it is.
    pub fn save_attachment(&self, _of: MessageId, _part: usize, _dir: &Path) -> Result<PathBuf> {
        Err(Error(NOT_BUILT.into()))
    }

    pub fn view_state(&self, _space: MailSpace) -> ViewState {
        ViewState::default()
    }

    // ponytail: not kept until WP-A's store has the `view` row.
    pub fn set_view_state(&self, _space: MailSpace, _state: &ViewState) {}

    /// Asks the Space's accounts for one INBOX check now, at most one every 10 s an account.
    pub fn check_now(&self, _space: MailSpace) {}

    pub fn codes(&self, _space: MailSpace, _since: SystemTime) -> Vec<Code> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space(session: &baa_model::Session) -> MailSpace {
        MailSpace::of(session.space(session.active_space().unwrap()).unwrap()).unwrap()
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
    fn open_writes_nothing_and_every_read_is_empty() {
        fn shared<T: Send + Sync>() {}
        shared::<Mail>();

        let dir = std::env::temp_dir().join(format!("baa-mail-{}-open", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mail = Mail::open(&dir, Box::new(MemorySecrets::default()), Box::new(|_| panic!("nothing has happened"))).unwrap();
        assert!(!dir.exists(), "nothing on disk before there is an account");

        let space = space(&baa_model::Session::first_run());
        let view = mail.search(space, "invoice");
        assert_eq!(view, View { space, folder: None, account: None, filter: Filter::All, search: "invoice".into() });
        assert!(mail.accounts(space).is_empty() && mail.folders(space).is_empty() && mail.other_spaces(space).is_empty());
        assert!(mail.rows(&view, 0..50).is_empty() && mail.count(&view) == 0);
        assert!(mail.conversation(ThreadId(1)).is_empty() && mail.body(MessageId(1)).is_none());
        assert!(mail.codes(space, SystemTime::now()).is_empty());
        assert!(mail.remove_accounts_of(space).is_empty());
        mail.set_view_state(space, &ViewState { scroll: 3, ..ViewState::default() });
        assert_eq!(mail.view_state(space), ViewState::default());

        let new = NewAccount { space: Some(space), name: "Dan".into(), address: "dan@example.com".into(), password: "app-password".into() };
        assert_eq!(mail.add_account(new), Err(Error("Mail is not built yet".into())));
        let draft = Draft {
            id: None,
            from: AccountId(1),
            to: vec![Address { name: String::new(), email: "ola@example.com".into() }],
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: "Hei".into(),
            html: "<p>Hei</p>".into(),
            text: "Hei".into(),
            attachments: Vec::new(),
            in_reply_to: None,
        };
        assert!(mail.save_draft(&draft).is_err() && mail.save_attachment(MessageId(1), 0, &dir).is_err());
        mail.act(&[MessageId(1)], Action::Archive);
        mail.send(DraftId(1));
        mail.check_now(space);
        mail.remove_account(AccountId(1));
        assert!(!dir.exists());
    }
}
