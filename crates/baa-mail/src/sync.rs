//! One thread an account (PLAN-mail.md §4 "Threads"): it owns the account's backend and a
//! write handle on mail.db. The UI thread changes mail.db first and queues what the server
//! must hear; this thread replays the queue, sends the outbox, keeps the folders current
//! (IDLE on INBOX, the rest every 10 minutes) and fetches bodies asked for.

use std::{
    sync::{
        mpsc::{Receiver, RecvTimeoutError, Sender},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crate::{
    backend::Backend,
    secrets::Secrets,
    store::{StoredFolder, Store},
    Account, Connect, Error, Event, FolderId, Flags, MessageId, Result, Role, SyncState,
};

/// Folders other than INBOX are asked for changes this often.
const POLL: Duration = Duration::from_secs(10 * 60);
/// At most this many bodies are fetched for codes in one sync of a folder, so a burst of new
/// mail cannot hold the thread.
const CODE_BODIES: usize = 20;
/// How long the thread waits in IDLE before it looks at its jobs again.
const STEP: Duration = if cfg!(test) { Duration::from_millis(20) } else { Duration::from_secs(1) };
/// Back-off after a failed connection: doubling from the first to the last.
const BACKOFF: (Duration, Duration) = if cfg!(test) {
    (Duration::from_millis(20), Duration::from_millis(80))
} else {
    (Duration::from_secs(5), Duration::from_secs(15 * 60))
};

pub(crate) enum Job {
    /// The op queue or the outbox has more: replay them.
    Wake,
    /// Look at INBOX now.
    Check,
    /// Sync this folder now.
    Folder(FolderId),
    Body(MessageId),
    Stop,
}

pub(crate) struct Shared {
    pub connect: Connect,
    pub secrets: Box<dyn Secrets>,
    pub notify: Box<dyn Fn(Event) + Send + Sync>,
}

/// Dropping it stops the thread at its next step, without waiting: a thread may be inside a
/// connect or a send for a while, and the UI thread drops it.
pub(crate) struct AccountThread {
    pub jobs: Sender<Job>,
    handle: JoinHandle<()>,
}

impl AccountThread {
    pub(crate) fn start(account: Account, store: Store, shared: Arc<Shared>) -> AccountThread {
        let (jobs, rx) = std::sync::mpsc::channel();
        let handle = std::thread::Builder::new()
            .name(format!("mail-{}", account.id.0))
            .spawn(move || run(&Thread { account, store, shared, jobs: rx }))
            .expect("a thread");
        AccountThread { jobs, handle }
    }

    pub(crate) fn running(&self) -> bool {
        !self.handle.is_finished()
    }
}

impl Drop for AccountThread {
    fn drop(&mut self) {
        let _ = self.jobs.send(Job::Stop);
    }
}

struct Thread {
    account: Account,
    store: Store,
    shared: Arc<Shared>,
    jobs: Receiver<Job>,
}

struct Stopped;

impl Thread {
    fn notify(&self, event: Event) {
        (self.shared.notify)(event)
    }

    fn status(&self, state: SyncState) {
        self.notify(Event::Status { account: self.account.id, state });
    }
}

fn run(t: &Thread) {
    let mut backoff = BACKOFF.0;
    loop {
        t.status(SyncState::Syncing);
        let outcome = (t.shared.connect)(&t.account, &*t.shared.secrets).and_then(|mut backend| {
            backoff = BACKOFF.0;
            online(t, &mut *backend)
        });
        let error = match outcome {
            Ok(Stopped) => return,
            Err(error) => error,
        };
        // No host, address or secret above debug (PLAN-mail.md §8).
        log::debug!("mail: account {} is offline: {error}", t.account.id.0);
        log::info!("mail: account {} is offline", t.account.id.0);
        let wait = match error {
            // The stored sign-in is refused: asking again with it changes nothing until the
            // user signs in again, or asks.
            Error::SignIn | Error::SignInAgain | Error::BasicAuthOff => {
                t.status(SyncState::SignInAgain);
                None
            }
            _ => {
                t.status(SyncState::Offline);
                let wait = backoff;
                backoff = (backoff * 2).min(BACKOFF.1);
                Some(wait)
            }
        };
        if offline(t, wait).is_err() {
            return;
        }
    }
}

/// Waits out `wait` (`None`: until asked) while offline. A check ends it early; the queue
/// and the outbox wait in mail.db, and a body asked for is asked for again by the next read.
fn offline(t: &Thread, wait: Option<Duration>) -> std::result::Result<(), Stopped> {
    let until = wait.map(|wait| Instant::now() + wait);
    loop {
        let job = match until {
            Some(until) => match t.jobs.recv_timeout(until.saturating_duration_since(Instant::now())) {
                Err(RecvTimeoutError::Timeout) => return Ok(()),
                job => job.map_err(|_| ()),
            },
            None => t.jobs.recv().map_err(|_| ()),
        };
        match job {
            Ok(Job::Stop) | Err(()) => return Err(Stopped),
            Ok(Job::Check | Job::Folder(_)) => return Ok(()),
            Ok(_) => {}
        }
    }
}

fn online(t: &Thread, backend: &mut dyn Backend) -> Result<Stopped> {
    replay(t, backend)?;
    // Folders first: a message waiting from before is filed in a Sent folder mail.db knows.
    sync_all(t, backend)?;
    send_outbox(t, backend)?;
    t.status(SyncState::Synced);
    let mut polled = Instant::now();
    loop {
        loop {
            match t.jobs.try_recv() {
                Ok(Job::Stop) | Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(Stopped),
                Ok(Job::Wake) => {
                    replay(t, backend)?;
                    send_outbox(t, backend)?;
                }
                Ok(Job::Check) => sync_role(t, backend, Role::Inbox)?,
                Ok(Job::Folder(id)) => {
                    if let Some(folder) = t.store.folder(id)? {
                        sync_folder(t, backend, &folder)?;
                    }
                }
                Ok(Job::Body(id)) => fetch_body(t, backend, id)?,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }
        if polled.elapsed() >= POLL {
            sync_all(t, backend)?;
            polled = Instant::now();
        }
        let Some(inbox) = t.store.folder_by_role(t.account.id, Role::Inbox)? else {
            // No INBOX to IDLE on: the jobs and the next poll still come.
            std::thread::sleep(STEP);
            continue;
        };
        if backend.wait(&inbox.path, STEP)? {
            sync_folder(t, backend, &inbox)?;
        }
    }
}

/// The user's actions the server has not heard yet, oldest first. An op the server refuses
/// (its message is gone there) is dropped; a lost connection leaves the rest queued.
fn replay(t: &Thread, backend: &mut dyn Backend) -> Result<()> {
    for pending in t.store.pending_ops(t.account.id)? {
        match backend.apply(&pending.folder, &pending.uid, &pending.op) {
            Ok(()) => {}
            Err(Error::Offline) => return Err(Error::Offline),
            Err(error) => log::debug!("mail: an op was refused and dropped: {error}"),
        }
        t.store.op_done(pending.id)?;
    }
    Ok(())
}

/// Sends what waits in the outbox. A sent message is filed in Sent, but not for a provider
/// that files it itself. A failed send stays in the outbox for the next connection.
fn send_outbox(t: &Thread, backend: &mut dyn Backend) -> Result<()> {
    for out in t.store.outbox(t.account.id)? {
        match backend.send(&out.from, &out.to, &out.raw) {
            Ok(()) => {}
            Err(Error::Offline) => return Err(Error::Offline),
            Err(error) => {
                t.store.outbox_tried(out.id)?;
                t.notify(Event::Failed { account: t.account.id, what: error.to_string() });
                continue;
            }
        }
        if !t.account.provider.files_sent_mail() {
            if let Some(sent) = t.store.folder_by_role(t.account.id, Role::Sent)? {
                let filed = backend.append(&sent.path, &out.raw, Flags { seen: true, ..Flags::default() });
                if let Err(error) = filed {
                    log::debug!("mail: a sent message was not filed in Sent: {error}");
                }
            }
        }
        t.store.outbox_done(out.id)?;
        if let Some(draft) = out.draft {
            t.notify(Event::Sent(draft));
        }
    }
    Ok(())
}

fn sync_all(t: &Thread, backend: &mut dyn Backend) -> Result<()> {
    let remote = backend.folders()?;
    t.store.put_folders(t.account.id, &remote)?;
    for folder in t.store.account_folders(t.account.id)? {
        sync_folder(t, backend, &folder)?;
    }
    Ok(())
}

fn sync_role(t: &Thread, backend: &mut dyn Backend, role: Role) -> Result<()> {
    match t.store.folder_by_role(t.account.id, role)? {
        Some(folder) => sync_folder(t, backend, &folder),
        None => Ok(()),
    }
}

fn sync_folder(t: &Thread, backend: &mut dyn Backend, folder: &StoredFolder) -> Result<()> {
    let changes = backend.changes(&folder.path, folder.cursor.as_deref())?;
    let changed = changes.reset || !changes.new.is_empty() || !changes.changed.is_empty() || !changes.removed.is_empty();
    t.store.apply_changes(folder.id, &changes)?;
    // A verification code is read from the body, and nobody opens the mail it is in: new mail
    // a code may be in gets its body with the sync. Mail the user wrote or threw away is never
    // read for codes (`Store::code_candidates`).
    if !changes.new.is_empty() && !matches!(folder.role, Role::Sent | Role::Drafts | Role::Trash) {
        for id in t.store.code_mail_without_body(folder.id, CODE_BODIES)? {
            store_body(t, backend, id)?;
        }
    }
    if changed {
        t.notify(Event::Changed { account: t.account.id });
    }
    Ok(())
}

fn fetch_body(t: &Thread, backend: &mut dyn Backend, id: MessageId) -> Result<()> {
    if store_body(t, backend, id)? {
        t.notify(Event::Changed { account: t.account.id });
    }
    Ok(())
}

/// Whether the body is in mail.db now. A body the server will not give is left out; only a
/// lost connection is an error.
fn store_body(t: &Thread, backend: &mut dyn Backend, id: MessageId) -> Result<bool> {
    let Some(at) = t.store.locate(id)? else { return Ok(false) };
    match backend.fetch_body(&at.path, &at.uid) {
        Ok(raw) => {
            t.store.put_body(id, &raw)?;
            Ok(true)
        }
        Err(Error::Offline) => Err(Error::Offline),
        Err(error) => {
            log::debug!("mail: a body was not fetched: {error}");
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests;
