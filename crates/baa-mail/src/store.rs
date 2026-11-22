//! mail.db (PLAN-mail.md §4 "Store", WP-A). The UI thread reads through one `Store` while each
//! account thread writes through its own (`Store::another`).
//!
//! Private mode: every row here belongs to an account, a draft of an account, or a
//! [`MailSpace`], and an ephemeral Space can make neither (`MailSpace::of`), so nothing an
//! incognito Space does reaches this file.

// ponytail: the write side is for the account threads (WP-C, WP-D), which land after it.
// Remove this once they call it.
#![allow(dead_code)]

use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row as SqlRow};

use crate::{
    mime, thread, Account, AccountId, Address, Changes, Draft, DraftId, Error, Filter, Flags, Folder, FolderId, MailSpace,
    Message, MessageId, Op, RemoteFolder, Result, Role, Row, ThreadId, View, ViewState,
};

/// The data version this build writes, as SQLite's `user_version`. Raise it when the schema
/// changes: an older file loses its cache (`CACHE`) and keeps the rest, migrated in `init`.
const SCHEMA: u32 = 1;

/// What the servers hold anyway: dropped and refilled by sync when the schema changes.
const CACHE: [&str; 5] = ["message_fts", "body", "message", "thread_key", "folder"];

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Error {
        Error::Other(format!("mail.db: {e}"))
    }
}

/// A handle on mail.db. A file gives each handle its own connection, so an account thread
/// writes while the UI thread reads (WAL); an in-memory database exists only on its one
/// connection, so every handle of it shares that one.
pub(crate) struct Store {
    conn: Arc<Mutex<Connection>>,
}

/// A folder as the sync thread needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredFolder {
    pub id: FolderId,
    pub account: AccountId,
    /// The server's path, as [`crate::Backend`] takes it.
    pub path: String,
    pub role: Role,
    /// The [`Changes::cursor`] of the last [`Store::apply_changes`]; `None` before the first sync.
    pub cursor: Option<String>,
}

/// Where a message is on its server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Located {
    pub account: AccountId,
    pub folder: FolderId,
    pub path: String,
    pub uid: String,
}

/// An [`Op`] not yet on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingOp {
    pub id: i64,
    pub folder: String,
    pub uid: String,
    pub op: Op,
}

/// A message waiting in the outbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Outgoing {
    pub id: i64,
    pub account: AccountId,
    /// The draft it was made from, removed once it is sent.
    pub draft: Option<DraftId>,
    pub from: String,
    /// Every recipient, Bcc too.
    pub to: Vec<String>,
    pub raw: Vec<u8>,
    /// Failed attempts so far.
    pub tries: u32,
}

/// A message that may carry a verification code: just arrived, not sent by the user.
pub(crate) struct CodeCandidate {
    pub account: AccountId,
    pub from: String,
    pub subject: String,
    pub header: Vec<u8>,
    pub raw: Option<Vec<u8>>,
    pub dmarc_failed: bool,
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Inbox => "inbox",
        Role::Drafts => "drafts",
        Role::Sent => "sent",
        Role::Archive => "archive",
        Role::Junk => "junk",
        Role::Trash => "trash",
        Role::Other => "other",
    }
}

fn role_of(name: &str) -> Role {
    match name {
        "inbox" => Role::Inbox,
        "drafts" => Role::Drafts,
        "sent" => Role::Sent,
        "archive" => Role::Archive,
        "junk" => Role::Junk,
        "trash" => Role::Trash,
        _ => Role::Other,
    }
}

fn op_columns(op: &Op) -> (&'static str, &str) {
    match op {
        Op::Seen(true) => ("seen", ""),
        Op::Seen(false) => ("unseen", ""),
        Op::Flagged(true) => ("flagged", ""),
        Op::Flagged(false) => ("unflagged", ""),
        Op::Move(to) => ("move", to),
        Op::Delete => ("delete", ""),
    }
}

fn op_of(kind: &str, arg: String) -> Option<Op> {
    Some(match kind {
        "seen" => Op::Seen(true),
        "unseen" => Op::Seen(false),
        "flagged" => Op::Flagged(true),
        "unflagged" => Op::Flagged(false),
        "move" => Op::Move(arg),
        "delete" => Op::Delete,
        _ => return None,
    })
}

fn json<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|e| Error::Other(format!("mail.db: {e}")))
}

/// What the user typed, as an FTS5 query: every word must be there, as a word or the start of
/// one. Quoting each word keeps FTS5's own syntax (`OR`, `NEAR`, `*`, `"`) out of it.
fn fts_query(text: &str) -> Option<String> {
    let words: Vec<String> = text.split_whitespace().map(|w| format!("\"{}\"*", w.replace('"', "\"\""))).collect();
    (!words.is_empty()).then(|| words.join(" "))
}

fn people(envelope: &mime::Envelope) -> String {
    let all = std::iter::once(&envelope.from).chain(&envelope.to).chain(&envelope.cc);
    all.map(|a| format!("{} {}", a.name, a.email)).collect::<Vec<_>>().join(" ")
}

impl Store {
    /// `dir`/mail.db, made if it is not there; `None`: in memory, for tests.
    pub(crate) fn open(dir: Option<&Path>) -> Result<Store> {
        let conn = match dir {
            Some(dir) => {
                std::fs::create_dir_all(dir).map_err(|e| Error::Other(format!("mail folder: {e}")))?;
                Connection::open(dir.join("mail.db"))?
            }
            None => Connection::open_in_memory()?,
        };
        init(&conn)?;
        Ok(Store { conn: Arc::new(Mutex::new(conn)) })
    }

    /// Another handle on the same database, for another thread.
    pub(crate) fn another(&self) -> Result<Store> {
        let path = self.conn().path().filter(|p| !p.is_empty()).map(str::to_owned);
        let Some(path) = path else { return Ok(Store { conn: self.conn.clone() }) };
        let conn = Connection::open(path)?;
        init(&conn)?;
        Ok(Store { conn: Arc::new(Mutex::new(conn)) })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // Accounts.

    /// Adds `account` or replaces the row with its id.
    pub(crate) fn put_account(&self, account: &Account) -> Result<()> {
        let row = json(account)?;
        self.conn().execute(
            "INSERT INTO account (id, space, json) VALUES (?1, ?2, ?3) \
             ON CONFLICT (id) DO UPDATE SET space = excluded.space, json = excluded.json",
            params![account.id.0, account.space.map(|s| s.0), row],
        )?;
        Ok(())
    }

    /// An id no account has had.
    pub(crate) fn next_account_id(&self) -> Result<AccountId> {
        let max: Option<u64> = self.conn().query_row("SELECT max(id) FROM account", [], |r| r.get(0))?;
        Ok(AccountId(max.unwrap_or(0) + 1))
    }

    /// `space`'s own accounts and those of every Space; `None`: every account.
    pub(crate) fn accounts(&self, space: Option<MailSpace>) -> Result<Vec<Account>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT json FROM account WHERE ?1 IS NULL OR space IS NULL OR space = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([space.map(|s| s.id().0)], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?).map_err(|e| Error::Other(format!("mail.db: an account row: {e}")))?);
        }
        Ok(out)
    }

    pub(crate) fn account(&self, id: AccountId) -> Result<Option<Account>> {
        let row: Option<String> =
            self.conn().query_row("SELECT json FROM account WHERE id = ?1", [id.0], |r| r.get(0)).optional()?;
        row.map(|row| serde_json::from_str(&row).map_err(|e| Error::Other(format!("mail.db: an account row: {e}")))).transpose()
    }

    /// The account and everything of it: folders, mail, drafts, queued ops and the outbox.
    pub(crate) fn delete_account(&self, id: AccountId) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for table in ["folder", "thread_key", "draft", "op", "outbox"] {
            tx.execute(&format!("DELETE FROM {table} WHERE account = ?1"), [id.0])?;
        }
        tx.execute("DELETE FROM account WHERE id = ?1", [id.0])?;
        tx.commit()?;
        Ok(())
    }

    // Folders.

    /// Makes the account's folders the server's `folders`: new ones are added, known ones
    /// keep their id, mail and cursor, and ones the server no longer lists go with their mail.
    /// Returns their ids in the order given.
    pub(crate) fn put_folders(&self, account: AccountId, folders: &[RemoteFolder]) -> Result<Vec<FolderId>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut ids = Vec::with_capacity(folders.len());
        for f in folders {
            let id: u64 = tx.query_row(
                "INSERT INTO folder (account, path, name, role) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (account, path) DO UPDATE SET name = excluded.name, role = excluded.role RETURNING id",
                params![account.0, f.path, f.name, role_name(f.role)],
                |r| r.get(0),
            )?;
            ids.push(FolderId(id));
        }
        let keep = ids.iter().map(|id| id.0.to_string()).collect::<Vec<_>>().join(",");
        tx.execute(&format!("DELETE FROM folder WHERE account = ?1 AND id NOT IN ({keep})"), [account.0])?;
        tx.commit()?;
        Ok(ids)
    }

    pub(crate) fn folder(&self, id: FolderId) -> Result<Option<StoredFolder>> {
        Ok(self
            .conn()
            .query_row("SELECT id, account, path, role, cursor FROM folder WHERE id = ?1", [id.0], stored_folder)
            .optional()?)
    }

    pub(crate) fn account_folders(&self, account: AccountId) -> Result<Vec<StoredFolder>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare_cached("SELECT id, account, path, role, cursor FROM folder WHERE account = ?1 ORDER BY id")?;
        let rows = stmt.query_map([account.0], stored_folder)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The account's folder for `role` (its Trash, its Archive), if the server marked one.
    pub(crate) fn folder_by_role(&self, account: AccountId, role: Role) -> Result<Option<StoredFolder>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, account, path, role, cursor FROM folder WHERE account = ?1 AND role = ?2 ORDER BY id LIMIT 1",
                params![account.0, role_name(role)],
                stored_folder,
            )
            .optional()?)
    }

    // Sync.

    /// Applies what the server reported for `folder` and keeps its cursor, in one transaction:
    /// a reset first forgets all the folder's mail; new messages are added (one already known
    /// by its uid only takes the new flags), changed ones take their flags, removed ones go.
    /// Returns the ids of the messages added.
    pub(crate) fn apply_changes(&self, folder: FolderId, changes: &Changes) -> Result<Vec<MessageId>> {
        let arrived = now();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let account: u64 = tx.query_row("SELECT account FROM folder WHERE id = ?1", [folder.0], |r| r.get(0))?;
        let account = AccountId(account);
        if changes.reset {
            tx.execute("DELETE FROM message WHERE folder = ?1", [folder.0])?;
        }
        let mut added = Vec::new();
        for m in &changes.new {
            if set_flags(&tx, folder, &m.uid, m.flags)? {
                continue;
            }
            let e = mime::envelope(&m.header);
            let id: i64 = tx.query_row(
                "INSERT INTO message (account, folder, uid, thread, message_id, header, from_name, from_email, subject, \
                 date, arrived, seen, flagged, answered, attachments, dmarc_failed) \
                 VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) RETURNING id",
                params![
                    account.0,
                    folder.0,
                    m.uid,
                    e.message_id,
                    m.header,
                    e.from.name,
                    e.from.email,
                    e.subject,
                    e.date as i64,
                    arrived,
                    m.flags.seen,
                    m.flags.flagged,
                    m.flags.answered,
                    e.attachments,
                    e.dmarc_failed
                ],
                |r| r.get(0),
            )?;
            let thread = thread::assign(&tx, account, id, &thread::keys(&e, m.thread.as_deref()))?;
            tx.execute("UPDATE message SET thread = ?1 WHERE id = ?2", params![thread, id])?;
            tx.execute(
                "INSERT INTO message_fts (rowid, subject, people, text) VALUES (?1, ?2, ?3, '')",
                params![id, e.subject, people(&e)],
            )?;
            added.push(MessageId(id as u64));
        }
        for (uid, flags) in &changes.changed {
            set_flags(&tx, folder, uid, *flags)?;
        }
        for uid in &changes.removed {
            tx.execute("DELETE FROM message WHERE folder = ?1 AND uid = ?2", params![folder.0, uid])?;
        }
        tx.execute("UPDATE folder SET cursor = ?1 WHERE id = ?2", params![changes.cursor, folder.0])?;
        tx.commit()?;
        Ok(added)
    }

    /// Where message `id` is on its server; `None` once it is gone from mail.db.
    pub(crate) fn locate(&self, id: MessageId) -> Result<Option<Located>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT m.account, m.folder, f.path, m.uid FROM message m JOIN folder f ON f.id = m.folder WHERE m.id = ?1",
                [id.0],
                |r| {
                    Ok(Located {
                        account: AccountId(r.get(0)?),
                        folder: FolderId(r.get(1)?),
                        path: r.get(2)?,
                        uid: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// Keeps the RFC 822 bytes `fetch_body` returned, and what the list and search take from them.
    pub(crate) fn put_body(&self, id: MessageId, raw: &[u8]) -> Result<()> {
        let body = mime::body(raw);
        let snippet = mime::snippet(&body);
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("INSERT OR REPLACE INTO body (message, raw) VALUES (?1, ?2)", params![id.0, raw])?;
        tx.execute(
            "UPDATE message SET snippet = ?1, attachments = ?2 WHERE id = ?3",
            params![snippet, !body.attachments.is_empty(), id.0],
        )?;
        tx.execute("UPDATE message_fts SET text = ?1 WHERE rowid = ?2", params![body.text, id.0])?;
        tx.commit()?;
        Ok(())
    }

    /// The user read, flagged or unflagged a message: mail.db changes now, the server when
    /// its [`Op`] is applied.
    pub(crate) fn set_seen_flagged(&self, id: MessageId, seen: Option<bool>, flagged: Option<bool>) -> Result<()> {
        self.conn().execute(
            "UPDATE message SET seen = coalesce(?1, seen), flagged = coalesce(?2, flagged) WHERE id = ?3",
            params![seen, flagged, id.0],
        )?;
        Ok(())
    }

    /// The user moved or deleted a message: it leaves its folder now. A move shows in the
    /// other folder when that folder next syncs.
    pub(crate) fn remove_message(&self, id: MessageId) -> Result<()> {
        self.conn().execute("DELETE FROM message WHERE id = ?1", [id.0])?;
        Ok(())
    }

    // The queue of ops not yet on the server. It is a table, so it survives a restart.

    pub(crate) fn push_op(&self, account: AccountId, folder: &str, uid: &str, op: &Op) -> Result<i64> {
        let (kind, arg) = op_columns(op);
        Ok(self.conn().query_row(
            "INSERT INTO op (account, folder, uid, kind, arg) VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
            params![account.0, folder, uid, kind, arg],
            |r| r.get(0),
        )?)
    }

    /// The account's queued ops, oldest first. They stay queued until [`Store::op_done`].
    pub(crate) fn pending_ops(&self, account: AccountId) -> Result<Vec<PendingOp>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT id, folder, uid, kind, arg FROM op WHERE account = ?1 ORDER BY id")?;
        let rows = stmt.query_map([account.0], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get(4)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, folder, uid, kind, arg) = row?;
            if let Some(op) = op_of(&kind, arg) {
                out.push(PendingOp { id, folder, uid, op });
            }
        }
        Ok(out)
    }

    /// The op reached the server, or never can (its message is gone there).
    pub(crate) fn op_done(&self, id: i64) -> Result<()> {
        self.conn().execute("DELETE FROM op WHERE id = ?1", [id])?;
        Ok(())
    }

    // The outbox.

    pub(crate) fn push_outbox(
        &self,
        account: AccountId,
        draft: Option<DraftId>,
        from: &str,
        to: &[String],
        raw: &[u8],
    ) -> Result<i64> {
        Ok(self.conn().query_row(
            "INSERT INTO outbox (account, draft, sender, recipients, raw) VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
            params![account.0, draft.map(|d| d.0), from, json(&to)?, raw],
            |r| r.get(0),
        )?)
    }

    /// The account's messages waiting to be sent, oldest first.
    pub(crate) fn outbox(&self, account: AccountId) -> Result<Vec<Outgoing>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, account, draft, sender, recipients, raw, tries FROM outbox WHERE account = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([account.0], |r| {
            Ok(Outgoing {
                id: r.get(0)?,
                account: AccountId(r.get(1)?),
                draft: r.get::<_, Option<u64>>(2)?.map(DraftId),
                from: r.get(3)?,
                to: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
                raw: r.get(5)?,
                tries: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// A send failed; it stays in the outbox.
    pub(crate) fn outbox_tried(&self, id: i64) -> Result<()> {
        self.conn().execute("UPDATE outbox SET tries = tries + 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    /// It was sent: it leaves the outbox, and the draft it was made from goes.
    pub(crate) fn outbox_done(&self, id: i64) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM draft WHERE id = (SELECT draft FROM outbox WHERE id = ?1)", [id])?;
        tx.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    // Drafts.

    pub(crate) fn save_draft(&self, draft: &Draft) -> Result<DraftId> {
        let conn = self.conn();
        let id = match draft.id {
            Some(id) => id,
            None => DraftId(conn.query_row("SELECT coalesce(max(id), 0) + 1 FROM draft", [], |r| r.get(0))?),
        };
        let row = json(&Draft { id: Some(id), ..draft.clone() })?;
        conn.execute(
            "INSERT INTO draft (id, account, json) VALUES (?1, ?2, ?3) \
             ON CONFLICT (id) DO UPDATE SET account = excluded.account, json = excluded.json",
            params![id.0, draft.from.0, row],
        )?;
        Ok(id)
    }

    pub(crate) fn draft(&self, id: DraftId) -> Result<Option<Draft>> {
        let row: Option<String> =
            self.conn().query_row("SELECT json FROM draft WHERE id = ?1", [id.0], |r| r.get(0)).optional()?;
        Ok(row.and_then(|row| serde_json::from_str(&row).ok()))
    }

    pub(crate) fn discard_draft(&self, id: DraftId) -> Result<()> {
        self.conn().execute("DELETE FROM draft WHERE id = ?1", [id.0])?;
        Ok(())
    }

    // Per-Space view memory.

    pub(crate) fn view_state(&self, space: MailSpace) -> Result<ViewState> {
        let row: Option<String> =
            self.conn().query_row("SELECT json FROM view WHERE space = ?1", [space.id().0], |r| r.get(0)).optional()?;
        Ok(row.and_then(|row| serde_json::from_str(&row).ok()).unwrap_or_default())
    }

    pub(crate) fn set_view_state(&self, space: MailSpace, state: &ViewState) -> Result<()> {
        self.conn().execute(
            "INSERT INTO view (space, json) VALUES (?1, ?2) ON CONFLICT (space) DO UPDATE SET json = excluded.json",
            params![space.id().0, json(state)?],
        )?;
        Ok(())
    }

    // Reading.

    /// The folders of the Space's accounts, with their unread counts.
    pub(crate) fn folders(&self, space: MailSpace) -> Result<Vec<Folder>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT f.id, f.account, f.name, f.role, \
             (SELECT count(*) FROM message m WHERE m.folder = f.id AND NOT m.seen) \
             FROM folder f JOIN account a ON a.id = f.account WHERE a.space IS NULL OR a.space = ?1",
        )?;
        let rows = stmt.query_map([space.id().0], |r| {
            Ok(Folder {
                id: FolderId(r.get(0)?),
                account: AccountId(r.get(1)?),
                name: r.get(2)?,
                role: role_of(&r.get::<_, String>(3)?),
                unread: r.get(4)?,
            })
        })?;
        let mut out: Vec<Folder> = rows.collect::<rusqlite::Result<_>>()?;
        out.sort_by(|a, b| (a.account.0, a.role as u8, &a.name).cmp(&(b.account.0, b.role as u8, &b.name)));
        Ok(out)
    }

    /// The other Spaces with accounts of their own, and the unread mail in their inboxes.
    pub(crate) fn other_spaces(&self, than: MailSpace) -> Result<Vec<(baa_model::SpaceId, u32)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT a.space, count(m.id) FROM account a \
             LEFT JOIN folder f ON f.account = a.id AND f.role = 'inbox' \
             LEFT JOIN message m ON m.folder = f.id AND NOT m.seen \
             WHERE a.space IS NOT NULL AND a.space <> ?1 GROUP BY a.space ORDER BY a.space",
        )?;
        let rows = stmt.query_map([than.id().0], |r| Ok((baa_model::SpaceId(r.get(0)?), r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One row a conversation, newest first.
    pub(crate) fn rows(&self, view: &View, range: std::ops::Range<usize>) -> Result<Vec<Row>> {
        let sql = format!(
            "{} SELECT thread, account, from_name, from_email, subject, snippet, newest, all_seen, any_flagged, any_attachment \
             FROM listed ORDER BY newest DESC, thread DESC LIMIT ? OFFSET ?",
            listed(view)
        );
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&sql)?;
        let mut args = list_args(view);
        args.push(rusqlite::types::Value::Integer(range.len() as i64));
        args.push(rusqlite::types::Value::Integer(range.start as i64));
        let rows = stmt.query_map(params_from_iter(args), |r| {
            let (name, email): (String, String) = (r.get(2)?, r.get(3)?);
            Ok(Row {
                thread: ThreadId(r.get(0)?),
                account: AccountId(r.get(1)?),
                from: if name.is_empty() { email } else { name },
                subject: r.get(4)?,
                snippet: r.get(5)?,
                date: r.get::<_, i64>(6)?.max(0) as u64,
                unread: !r.get::<_, bool>(7)?,
                flagged: r.get(8)?,
                attachments: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn count(&self, view: &View) -> Result<usize> {
        let sql = format!("{} SELECT count(*) FROM listed", listed(view));
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&sql)?;
        Ok(stmt.query_row(params_from_iter(list_args(view)), |r| r.get::<_, i64>(0))? as usize)
    }

    /// A conversation's messages, oldest first. A message the server keeps in two folders
    /// (a Gmail label and All Mail) is listed once.
    pub(crate) fn conversation(&self, id: ThreadId) -> Result<Vec<Message>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, account, message_id, header, from_name, from_email, subject, date, seen, flagged \
             FROM message WHERE thread = ?1 ORDER BY date, id",
        )?;
        let rows = stmt.query_map([id.0], |r| {
            let header: Vec<u8> = r.get(3)?;
            let e = mime::envelope(&header);
            let message = Message {
                id: MessageId(r.get(0)?),
                account: AccountId(r.get(1)?),
                from: Address { name: r.get(4)?, email: r.get(5)? },
                to: e.to,
                cc: e.cc,
                subject: r.get(6)?,
                date: r.get::<_, i64>(7)?.max(0) as u64,
                unread: !r.get::<_, bool>(8)?,
                flagged: r.get(9)?,
            };
            Ok((message, r.get::<_, Option<String>>(2)?))
        })?;
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for row in rows {
            let (message, message_id) = row?;
            if message_id.is_none_or(|id| seen.insert((message.account, id))) {
                out.push(message);
            }
        }
        Ok(out)
    }

    /// The RFC 822 bytes of a message, if they have been fetched.
    pub(crate) fn raw(&self, id: MessageId) -> Result<Option<Vec<u8>>> {
        Ok(self.conn().query_row("SELECT raw FROM body WHERE message = ?1", [id.0], |r| r.get(0)).optional()?)
    }

    /// What a reply or forward needs from a message besides its body.
    pub(crate) fn envelope(&self, id: MessageId) -> Result<Option<(AccountId, mime::Envelope)>> {
        let row: Option<(u64, Vec<u8>)> = self
            .conn()
            .query_row("SELECT account, header FROM message WHERE id = ?1", [id.0], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        Ok(row.map(|(account, header)| (AccountId(account), mime::envelope(&header))))
    }

    /// Messages of the Space's accounts that came in at `after` or later (stored then, and
    /// dated then), newest first, outside the folders the user's own mail is in.
    pub(crate) fn code_candidates(&self, space: MailSpace, after: i64) -> Result<Vec<CodeCandidate>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT m.account, m.from_email, m.subject, m.header, b.raw, m.dmarc_failed FROM message m \
             JOIN folder f ON f.id = m.folder JOIN account a ON a.id = m.account LEFT JOIN body b ON b.message = m.id \
             WHERE (a.space IS NULL OR a.space = ?1) AND f.role NOT IN ('sent', 'drafts', 'trash') \
             AND m.arrived >= ?2 AND m.date >= ?2 ORDER BY m.date DESC, m.id DESC",
        )?;
        let rows = stmt.query_map(params![space.id().0, after], |r| {
            Ok(CodeCandidate {
                account: AccountId(r.get(0)?),
                from: r.get(1)?,
                subject: r.get(2)?,
                header: r.get(3)?,
                raw: r.get(4)?,
                dmarc_failed: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

fn stored_folder(r: &SqlRow) -> rusqlite::Result<StoredFolder> {
    Ok(StoredFolder {
        id: FolderId(r.get(0)?),
        account: AccountId(r.get(1)?),
        path: r.get(2)?,
        role: role_of(&r.get::<_, String>(3)?),
        cursor: r.get(4)?,
    })
}

/// `true`: a message with this uid is in the folder, and now has `flags`.
fn set_flags(tx: &rusqlite::Transaction, folder: FolderId, uid: &str, flags: Flags) -> rusqlite::Result<bool> {
    let changed = tx.execute(
        "UPDATE message SET seen = ?1, flagged = ?2, answered = ?3 WHERE folder = ?4 AND uid = ?5",
        params![flags.seen, flags.flagged, flags.answered, folder.0, uid],
    )?;
    Ok(changed > 0)
}

/// The `listed` CTE of a list: the newest message of each conversation in the view, with what
/// the row says of the whole conversation. Its parameters are [`list_args`].
fn listed(view: &View) -> String {
    let having = match view.filter {
        Filter::All => "",
        Filter::Unread => " AND NOT all_seen",
        Filter::Flagged => " AND any_flagged",
    };
    let search = if fts_query(&view.search).is_some() {
        " AND m.id IN (SELECT rowid FROM message_fts WHERE message_fts MATCH ?4)"
    } else {
        " AND ?4 IS NULL"
    };
    format!(
        "WITH scoped AS (SELECT m.*, \
           row_number() OVER (PARTITION BY m.thread ORDER BY m.date DESC, m.id DESC) AS n, \
           max(m.date) OVER t AS newest, min(m.seen) OVER t AS all_seen, \
           max(m.flagged) OVER t AS any_flagged, max(m.attachments) OVER t AS any_attachment \
           FROM message m JOIN folder f ON f.id = m.folder JOIN account a ON a.id = m.account \
           WHERE (a.space IS NULL OR a.space = ?1) AND (?2 IS NULL OR m.account = ?2) \
           AND CASE WHEN ?3 IS NULL THEN f.role = 'inbox' ELSE m.folder = ?3 END{search} \
           WINDOW t AS (PARTITION BY m.thread)), \
         listed AS (SELECT * FROM scoped WHERE n = 1{having})"
    )
}

fn list_args(view: &View) -> Vec<rusqlite::types::Value> {
    use rusqlite::types::Value;
    let opt = |v: Option<u64>| v.map_or(Value::Null, |v| Value::Integer(v as i64));
    vec![
        Value::Integer(view.space.id().0 as i64),
        opt(view.account.map(|a| a.0)),
        opt(view.folder.map(|f| f.0)),
        fts_query(&view.search).map_or(Value::Null, Value::Text),
    ]
}

/// The version rule of baa-model's `Store::init`: a newer file is refused untouched; an older
/// one loses its cache, which sync fills again, and keeps accounts, drafts, the outbox, the op
/// queue, `local` and view rows.
fn init(conn: &Connection) -> Result<()> {
    let stored: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if stored > SCHEMA {
        return Err(Error::Other(format!(
            "mail.db was written by a newer baa (data version {stored}; this build reads up to {SCHEMA}). \
             Update baa, or move mail.db out of the mail folder to start fresh."
        )));
    }
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    if conn.path().is_some_and(|p| !p.is_empty()) {
        conn.pragma_update(None, "journal_mode", "WAL")?;
    }
    conn.pragma_update(None, "foreign_keys", true)?;
    if stored < SCHEMA {
        for table in CACHE {
            conn.execute(&format!("DROP TABLE IF EXISTS {table}"), [])?;
        }
    }
    conn.execute_batch(
        "BEGIN;
         CREATE TABLE IF NOT EXISTS account (id INTEGER PRIMARY KEY, space INTEGER, json TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS draft (id INTEGER PRIMARY KEY, account INTEGER NOT NULL, json TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS outbox (id INTEGER PRIMARY KEY, account INTEGER NOT NULL, draft INTEGER,
           sender TEXT NOT NULL, recipients TEXT NOT NULL, raw BLOB NOT NULL, tries INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE IF NOT EXISTS op (id INTEGER PRIMARY KEY, account INTEGER NOT NULL, folder TEXT NOT NULL,
           uid TEXT NOT NULL, kind TEXT NOT NULL, arg TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS local (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS view (space INTEGER PRIMARY KEY, json TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS folder (id INTEGER PRIMARY KEY, account INTEGER NOT NULL, path TEXT NOT NULL,
           name TEXT NOT NULL, role TEXT NOT NULL, cursor TEXT, UNIQUE (account, path));
         CREATE TABLE IF NOT EXISTS message (id INTEGER PRIMARY KEY, account INTEGER NOT NULL,
           folder INTEGER NOT NULL REFERENCES folder (id) ON DELETE CASCADE, uid TEXT NOT NULL,
           thread INTEGER NOT NULL, message_id TEXT, header BLOB NOT NULL, from_name TEXT NOT NULL,
           from_email TEXT NOT NULL, subject TEXT NOT NULL, date INTEGER NOT NULL, arrived INTEGER NOT NULL,
           seen INTEGER NOT NULL, flagged INTEGER NOT NULL, answered INTEGER NOT NULL,
           attachments INTEGER NOT NULL, dmarc_failed INTEGER NOT NULL, snippet TEXT NOT NULL DEFAULT '',
           UNIQUE (folder, uid));
         CREATE INDEX IF NOT EXISTS message_thread ON message (thread);
         CREATE INDEX IF NOT EXISTS message_folder_date ON message (folder, date);
         CREATE TABLE IF NOT EXISTS body (message INTEGER PRIMARY KEY REFERENCES message (id) ON DELETE CASCADE,
           raw BLOB NOT NULL);
         CREATE TABLE IF NOT EXISTS thread_key (account INTEGER NOT NULL, key TEXT NOT NULL,
           thread INTEGER NOT NULL, PRIMARY KEY (account, key));
         CREATE VIRTUAL TABLE IF NOT EXISTS message_fts USING fts5 (subject, people, text);
         CREATE TRIGGER IF NOT EXISTS message_gone AFTER DELETE ON message
           BEGIN DELETE FROM message_fts WHERE rowid = old.id; END;
         COMMIT;",
    )?;
    if stored != SCHEMA {
        conn.pragma_update(None, "user_version", SCHEMA)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::RemoteMessage;

    /// A Space that may have mail, with this id.
    pub(crate) fn space(id: u64) -> MailSpace {
        let mut session = baa_model::Session::first_run();
        let own = session.active_space().unwrap();
        session.space_mut(own).unwrap().id = baa_model::SpaceId(id);
        MailSpace::of(session.space(baa_model::SpaceId(id)).unwrap()).unwrap()
    }

    pub(crate) fn account(id: u64, space: Option<MailSpace>) -> Account {
        let address = format!("dan{id}@example.com");
        let provider = crate::provider::Provider::Fastmail;
        let servers = provider.servers(&address).unwrap();
        Account { id: AccountId(id), space: space.map(MailSpace::id), name: "Dan".into(), address, codes: true, provider, servers, sign_in: crate::SignIn::Password }
    }

    pub(crate) fn header(id: &str, subject: &str, date: &str, extra: &str) -> Vec<u8> {
        format!("From: Ola <ola@example.com>\r\nTo: dan@example.com\r\nSubject: {subject}\r\nDate: {date}\r\nMessage-ID: <{id}>\r\n{extra}\r\n").into_bytes()
    }

    pub(crate) fn new(uid: &str, header: Vec<u8>, flags: Flags) -> RemoteMessage {
        RemoteMessage { uid: uid.into(), flags, header, thread: None }
    }

    pub(crate) fn inbox(store: &Store, account: AccountId) -> FolderId {
        let folders = [
            RemoteFolder { path: "INBOX".into(), name: "Inbox".into(), role: Role::Inbox },
            RemoteFolder { path: "Archive".into(), name: "Archive".into(), role: Role::Archive },
        ];
        store.put_folders(account, &folders).unwrap()[0]
    }

    fn view(space: MailSpace) -> View {
        View { space, folder: None, account: None, filter: Filter::All, search: String::new() }
    }

    fn draft(subject: &str) -> Draft {
        Draft { id: None, from: AccountId(1), to: Vec::new(), cc: Vec::new(), bcc: Vec::new(), subject: subject.into(), html: String::new(), text: String::new(), attachments: Vec::new(), in_reply_to: None }
    }

    pub(crate) fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("baa-mail-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn subjects(store: &Store, view: &View) -> Vec<String> {
        store.rows(view, 0..10).unwrap().into_iter().map(|r| r.subject).collect()
    }

    #[test]
    fn all_inboxes_of_two_accounts_is_one_list_by_date_and_the_scope_chip_narrows_it() {
        let store = Store::open(None).unwrap();
        let home = space(1);
        store.put_account(&account(1, Some(home))).unwrap();
        store.put_account(&account(2, None)).unwrap();
        let (a, b) = (inbox(&store, AccountId(1)), inbox(&store, AccountId(2)));
        let changes = |new, cursor: &str| Changes { new, cursor: cursor.into(), ..Changes::default() };
        let seen = Flags { seen: true, ..Flags::default() };
        let flagged = Flags { flagged: true, ..Flags::default() };
        store.apply_changes(a, &changes(vec![new("1", header("a1@x", "Oldest", "Sun, 1 Nov 2026 10:00:00 +0000", ""), Flags::default())], "1")).unwrap();
        store.apply_changes(b, &changes(vec![new("1", header("b1@x", "Newest", "Sun, 22 Nov 2026 10:00:00 +0000", ""), seen)], "1")).unwrap();
        store.apply_changes(a, &changes(vec![new("2", header("a2@x", "Middle", "Sun, 15 Nov 2026 10:00:00 +0000", ""), flagged)], "2")).unwrap();

        let all = view(home);
        assert_eq!(subjects(&store, &all), ["Newest", "Middle", "Oldest"]);
        assert_eq!(store.count(&all).unwrap(), 3);
        assert_eq!(store.rows(&all, 1..2).unwrap()[0].subject, "Middle");
        assert_eq!(subjects(&store, &View { account: Some(AccountId(1)), ..all.clone() }), ["Middle", "Oldest"]);
        assert_eq!(subjects(&store, &View { filter: Filter::Unread, ..all.clone() }), ["Middle", "Oldest"]);
        assert_eq!(subjects(&store, &View { filter: Filter::Flagged, ..all.clone() }), ["Middle"]);
        let archive = store.folder_by_role(AccountId(1), Role::Archive).unwrap().unwrap().id;
        assert!(subjects(&store, &View { folder: Some(archive), ..all.clone() }).is_empty());
        let row = &store.rows(&all, 0..1).unwrap()[0];
        assert_eq!((row.from.as_str(), row.account, row.date, row.unread), ("Ola", AccountId(2), 1_795_341_600, false));
        assert_eq!(store.folder(a).unwrap().unwrap().cursor.as_deref(), Some("2"));
        let inboxes: Vec<(AccountId, u32)> =
            store.folders(home).unwrap().iter().filter(|f| f.role == Role::Inbox).map(|f| (f.account, f.unread)).collect();
        assert_eq!(inboxes, [(AccountId(1), 2), (AccountId(2), 0)]);
    }

    #[test]
    fn a_space_sees_its_own_accounts_and_the_shared_ones_never_another_spaces() {
        let store = Store::open(None).unwrap();
        let (home, work) = (space(1), space(2));
        store.put_account(&account(1, Some(home))).unwrap();
        store.put_account(&account(2, Some(work))).unwrap();
        store.put_account(&account(3, None)).unwrap();
        let ids = |s| store.accounts(Some(s)).unwrap().into_iter().map(|a| a.id.0).collect::<Vec<_>>();
        assert_eq!((ids(home), ids(work)), (vec![1, 3], vec![2, 3]));
        assert_eq!(store.next_account_id().unwrap(), AccountId(4));

        let work_inbox = inbox(&store, AccountId(2));
        let mail = Changes { new: vec![new("1", header("w@x", "Work only", "Sun, 22 Nov 2026 10:00:00 +0000", ""), Flags::default())], ..Changes::default() };
        store.apply_changes(work_inbox, &mail).unwrap();
        assert_eq!(store.count(&view(home)).unwrap(), 0);
        assert_eq!(store.count(&View { folder: Some(work_inbox), ..view(home) }).unwrap(), 0, "a folder of another Space's account");
        assert_eq!(store.count(&view(work)).unwrap(), 1);
        assert!(store.folders(home).unwrap().iter().all(|f| f.account != AccountId(2)));
        assert_eq!(store.other_spaces(home).unwrap(), [(work.id(), 1)]);
        assert_eq!(store.other_spaces(work).unwrap(), [(home.id(), 0)]);

        store.delete_account(AccountId(2)).unwrap();
        assert_eq!(store.count(&view(work)).unwrap(), 0);
        assert!(store.account_folders(AccountId(2)).unwrap().is_empty() && store.account(AccountId(2)).unwrap().is_none());
    }

    #[test]
    fn a_reply_joins_its_conversation_by_references_or_by_the_servers_thread() {
        let store = Store::open(None).unwrap();
        let home = space(1);
        store.put_account(&account(1, Some(home))).unwrap();
        let a = inbox(&store, AccountId(1));
        let first = header("root@x", "Plans", "Sun, 1 Nov 2026 10:00:00 +0000", "");
        let reply = header("reply@x", "Re: Plans", "Mon, 2 Nov 2026 10:00:00 +0000", "In-Reply-To: <root@x>\r\n");
        let deeper = header("deeper@x", "Re: Plans", "Tue, 3 Nov 2026 10:00:00 +0000", "References: <root@x> <reply@x>\r\n");
        let other = header("other@x", "Other", "Tue, 3 Nov 2026 09:00:00 +0000", "");
        let mut by_server = new("5", header("gm@x", "Gmail", "Wed, 4 Nov 2026 10:00:00 +0000", ""), Flags::default());
        by_server.thread = Some("t1".into());
        let mut by_server_too = new("6", header("gm2@x", "Gmail again", "Thu, 5 Nov 2026 10:00:00 +0000", ""), Flags::default());
        by_server_too.thread = Some("t1".into());
        let messages = vec![
            new("1", first, Flags::default()),
            new("2", reply, Flags::default()),
            new("3", deeper, Flags::default()),
            new("4", other, Flags::default()),
            by_server,
            by_server_too,
        ];
        store.apply_changes(a, &Changes { new: messages, ..Changes::default() }).unwrap();

        let rows = store.rows(&view(home), 0..10).unwrap();
        assert_eq!(rows.iter().map(|r| r.subject.as_str()).collect::<Vec<_>>(), ["Gmail again", "Re: Plans", "Other"]);
        let plans = store.conversation(rows[1].thread).unwrap();
        assert_eq!(plans.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["Plans", "Re: Plans", "Re: Plans"]);
        assert_eq!(plans[0].to, [Address { name: String::new(), email: "dan@example.com".into() }]);
        assert_eq!(store.conversation(rows[0].thread).unwrap().len(), 2);
    }

    #[test]
    fn changes_update_flags_remove_mail_and_a_reset_starts_the_folder_over() {
        let store = Store::open(None).unwrap();
        let home = space(1);
        store.put_account(&account(1, Some(home))).unwrap();
        let a = inbox(&store, AccountId(1));
        let one = || new("1", header("one@x", "One", "Sun, 1 Nov 2026 10:00:00 +0000", ""), Flags::default());
        let two = new("2", header("two@x", "Two", "Sun, 1 Nov 2026 11:00:00 +0000", ""), Flags::default());
        let added = store.apply_changes(a, &Changes { new: vec![one(), two], ..Changes::default() }).unwrap();
        assert_eq!(added.len(), 2);
        assert_eq!(store.apply_changes(a, &Changes { new: vec![one()], ..Changes::default() }).unwrap(), [], "a uid it knows is not added twice");

        let seen = Flags { seen: true, ..Flags::default() };
        store.apply_changes(a, &Changes { changed: vec![("1".into(), seen)], removed: vec!["2".into()], ..Changes::default() }).unwrap();
        let rows = store.rows(&view(home), 0..10).unwrap();
        assert_eq!((rows.len(), rows[0].subject.as_str(), rows[0].unread), (1, "One", false));
        assert_eq!(store.locate(added[0]).unwrap().map(|l| (l.path, l.uid)), Some(("INBOX".into(), "1".into())));
        assert_eq!(store.locate(added[1]).unwrap(), None);

        let fresh = new("9", header("nine@x", "Nine", "Sun, 1 Nov 2026 12:00:00 +0000", ""), Flags::default());
        store.apply_changes(a, &Changes { new: vec![fresh], reset: true, cursor: "v2".into(), ..Changes::default() }).unwrap();
        assert_eq!(subjects(&store, &view(home)), ["Nine"]);

        let nine = store.conversation(store.rows(&view(home), 0..1).unwrap()[0].thread).unwrap()[0].id;
        store.set_seen_flagged(nine, None, Some(true)).unwrap();
        assert!(store.rows(&view(home), 0..1).unwrap()[0].flagged);
        store.remove_message(nine).unwrap();
        assert_eq!(store.count(&view(home)).unwrap(), 0);

        store.put_folders(AccountId(1), &[RemoteFolder { path: "Archive".into(), name: "Archive".into(), role: Role::Archive }]).unwrap();
        assert_eq!(store.folder(a).unwrap(), None, "a folder the server no longer lists goes");
    }

    #[test]
    fn search_finds_a_word_of_the_body_and_of_the_subject() {
        let store = Store::open(None).unwrap();
        let home = space(1);
        store.put_account(&account(1, Some(home))).unwrap();
        let a = inbox(&store, AccountId(1));
        let h = header("s@x", "Call sheet", "Sun, 1 Nov 2026 10:00:00 +0000", "");
        let id = store.apply_changes(a, &Changes { new: vec![new("1", h.clone(), Flags::default())], ..Changes::default() }).unwrap()[0];
        let mut raw = h;
        raw.extend_from_slice(b"Bring the clapperboard at 09:10.\r\n");
        store.put_body(id, &raw).unwrap();

        let search = |text: &str| store.count(&View { search: text.into(), ..view(home) }).unwrap();
        assert_eq!((search("clapper"), search("CALL"), search("ola"), search("nothing")), (1, 1, 1, 0));
        assert_eq!(search("\"unbalanced OR NEAR( *"), 0, "FTS syntax in the text is only text");
        assert_eq!(store.rows(&view(home), 0..1).unwrap()[0].snippet, "Bring the clapperboard at 09:10.");
        assert_eq!(store.raw(id).unwrap(), Some(raw));
    }

    #[test]
    fn the_op_queue_and_the_outbox_survive_a_restart() {
        let dir = temp_dir("queue");
        {
            let store = Store::open(Some(&dir)).unwrap();
            let thread = store.another().unwrap();
            thread.put_account(&account(1, None)).unwrap();
            thread.push_op(AccountId(1), "INBOX", "7", &Op::Move("Archive".into())).unwrap();
            thread.push_op(AccountId(1), "INBOX", "8", &Op::Seen(false)).unwrap();
            let draft = store.save_draft(&draft("s")).unwrap();
            thread.push_outbox(AccountId(1), Some(draft), "dan1@example.com", &["ola@example.com".into()], b"raw").unwrap();
            assert_eq!(store.outbox(AccountId(1)).unwrap().len(), 1, "the UI's handle sees the thread's write");
        }
        let store = Store::open(Some(&dir)).unwrap();
        let ops = store.pending_ops(AccountId(1)).unwrap();
        assert_eq!(ops.iter().map(|o| (o.uid.as_str(), o.op.clone())).collect::<Vec<_>>(), [("7", Op::Move("Archive".into())), ("8", Op::Seen(false))]);
        store.op_done(ops[0].id).unwrap();
        assert_eq!(store.pending_ops(AccountId(1)).unwrap().len(), 1);

        let out = store.outbox(AccountId(1)).unwrap();
        assert_eq!((out[0].to.as_slice(), out[0].raw.as_slice(), out[0].tries), (&["ola@example.com".to_owned()][..], &b"raw"[..], 0));
        store.outbox_tried(out[0].id).unwrap();
        assert_eq!(store.outbox(AccountId(1)).unwrap()[0].tries, 1);
        let draft = out[0].draft.unwrap();
        assert!(store.draft(draft).unwrap().is_some());
        store.outbox_done(out[0].id).unwrap();
        assert!(store.outbox(AccountId(1)).unwrap().is_empty() && store.draft(draft).unwrap().is_none(), "a sent draft goes");
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_newer_file_is_refused_untouched_and_an_older_one_keeps_what_no_server_holds() {
        let dir = temp_dir("version");
        let home = space(1);
        {
            let store = Store::open(Some(&dir)).unwrap();
            store.put_account(&account(1, Some(home))).unwrap();
            let a = inbox(&store, AccountId(1));
            let cached = new("1", header("v@x", "Cached", "Sun, 1 Nov 2026 10:00:00 +0000", ""), Flags::default());
            store.apply_changes(a, &Changes { new: vec![cached], ..Changes::default() }).unwrap();
            store.save_draft(&draft("kept")).unwrap();
            store.set_view_state(home, &ViewState { scroll: 4, ..ViewState::default() }).unwrap();
            store.push_op(AccountId(1), "INBOX", "1", &Op::Delete).unwrap();
            store.conn().pragma_update(None, "user_version", SCHEMA + 1).unwrap();
            store.conn().pragma_update(None, "wal_checkpoint", "TRUNCATE").unwrap();
        }
        let db = dir.join("mail.db");
        let before = std::fs::read(&db).unwrap();
        assert!(Store::open(Some(&dir)).err().unwrap().to_string().contains("newer baa"));
        assert_eq!(std::fs::read(&db).unwrap(), before, "untouched");

        Connection::open(&db).unwrap().pragma_update(None, "user_version", 0).unwrap();
        let store = Store::open(Some(&dir)).unwrap();
        assert_eq!(store.count(&view(home)).unwrap(), 0, "the cache is dropped");
        assert!(store.account_folders(AccountId(1)).unwrap().is_empty());
        assert_eq!(store.accounts(None).unwrap().len(), 1);
        assert_eq!(store.draft(DraftId(1)).unwrap().map(|d| d.subject).as_deref(), Some("kept"));
        assert_eq!(store.view_state(home).unwrap().scroll, 4);
        assert_eq!(store.pending_ops(AccountId(1)).unwrap().len(), 1);
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
