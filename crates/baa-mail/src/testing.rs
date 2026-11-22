//! For other crates' tests (feature `test-support`): puts accounts, folders and mail into a
//! [`Mail`] as an account's sync would, and raises the events sync raises.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::{
    provider::Provider, Account, AccountId, Changes, Event, Flags, FolderId, Mail, MailSpace, MessageId, RemoteFolder,
    RemoteMessage, Role, SignIn,
};

/// An account at Fastmail of `space` (`None`: every Space), offering codes.
pub fn account(id: u64, space: Option<MailSpace>, address: &str) -> Account {
    let provider = Provider::Fastmail;
    let servers = provider.servers(address).expect("a preset");
    Account {
        id: AccountId(id),
        space: space.map(MailSpace::id),
        name: "Dan".into(),
        address: address.into(),
        codes: true,
        provider,
        servers,
        sign_in: SignIn::Password,
    }
}

/// An RFC 822 message with a Message-ID of its own; `extra` is more header lines, each
/// ending in `\r\n`.
pub fn message(from: &str, to: &str, subject: &str, date: u64, extra: &str, body: &str) -> Vec<u8> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let date = mail_parser::DateTime::from_timestamp(date as i64).to_rfc822();
    format!("From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nDate: {date}\r\nMessage-ID: <seed{id}@test>\r\n{extra}\r\n{body}")
        .into_bytes()
}

impl Mail {
    /// Stores `account` with an Inbox, Drafts, Sent, Archive and Trash; returns their ids in
    /// that order.
    pub fn seed_account(&self, account: &Account) -> Vec<FolderId> {
        self.store().unwrap().put_account(account).unwrap();
        let folders = [("INBOX", "Inbox", Role::Inbox), ("Drafts", "Drafts", Role::Drafts), ("Sent", "Sent", Role::Sent), ("Archive", "Archive", Role::Archive), ("Trash", "Trash", Role::Trash)];
        let folders: Vec<RemoteFolder> = folders.into_iter().map(|(path, name, role)| RemoteFolder { path: path.into(), name: name.into(), role }).collect();
        self.seed_folders(account.id, &folders)
    }

    /// Makes the account's folders `folders`, as a folder list from its server does.
    pub fn seed_folders(&self, account: AccountId, folders: &[RemoteFolder]) -> Vec<FolderId> {
        self.store().unwrap().put_folders(account, folders).unwrap()
    }

    /// `raw` arrives in `folder` under `uid`; with `body` its content is fetched too. Raises
    /// [`Event::Changed`].
    pub fn seed_message(&self, folder: FolderId, uid: &str, raw: &[u8], flags: Flags, body: bool) -> MessageId {
        let store = self.store().unwrap();
        let end = raw.windows(4).position(|w| w == b"\r\n\r\n").map_or(raw.len(), |at| at + 4);
        let new = RemoteMessage { uid: uid.into(), flags, header: raw[..end].to_vec(), thread: None };
        let id = store.apply_changes(folder, &Changes { new: vec![new], ..Changes::default() }).unwrap()[0];
        if body {
            store.put_body(id, raw).unwrap();
        }
        self.changed(folder);
        id
    }

    /// The content of message `id` arrives, as when it is first opened. Raises [`Event::Changed`].
    pub fn seed_body(&self, id: MessageId, raw: &[u8]) {
        let store = self.store().unwrap();
        store.put_body(id, raw).unwrap();
        let folder = store.locate(id).unwrap().expect("a seeded message").folder;
        self.changed(folder);
    }

    /// Raises `event` as an account's thread would.
    pub fn seed_event(&self, event: Event) {
        (self.shared.notify)(event);
    }

    fn changed(&self, folder: FolderId) {
        let account = self.store().unwrap().folder(folder).unwrap().expect("a seeded folder").account;
        (self.shared.notify)(Event::Changed { account });
    }
}
