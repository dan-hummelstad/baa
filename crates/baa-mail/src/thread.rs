//! Conversations (PLAN-mail.md §11 WP-A). A conversation's id is the row id of the first of
//! its messages that baa stored; `thread_key` maps every Message-ID a message names, and the
//! server's own conversation id, to it.

use rusqlite::{params, OptionalExtension, Transaction};

use crate::{mime::Envelope, AccountId};

/// What ties a message to its conversation: the server's conversation id first, then its own
/// Message-ID, then the ones it answers.
pub(crate) fn keys(envelope: &Envelope, hint: Option<&str>) -> Vec<String> {
    let ids = envelope.message_id.iter().chain(&envelope.in_reply_to).chain(&envelope.references);
    hint.map(|h| format!("server:{h}")).into_iter().chain(ids.map(|id| format!("id:{id}"))).collect()
}

/// The conversation of message `id`, which has `keys`: the oldest one any key already names,
/// else its own.
// ponytail: two conversations a message bridges are not merged, and nothing threads by
// subject. Ceiling: a reply whose References name only a message baa never stored starts a
// conversation of its own.
pub(crate) fn assign(tx: &Transaction, account: AccountId, id: i64, keys: &[String]) -> rusqlite::Result<i64> {
    let mut known = None::<i64>;
    let mut find = tx.prepare_cached("SELECT thread FROM thread_key WHERE account = ?1 AND key = ?2")?;
    for key in keys {
        if let Some(thread) = find.query_row(params![account.0, key], |r| r.get::<_, i64>(0)).optional()? {
            known = Some(known.map_or(thread, |k| k.min(thread)));
        }
    }
    let thread = known.unwrap_or(id);
    let mut add = tx.prepare_cached("INSERT OR IGNORE INTO thread_key (account, key, thread) VALUES (?1, ?2, ?3)")?;
    for key in keys {
        add.execute(params![account.0, key, thread])?;
    }
    Ok(thread)
}
