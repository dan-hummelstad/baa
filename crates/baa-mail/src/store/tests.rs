use super::*;
use crate::RemoteMessage;

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
