//! One account's server (PLAN-mail.md §4). Protocol only: a backend never reads or writes
//! mail.db, parses no mail and decides no policy. The account's sync thread owns it, so
//! every call may block.

pub mod fake;

use std::time::Duration;

use crate::{Result, Role};

/// A folder as its server lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFolder {
    /// The server's own name or id for it: what the other calls take as `folder`.
    pub path: String,
    /// What the user reads.
    pub name: String,
    pub role: Role,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flags {
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
}

/// A message as its folder's server lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteMessage {
    /// The server's id for it within its folder (an IMAP UID, a Graph id). A move gives
    /// it a new one.
    pub uid: String,
    pub flags: Flags,
    /// Its RFC 822 header block, unparsed.
    pub header: Vec<u8>,
    /// The server's own conversation id, where it has one (Gmail's `X-GM-THRID`).
    pub thread: Option<String>,
}

/// What [`Backend::changes`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    pub new: Vec<RemoteMessage>,
    /// Messages listed before whose flags differ now: the uid and the flags it has now.
    pub changed: Vec<(String, Flags)>,
    /// The uids of messages that left the folder.
    pub removed: Vec<String>,
    /// The cursor no longer fits the folder (IMAP's UIDVALIDITY changed): everything known
    /// of the folder is stale, and `new` holds all of it.
    pub reset: bool,
    /// For the next call.
    pub cursor: String,
}

/// One change to one message, in the server's terms. The sync thread makes these from a
/// [`crate::Action`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Seen(bool),
    Flagged(bool),
    /// Into the folder with this path.
    Move(String),
    /// For good, not into a Trash folder.
    Delete,
}

pub trait Backend: Send {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>>;
    /// What changed in `folder` since `cursor`, the [`Changes::cursor`] of the call before.
    /// `None` is a first sync: every message is new.
    fn changes(&mut self, folder: &str, cursor: Option<&str>) -> Result<Changes>;
    /// The whole message, as RFC 822 bytes.
    fn fetch_body(&mut self, folder: &str, uid: &str) -> Result<Vec<u8>>;
    fn apply(&mut self, folder: &str, uid: &str, op: &Op) -> Result<()>;
    /// Blocks until `folder` may have changed on the server since the last
    /// [`Self::changes`] (IMAP IDLE), or `timeout` passes. `true`: ask for the changes.
    fn wait(&mut self, folder: &str, timeout: Duration) -> Result<bool>;
    /// Stores `raw`, an RFC 822 message, in `folder`: a draft, or the copy of a sent message.
    fn append(&mut self, folder: &str, raw: &[u8], flags: Flags) -> Result<()>;
    /// `to` holds every recipient, Bcc too, which `raw` does not name.
    fn send(&mut self, from: &str, to: &[String], raw: &[u8]) -> Result<()>;
}
