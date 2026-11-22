//! A mail server in memory, for tests.

use std::{
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

use super::{Backend, Changes, Flags, Op, RemoteFolder, RemoteMessage};
use crate::{Error, Result, Role};

/// Holds the folders and messages it was given and what was sent through it. A clone is
/// the same server: the sync thread owns one, the test keeps another to deliver mail and
/// to read what arrived.
#[derive(Clone, Default)]
pub struct Fake {
    server: Arc<(Mutex<Server>, Condvar)>,
    /// The server's clock at this clone's last `changes`, for `wait`.
    asked: u64,
}

#[derive(Default)]
struct Server {
    folders: Vec<RemoteFolder>,
    messages: Vec<Held>,
    gone: Vec<Gone>,
    /// Counts every change. A cursor is its value at that `changes`.
    clock: u64,
    sent: Vec<Sent>,
}

struct Held {
    folder: String,
    message: RemoteMessage,
    raw: Vec<u8>,
    /// The clock when it came into its folder, and when its flags last changed.
    added: u64,
    changed: u64,
}

/// A message that left `folder`.
struct Gone {
    folder: String,
    uid: String,
    added: u64,
    at: u64,
}

/// A message that went through [`Backend::send`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub from: String,
    pub to: Vec<String>,
    pub raw: Vec<u8>,
}

impl Fake {
    pub fn add_folder(&self, path: &str, role: Role) {
        self.change(|server, _| server.folders.push(RemoteFolder { path: path.into(), name: path.into(), role }));
    }

    /// A message arrives in `folder`. Returns its uid.
    pub fn deliver(&self, folder: &str, raw: &[u8], flags: Flags) -> String {
        self.change(|server, now| server.hold(folder, raw.to_vec(), flags, None, now))
    }

    pub fn sent(&self) -> Vec<Sent> {
        self.server.0.lock().unwrap().sent.clone()
    }

    /// Runs `change` at the next tick of the clock and wakes every `wait`.
    fn change<T>(&self, change: impl FnOnce(&mut Server, u64) -> T) -> T {
        let mut server = self.server.0.lock().unwrap();
        server.clock += 1;
        let now = server.clock;
        let out = change(&mut server, now);
        self.server.1.notify_all();
        out
    }
}

impl Server {
    fn hold(&mut self, folder: &str, raw: Vec<u8>, flags: Flags, thread: Option<String>, now: u64) -> String {
        let uid = now.to_string();
        let message = RemoteMessage { uid: uid.clone(), flags, header: header_of(&raw), thread };
        self.messages.push(Held { folder: folder.into(), message, raw, added: now, changed: now });
        uid
    }

    fn position(&self, folder: &str, uid: &str) -> Result<usize> {
        self.messages
            .iter()
            .position(|m| m.folder == folder && m.message.uid == uid)
            .ok_or_else(|| Error(format!("no message {uid} in {folder}")))
    }

    /// To change them: the change is stamped `now`.
    fn flags(&mut self, folder: &str, uid: &str, now: u64) -> Result<&mut Flags> {
        let at = self.position(folder, uid)?;
        let held = &mut self.messages[at];
        held.changed = now;
        Ok(&mut held.message.flags)
    }

    fn take(&mut self, folder: &str, uid: &str, now: u64) -> Result<Held> {
        let held = self.messages.remove(self.position(folder, uid)?);
        self.gone.push(Gone { folder: folder.into(), uid: uid.into(), added: held.added, at: now });
        Ok(held)
    }
}

/// Up to and including the blank line that ends the headers.
fn header_of(raw: &[u8]) -> Vec<u8> {
    let end = (0..raw.len())
        .find_map(|i| [&b"\r\n\r\n"[..], b"\n\n"].into_iter().find(|blank| raw[i..].starts_with(blank)).map(|blank| i + blank.len()));
    raw[..end.unwrap_or(raw.len())].to_vec()
}

impl Backend for Fake {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>> {
        Ok(self.server.0.lock().unwrap().folders.clone())
    }

    fn changes(&mut self, folder: &str, cursor: Option<&str>) -> Result<Changes> {
        let server = self.server.0.lock().unwrap();
        let since: u64 = match cursor {
            Some(cursor) => cursor.parse().map_err(|_| Error(format!("not a cursor of this server: {cursor}")))?,
            None => 0,
        };
        self.asked = server.clock;
        let held = || server.messages.iter().filter(|m| m.folder == folder);
        Ok(Changes {
            new: held().filter(|m| m.added > since).map(|m| m.message.clone()).collect(),
            changed: held()
                .filter(|m| m.added <= since && m.changed > since)
                .map(|m| (m.message.uid.clone(), m.message.flags))
                .collect(),
            removed: server
                .gone
                .iter()
                .filter(|g| g.folder == folder && g.added <= since && g.at > since)
                .map(|g| g.uid.clone())
                .collect(),
            reset: false,
            cursor: server.clock.to_string(),
        })
    }

    fn fetch_body(&mut self, folder: &str, uid: &str) -> Result<Vec<u8>> {
        let server = self.server.0.lock().unwrap();
        Ok(server.messages[server.position(folder, uid)?].raw.clone())
    }

    fn apply(&mut self, folder: &str, uid: &str, op: &Op) -> Result<()> {
        self.change(|server, now| {
            match op {
                Op::Seen(on) => server.flags(folder, uid, now)?.seen = *on,
                Op::Flagged(on) => server.flags(folder, uid, now)?.flagged = *on,
                Op::Move(to) => {
                    let held = server.take(folder, uid, now)?;
                    server.hold(to, held.raw, held.message.flags, held.message.thread, now);
                }
                Op::Delete => drop(server.take(folder, uid, now)?),
            }
            Ok(())
        })
    }

    fn wait(&mut self, _folder: &str, timeout: Duration) -> Result<bool> {
        let (server, changed) = &*self.server;
        let (server, _) = changed.wait_timeout_while(server.lock().unwrap(), timeout, |s| s.clock == self.asked).unwrap();
        Ok(server.clock != self.asked)
    }

    fn append(&mut self, folder: &str, raw: &[u8], flags: Flags) -> Result<()> {
        self.deliver(folder, raw, flags);
        Ok(())
    }

    fn send(&mut self, from: &str, to: &[String], raw: &[u8]) -> Result<()> {
        self.change(|server, _| server.sent.push(Sent { from: from.into(), to: to.to_vec(), raw: raw.to_vec() }));
        Ok(())
    }
}
