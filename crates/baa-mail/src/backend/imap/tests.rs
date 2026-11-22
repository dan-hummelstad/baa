//! IMAP against a scripted server over an in-memory pipe: nothing connects anywhere.

use std::{
    io,
    sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender},
    thread::JoinHandle,
};

use super::*;

/// One end of an in-memory byte pipe.
pub(crate) struct Duplex {
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
    pending: Vec<u8>,
    timeout: Option<Duration>,
}

pub(crate) fn pair() -> (Duplex, Duplex) {
    let (a_tx, b_rx) = channel();
    let (b_tx, a_rx) = channel();
    let end = |tx, rx| Duplex { tx, rx, pending: Vec::new(), timeout: None };
    (end(a_tx, a_rx), end(b_tx, b_rx))
}

impl Read for Duplex {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pending.is_empty() {
            let next = match self.timeout {
                Some(timeout) => self.rx.recv_timeout(timeout),
                None => self.rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match next {
                Ok(bytes) => self.pending = bytes,
                Err(RecvTimeoutError::Timeout) => return Err(io::ErrorKind::WouldBlock.into()),
                Err(RecvTimeoutError::Disconnected) => return Ok(0),
            }
        }
        let n = buf.len().min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }
}

impl Write for Duplex {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.tx.send(buf.to_vec()).map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Conn for Duplex {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.timeout = timeout;
        Ok(())
    }
}

/// The server's end, driven line by line by a test's script.
pub(crate) struct Script {
    end: Duplex,
}

impl Script {
    /// Sends `text` as it is: the script writes the `\r\n`s.
    pub(crate) fn send(&mut self, text: &str) {
        self.end.write_all(text.as_bytes()).unwrap();
    }

    /// The client's next line, without its `\r\n`.
    pub(crate) fn line(&mut self) -> String {
        self.end.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        line(&mut self.end).expect("the client sent a line")
    }

    /// The next command, which must start with `start` (after its tag; case as the client
    /// sends it). Returns its tag and the whole line after the tag.
    pub(crate) fn expect(&mut self, start: &str) -> (String, String) {
        let line = self.line();
        let (tag, rest) = line.split_once(' ').expect("a tagged command");
        assert!(rest.starts_with(start), "expected {start:?}, got {line:?}");
        (tag.to_string(), rest.to_string())
    }

    /// Answers the next command, which must start with `start`, with the untagged lines
    /// `untagged` (each without `* ` or `\r\n`) and then `<tag> OK done`.
    pub(crate) fn answer(&mut self, start: &str, untagged: &[&str]) -> String {
        let (tag, rest) = self.expect(start);
        for line in untagged {
            self.send(&format!("* {line}\r\n"));
        }
        self.send(&format!("{tag} OK done\r\n"));
        rest
    }

    /// `n` bytes of a literal the client sends after a `+` from the server.
    pub(crate) fn literal(&mut self, n: usize) -> Vec<u8> {
        self.send("+ go ahead\r\n");
        let mut out = vec![0; n];
        self.end.read_exact(&mut out).unwrap();
        out
    }
}

/// A server running `script` on its own thread, and the client's end of the pipe.
pub(crate) fn server(script: impl FnOnce(&mut Script) + Send + 'static) -> (Duplex, JoinHandle<()>) {
    let (client, end) = pair();
    let handle = std::thread::spawn(move || script(&mut Script { end }));
    (client, handle)
}

/// A server that greets, answers CAPABILITY and LOGIN, and then runs `then`.
pub(crate) fn signed_in(capabilities: &'static str, then: impl FnOnce(&mut Script) + Send + 'static) -> (Session, JoinHandle<()>) {
    let (client, handle) = server(move |s| {
        s.send("* OK IMAP4rev1 ready\r\n");
        s.answer("CAPABILITY", &[capabilities]);
        s.answer("LOGIN", &[]);
        s.answer("CAPABILITY", &[capabilities]);
        then(s);
    });
    let session = Session::sign_in(Box::new(client), false, "dan@example.com", &Credential::Password("app-password".into())).ok().unwrap();
    (session, handle)
}

const CAPS: &str = "CAPABILITY IMAP4rev1 IDLE UIDPLUS MOVE CONDSTORE";

fn greeted(then: impl FnOnce(&mut Script) + Send + 'static) -> (Duplex, JoinHandle<()>) {
    server(move |s| {
        s.send("* OK IMAP4rev1 ready\r\n");
        s.answer("CAPABILITY", &[CAPS]);
        then(s);
    })
}

fn sign_in(conn: Duplex, credential: &Credential) -> Result<Session> {
    Session::sign_in(Box::new(conn), false, "dan@example.com", credential)
}

/// Answers a SELECT; HIGHESTMODSEQ only when `modseq` is not 0.
fn select(s: &mut Script, validity: u32, next: u32, modseq: u64) -> String {
    let mut lines = vec![format!("OK [UIDVALIDITY {validity}] ok"), format!("OK [UIDNEXT {next}] ok"), "5 EXISTS".to_string()];
    if modseq > 0 {
        lines.push(format!("OK [HIGHESTMODSEQ {modseq}] ok"));
    }
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    s.answer("SELECT", &lines)
}

fn header_of(uid: u32, flags: &str) -> String {
    let header = format!("Subject: mail {uid}\r\n\r\n");
    format!("{uid} FETCH (UID {uid} FLAGS ({flags}) RFC822.HEADER {{{}}}\r\n{header})", header.len())
}

fn header_bytes(uid: u32) -> Vec<u8> {
    format!("Subject: mail {uid}\r\n\r\n").into_bytes()
}

fn flags_fetch(uid: u32, flags: &str) -> String {
    format!("{uid} FETCH (UID {uid} FLAGS ({flags}))")
}

fn uids(messages: &[RemoteMessage]) -> Vec<&str> {
    messages.iter().map(|m| m.uid.as_str()).collect()
}

const SEEN: Flags = Flags { seen: true, flagged: false, answered: false };
const FLAGGED: Flags = Flags { seen: false, flagged: true, answered: false };

#[test]
fn a_scripted_server_takes_the_client_from_greeting_to_idle() {
    let body = b"Subject: hi\r\n\r\nHello\r\n".to_vec();
    let served = format!("1 FETCH (UID 7 BODY[] {{{}}}\r\n{})", body.len(), String::from_utf8(body.clone()).unwrap());
    let (client, done) = server(move |s| {
        s.send("* OK IMAP4rev1 ready\r\n");
        s.answer("CAPABILITY", &[CAPS]);
        let login = s.answer("LOGIN", &[]);
        assert!(login.contains("dan@example.com") && login.contains("app-password"), "{login}");
        s.answer("CAPABILITY", &[CAPS]);
        s.answer("LIST", &[r#"LIST (\HasNoChildren) "/" INBOX"#, r#"LIST (\HasNoChildren \Sent) "/" "Sent Items""#]);
        select(s, 3, 9, 0);
        s.answer("UID FETCH 7", &[&served]);
        let (idle, _) = s.expect("IDLE");
        s.send("+ idling\r\n");
        s.send("* 6 EXISTS\r\n");
        assert_eq!(s.line(), "DONE");
        s.send(&format!("{idle} OK idle done\r\n"));
        s.answer("UID FETCH 7", &[&served]);
    });
    let mut session = sign_in(client, &Credential::Password("app-password".into())).ok().unwrap();
    let folders = session.folders().unwrap();
    assert_eq!(folders.len(), 2);
    assert_eq!((folders[0].path.as_str(), folders[0].role), ("INBOX", Role::Inbox));
    assert_eq!((folders[1].name.as_str(), folders[1].role), ("Sent Items", Role::Sent));
    assert_eq!(session.fetch_body("INBOX", "7").unwrap(), body);
    assert!(session.wait("INBOX", Duration::from_secs(5)).unwrap());
    assert_eq!(session.fetch_body("INBOX", "7").unwrap(), body);
    done.join().unwrap();
}

#[test]
fn implicit_tls_to_a_server_that_speaks_plain_text_is_refused() {
    let (client, done) = server(|s| s.send("* OK plain\r\n"));
    assert!(matches!(secure(client, "imap.example.com", Tls::Implicit).err(), Some(Error::NoTls)));
    done.join().unwrap();
}

#[test]
fn a_starttls_port_whose_server_refuses_starttls_is_refused() {
    let (client, done) = server(|s| {
        s.send("* OK hi\r\n");
        let (tag, _) = s.expect("STARTTLS");
        assert_eq!(tag, "a0");
        s.send("a0 BAD unknown command\r\n");
        assert!(line(&mut s.end).is_err(), "the client sent a command after the refusal");
    });
    assert!(matches!(secure(client, "imap.example.com", Tls::StartTls).err(), Some(Error::NoTls)));
    done.join().unwrap();

    let (client, done) = server(|s| {
        s.send("* PREAUTH welcome\r\n");
        assert!(line(&mut s.end).is_err(), "the client sent a command after PREAUTH");
    });
    assert!(matches!(secure(client, "imap.example.com", Tls::StartTls).err(), Some(Error::NoTls)));
    done.join().unwrap();
}

#[test]
fn login_refusals_tell_a_wrong_password_from_basic_auth_being_off() {
    assert!(matches!(sign_in_error("[AUTHENTICATIONFAILED] Invalid credentials (Failure)"), Error::SignIn));
    for text in ["[ALERT] Application-specific password required: https://support.google.com/accounts/answer/185833 (Failure)", "BasicAuthBlocked"] {
        assert!(matches!(sign_in_error(text), Error::BasicAuthOff), "{text}");
    }

    let (client, done) = greeted(|s| {
        let (tag, _) = s.expect("LOGIN");
        s.send(&format!("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n"));
    });
    assert!(matches!(sign_in(client, &Credential::Password("wrong".into())).err(), Some(Error::SignIn)));
    done.join().unwrap();
}

#[test]
fn xoauth2_sends_the_bearer_and_a_refusal_reads_as_sign_in() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let (client, done) = greeted(|s| {
        let (tag, rest) = s.expect("AUTHENTICATE XOAUTH2 ");
        let encoded = rest.strip_prefix("AUTHENTICATE XOAUTH2 ").unwrap();
        assert_eq!(STANDARD.decode(encoded).unwrap(), b"user=dan@example.com\x01auth=Bearer tok\x01\x01");
        s.send(&format!("{tag} OK done\r\n"));
        s.answer("CAPABILITY", &[CAPS]);
    });
    assert!(sign_in(client, &Credential::Bearer("tok".into())).is_ok());
    done.join().unwrap();

    let (client, done) = greeted(|s| {
        let (tag, _) = s.expect("AUTHENTICATE XOAUTH2 ");
        s.send("+ eyJzdGF0dXMiOiI0MDAifQ==\r\n");
        assert_eq!(s.line(), "");
        s.send(&format!("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n"));
    });
    assert!(matches!(sign_in(client, &Credential::Bearer("tok".into())).err(), Some(Error::SignIn)));
    done.join().unwrap();
}

#[test]
fn a_first_sync_fetches_the_headers_of_the_last_30_days() {
    let (mut session, done) = signed_in(CAPS, |s| {
        select(s, 3, 12, 90);
        let search = s.answer("UID SEARCH SINCE", &["SEARCH 4 5 6 9"]);
        let ago = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap() - FIRST_SYNC;
        // The client asked a moment ago, maybe before midnight.
        let day = |secs: u64| chrono::DateTime::from_timestamp(secs as i64, 0).unwrap().format("UID SEARCH SINCE \"%d-%b-%Y\"").to_string();
        assert!([day(ago.as_secs()), day(ago.as_secs() - 5)].contains(&search), "{search}");
        let fetch = s.answer("UID FETCH", &[&header_of(4, ""), &header_of(5, r"\Seen"), &header_of(6, ""), &header_of(9, "")]);
        assert_eq!(fetch, "UID FETCH 4:6,9 (UID FLAGS RFC822.HEADER)");
    });
    let changes = session.changes("INBOX", None).unwrap();
    assert_eq!(uids(&changes.new), ["4", "5", "6", "9"]);
    assert_eq!(changes.new[1].flags, SEEN);
    assert_eq!(changes.new[0].flags, Flags::default());
    assert_eq!(changes.new[3].header, header_bytes(9));
    assert!(!changes.reset);
    assert_eq!(changes.cursor, "3:12:90:4:6,9");
    done.join().unwrap();
}

#[test]
fn a_later_sync_takes_new_mail_changed_flags_and_removals() {
    let (mut session, done) = signed_in(CAPS, |s| {
        let line = select(s, 3, 14, 95);
        assert_eq!(line, "SELECT INBOX (CONDSTORE)");
        let fetch = s.answer("UID FETCH", &[&header_of(12, ""), &header_of(13, r"\Seen")]);
        assert_eq!(fetch, "UID FETCH 12:* (UID FLAGS RFC822.HEADER)");
        let fetch = s.answer("UID FETCH", &[&flags_fetch(5, r"\Flagged")]);
        assert_eq!(fetch, "UID FETCH 4:11 (UID FLAGS) (CHANGEDSINCE 90)");
        assert_eq!(s.answer("UID SEARCH", &["SEARCH 4 5 9"]), "UID SEARCH UID 4:11");
    });
    let changes = session.changes("INBOX", Some("3:12:90:4:6,9")).unwrap();
    assert_eq!(uids(&changes.new), ["12", "13"]);
    assert_eq!(changes.changed, [("5".to_string(), FLAGGED)]);
    assert_eq!(changes.removed, ["6"]);
    assert!(!changes.reset);
    assert_eq!(changes.cursor, "3:14:95:4:5,9,12:13");
    done.join().unwrap();
}

#[test]
fn without_condstore_a_later_sync_reads_every_known_message_flags() {
    let (mut session, done) = signed_in("CAPABILITY IMAP4rev1 IDLE UIDPLUS MOVE", |s| {
        assert_eq!(select(s, 3, 12, 0), "SELECT INBOX");
        let fetch = s.answer("UID FETCH", &[&flags_fetch(4, ""), &flags_fetch(5, r"\Seen"), &flags_fetch(6, ""), &flags_fetch(9, r"\Flagged")]);
        assert_eq!(fetch, "UID FETCH 4:11 (UID FLAGS)");
        s.answer("UID SEARCH", &["SEARCH 4 5 6 9"]);
    });
    let changes = session.changes("INBOX", Some("3:12:0:4:6,9")).unwrap();
    assert!(changes.new.is_empty() && changes.removed.is_empty());
    let expected = [("4", Flags::default()), ("5", SEEN), ("6", Flags::default()), ("9", FLAGGED)];
    assert_eq!(changes.changed, expected.map(|(uid, flags)| (uid.to_string(), flags)));
    done.join().unwrap();
}

#[test]
fn a_changed_uidvalidity_refills_the_folder() {
    let (mut session, done) = signed_in(CAPS, |s| {
        select(s, 4, 3, 7);
        s.answer("UID SEARCH SINCE", &["SEARCH 1 2"]);
        s.answer("UID FETCH 1:2", &[&header_of(1, ""), &header_of(2, "")]);
    });
    let changes = session.changes("INBOX", Some("3:12:90:4:6,9")).unwrap();
    assert!(changes.reset);
    assert_eq!(uids(&changes.new), ["1", "2"]);
    assert!(changes.removed.is_empty() && changes.changed.is_empty());
    assert_eq!(changes.cursor, "4:3:7:1:2");
    done.join().unwrap();
}

#[test]
fn a_move_uses_move_where_offered_and_copy_then_expunge_where_not() {
    let (mut session, done) = signed_in(CAPS, |s| {
        select(s, 3, 9, 90);
        assert_eq!(s.answer("UID MOVE", &[]), "UID MOVE 7 Archive");
        assert_eq!(s.answer("UID STORE", &[]), r"UID STORE 7 +FLAGS.SILENT (\Seen)");
        assert_eq!(s.answer("UID STORE", &[]), r"UID STORE 7 -FLAGS.SILENT (\Flagged)");
    });
    session.apply("INBOX", "7", &Op::Move("Archive".into())).unwrap();
    session.apply("INBOX", "7", &Op::Seen(true)).unwrap();
    session.apply("INBOX", "7", &Op::Flagged(false)).unwrap();
    done.join().unwrap();

    let (mut session, done) = signed_in("CAPABILITY IMAP4rev1 UIDPLUS", |s| {
        select(s, 3, 9, 0);
        assert_eq!(s.answer("UID COPY", &[]), "UID COPY 7 Archive");
        assert_eq!(s.answer("UID STORE", &[]), r"UID STORE 7 +FLAGS.SILENT (\Deleted)");
        assert_eq!(s.answer("UID EXPUNGE", &[]), "UID EXPUNGE 7");
    });
    session.apply("INBOX", "7", &Op::Move("Archive".into())).unwrap();
    done.join().unwrap();

    let (mut session, done) = signed_in("CAPABILITY IMAP4rev1", |s| {
        select(s, 3, 9, 0);
        s.answer("UID COPY", &[]);
        s.answer("UID STORE", &[]);
        assert_eq!(s.answer("EXPUNGE", &[]), "EXPUNGE");
    });
    session.apply("INBOX", "7", &Op::Move("Archive".into())).unwrap();
    done.join().unwrap();
}

#[test]
fn append_sends_the_message_as_a_literal_with_its_flags() {
    let raw = b"Subject: draft\r\n\r\nText\r\n".to_vec();
    let sent = raw.clone();
    let (mut session, done) = signed_in(CAPS, move |s| {
        let (tag, rest) = s.expect("APPEND");
        assert_eq!(rest, format!(r"APPEND Sent (\Seen) {{{}}}", sent.len()));
        assert_eq!(s.literal(sent.len()), sent);
        assert_eq!(s.line(), "");
        s.send(&format!("{tag} OK done\r\n"));
    });
    session.append("Sent", &raw, SEEN).unwrap();
    done.join().unwrap();
}

#[test]
fn folders_skip_noselect_and_name_gmail_all_mail_the_archive() {
    let (mut session, done) = signed_in(CAPS, |s| {
        s.answer(
            "LIST",
            &[
                r#"LIST (\HasChildren \Noselect) "/" "[Gmail]""#,
                r#"LIST (\All \HasNoChildren) "/" "[Gmail]/All Mail""#,
                r#"LIST (\HasNoChildren \Trash) "/" "[Gmail]/Trash""#,
                r#"LIST (\HasNoChildren) "/" "Entw&APw-rfe""#,
                r#"LIST (\HasNoChildren) "/" INBOX"#,
            ],
        );
    });
    let folders = session.folders().unwrap();
    let got: Vec<_> = folders.iter().map(|f| (f.path.as_str(), f.name.as_str(), f.role)).collect();
    let expected = [
        ("[Gmail]/All Mail", "All Mail", Role::Archive),
        ("[Gmail]/Trash", "Trash", Role::Trash),
        ("Entw&APw-rfe", "Entwürfe", Role::Other),
        ("INBOX", "INBOX", Role::Inbox),
    ];
    assert_eq!(got, expected);
    done.join().unwrap();
}

#[test]
fn wait_times_out_without_a_change_and_keeps_its_idle() {
    let (mut session, done) = signed_in(CAPS, |s| {
        select(s, 3, 9, 90);
        let (idle, _) = s.expect("IDLE");
        s.send("+ idling\r\n");
        assert_eq!(s.line(), "DONE");
        s.send(&format!("{idle} OK idle done\r\n"));
        s.answer("UID FETCH 7", &["1 FETCH (UID 7 BODY[] {2}\r\nhi)"]);
    });
    let step = Duration::from_millis(200);
    assert!(!session.wait("INBOX", step).unwrap());
    assert!(!session.wait("INBOX", step).unwrap());
    assert_eq!(session.fetch_body("INBOX", "7").unwrap(), b"hi");
    done.join().unwrap();

    let (mut session, done) = signed_in(CAPS, |s| {
        select(s, 3, 9, 90);
        s.expect("IDLE");
        s.send("+ idling\r\n* BYE shutting down\r\n");
    });
    assert!(session.wait("INBOX", Duration::from_secs(5)).is_err());
    done.join().unwrap();
}

#[test]
fn small_helpers_hold() {
    assert_eq!(uid_set(&[1, 2, 3, 7, 9, 10]), "1:3,7,9:10");
    assert_eq!(uid_set(&[5]), "5");

    let cursor = Cursor::parse("3:12:90:4:6,9").unwrap();
    assert_eq!((cursor.validity, cursor.next, cursor.modseq), (3, 12, 90));
    assert_eq!(cursor.uids, [4, 5, 6, 9]);
    assert!(Cursor::parse("3:12:90:").unwrap().uids.is_empty());
    assert!(Cursor::parse("3:12").is_none() && Cursor::parse("x").is_none());

    assert_eq!(decode_utf7("&ZeVnLIqe-"), "日本語");
    assert_eq!(decode_utf7("A&-B"), "A&B");
    assert_eq!(decode_utf7("Sent Items"), "Sent Items");
    assert_eq!(decode_utf7("&ZeV"), "&ZeV");
}
