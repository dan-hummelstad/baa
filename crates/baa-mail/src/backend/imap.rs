//! IMAP on a blocking socket under rustls (PLAN-mail.md §4): imap-next frames the protocol,
//! this file drives it. One connection an account, owned by its sync thread.

use std::{
    borrow::Cow,
    collections::HashSet,
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    num::NonZeroU64,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use imap_next::{
    client::{Client, Event, Options},
    imap_types::{
        auth::{AuthMechanism, AuthenticateData},
        command::{Command, CommandBody, FetchModifier, SelectParameter},
        core::{Tag, Vec1},
        fetch::{MacroOrMessageDataItemNames, MessageDataItem, MessageDataItemName},
        flag::{Flag, FlagFetch, StoreResponse, StoreType},
        mailbox::Mailbox,
        response::{Code, Data, GreetingKind, Status, StatusKind},
        search::SearchKey,
        secret::Secret as Hidden,
        sequence::SequenceSet,
    },
    Interrupt, Io, State,
};
use rustls::{pki_types::ServerName, ClientConfig, ClientConnection, StreamOwned};
use rustls_platform_verifier::BuilderVerifierExt;

use super::{Backend, Changes, Flags, Op, RemoteFolder, RemoteMessage};
use crate::{
    account::{Protocol, Server, SignIn, Tls},
    oauth,
    secrets::{Secret, Secrets},
    send::{self, Login},
    Account, Error, Result, Role,
};

/// How far back a first sync of a folder reaches; older mail is not fetched.
const FIRST_SYNC: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// How long a command may wait for the server.
const ANSWER: Duration = Duration::from_secs(60);
/// RFC 2177: a server may drop an IDLE after 30 minutes.
const IDLE_RENEW: Duration = Duration::from_secs(25 * 60);
/// UIDs a FETCH names at most, so a command line stays short.
const CHUNK: usize = 500;

/// A byte stream with a read timeout: a socket, TLS over one, or a test's pipe.
pub(crate) trait Conn: Read + Write + Send {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()>;
}

impl Conn for TcpStream {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        TcpStream::set_read_timeout(self, timeout)
    }
}

impl<S: Conn> Conn for StreamOwned<ClientConnection, S> {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.sock.set_read_timeout(timeout)
    }
}

/// The secret a connection signs in with. No `Debug`.
pub(crate) enum Credential {
    Password(String),
    /// An OAuth access token, for XOAUTH2.
    Bearer(String),
}

/// The real connector [`crate::Mail::open`] gives the store: each account's backend from its
/// stored servers and secrets. A Google account gets a fresh access token at each connect.
// ponytail: one token refresh a connect, not a cached token with its expiry.
pub(crate) fn connector(dir: PathBuf) -> crate::Connect {
    Box::new(move |account, secrets| Ok(Box::new(Imap::connect(account, secrets, &dir)?)))
}

/// An account's IMAP server for sync, and its SMTP server for [`Backend::send`].
pub(crate) struct Imap {
    session: Session,
    smtp: Server,
    user: String,
    credential: Credential,
    dir: PathBuf,
}

impl Imap {
    fn connect(account: &Account, secrets: &dyn Secrets, dir: &Path) -> Result<Imap> {
        let stored = |kind| secrets.get(account.id, kind).ok_or(Error::SignInAgain);
        let credential = match account.sign_in {
            SignIn::Password => Credential::Password(stored(Secret::Password)?),
            SignIn::Google => Credential::Bearer(oauth::refresh(&stored(Secret::RefreshToken)?)?),
        };
        let session = Session::connect(&account.servers.imap, &account.servers.login, &credential)?;
        Ok(Imap { session, smtp: account.servers.smtp.clone(), user: account.servers.login.clone(), credential, dir: dir.into() })
    }
}

impl Imap {
    fn login(&self) -> Login<'_> {
        match &self.credential {
            Credential::Password(password) => Login::Password { user: &self.user, password },
            Credential::Bearer(token) => Login::Bearer { user: &self.user, token },
        }
    }
}

impl Backend for Imap {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>> {
        self.session.folders()
    }

    fn changes(&mut self, folder: &str, cursor: Option<&str>) -> Result<Changes> {
        self.session.changes(folder, cursor)
    }

    fn fetch_body(&mut self, folder: &str, uid: &str) -> Result<Vec<u8>> {
        self.session.fetch_body(folder, uid)
    }

    fn apply(&mut self, folder: &str, uid: &str, op: &Op) -> Result<()> {
        self.session.apply(folder, uid, op)
    }

    fn wait(&mut self, folder: &str, timeout: Duration) -> Result<bool> {
        self.session.wait(folder, timeout)
    }

    fn append(&mut self, folder: &str, raw: &[u8], flags: Flags) -> Result<()> {
        self.session.append(folder, raw, flags)
    }

    fn send(&mut self, from: &str, to: &[String], raw: &[u8]) -> Result<()> {
        send::smtp(&self.smtp, &self.login(), from, to, raw, &self.dir)
    }

    fn check_send(&mut self) -> Result<()> {
        send::smtp_check(&self.smtp, &self.login())
    }
}


pub(crate) struct Session {
    conn: Box<dyn Conn>,
    client: Client,
    capabilities: Vec<String>,
    tag: u32,
    selected: Option<Selected>,
    /// The IDLE under way, with when it began.
    idle: Option<(Tag<'static>, Instant)>,
    /// While `Some`, the responses the codec could not read are kept here.
    skipped: Option<Vec<Vec<u8>>>,
}

#[derive(Clone)]
struct Selected {
    path: String,
    validity: u32,
    next: u32,
    modseq: u64,
}

/// Where a folder's sync got to: `UIDVALIDITY:UIDNEXT:HIGHESTMODSEQ:UIDS`, HIGHESTMODSEQ 0
/// without CONDSTORE. UIDS are the messages reported so far, as a uid set: without QRESYNC
/// the server cannot say what left, so the client compares.
// ponytail: UIDS grows with every gap deletions leave; fine for 30 days of mail a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cursor {
    validity: u32,
    next: u32,
    modseq: u64,
    uids: Vec<u32>,
}

impl Cursor {
    fn parse(text: &str) -> Option<Cursor> {
        let mut parts = text.splitn(4, ':');
        let mut number = || parts.next()?.parse::<u64>().ok();
        let (validity, next, modseq) = (number()?, number()?, number()?);
        let uids = parse_uid_set(parts.next()?)?;
        Some(Cursor { validity: validity.try_into().ok()?, next: next.try_into().ok()?, modseq, uids })
    }
}

fn lost() -> Error {
    Error::Offline
}

impl Session {
    pub(crate) fn connect(server: &Server, user: &str, credential: &Credential) -> Result<Session> {
        let tls = server.check(Protocol::Imap)?;
        let unreachable = || Error::Offline;
        let addr = (server.host.as_str(), server.port).to_socket_addrs().map_err(|_| unreachable())?.next().ok_or_else(unreachable)?;
        let socket = TcpStream::connect_timeout(&addr, Duration::from_secs(20)).map_err(|_| unreachable())?;
        socket.set_read_timeout(Some(ANSWER)).map_err(|_| unreachable())?;
        let conn = secure(socket, &server.host, tls)?;
        Session::sign_in(Box::new(conn), tls == Tls::StartTls, user, credential)
    }

    /// Signs in on a connection that is already secure. After STARTTLS the greeting was read
    /// before TLS began.
    pub(crate) fn sign_in(conn: Box<dyn Conn>, greeted: bool, user: &str, credential: &Credential) -> Result<Session> {
        let mut options = Options::default();
        options.discard_greeting = greeted;
        let client = Client::new(options);
        let mut session = Session { conn, client, capabilities: Vec::new(), tag: 0, selected: None, idle: None, skipped: None };
        if !greeted {
            loop {
                if let Event::GreetingReceived { greeting } = session.next(ANSWER)?.ok_or_else(lost)? {
                    match greeting.kind {
                        GreetingKind::Ok => break,
                        // PREAUTH skips the sign-in this account was added with; BYE is a refusal.
                        _ => return Err(Error::Other("The mail server did not accept a connection".into())),
                    }
                }
            }
        }
        session.capabilities = session.capabilities()?;
        let refused = match credential {
            Credential::Password(password) => {
                let body = CommandBody::login(user.to_string(), password.clone())
                    .map_err(|_| Error::Other("The user name or password holds a character IMAP cannot send".into()))?;
                session.exec(body)?.err().map(|text| sign_in_error(&text))
            }
            Credential::Bearer(token) => session.xoauth2(user, token)?.then_some(Error::SignIn),
        };
        if let Some(error) = refused {
            return Err(error);
        }
        session.capabilities = session.capabilities()?;
        Ok(session)
    }

    fn has(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c.eq_ignore_ascii_case(capability))
    }

    /// The next event; `None` when nothing came within `wait`.
    fn next(&mut self, wait: Duration) -> Result<Option<Event>> {
        loop {
            match self.client.next() {
                Ok(event) => return Ok(Some(event)),
                Err(Interrupt::Io(Io::Output(bytes))) => {
                    self.conn.write_all(&bytes).and_then(|()| self.conn.flush()).map_err(|_| lost())?
                }
                Err(Interrupt::Io(Io::NeedMoreInput)) => {
                    self.conn.set_read_timeout(Some(wait)).map_err(|_| lost())?;
                    let mut buffer = [0; 16 * 1024];
                    match self.conn.read(&mut buffer) {
                        Ok(0) => return Err(lost()),
                        Ok(n) => self.client.enqueue_input(&buffer[..n]),
                        Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => return Ok(None),
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => return Err(lost()),
                    }
                }
                // The codec dropped a response it could not read (a server's own extension):
                // the next one starts after it.
                Err(Interrupt::Error(imap_next::client::Error::MalformedMessage { discarded_bytes })) if self.skipped.is_some() => {
                    self.skipped.as_mut().expect("checked").push(discarded_bytes.declassify().to_vec());
                }
                Err(Interrupt::Error(e)) => log::debug!("imap: skipped a response: {e:?}"),
            }
        }
    }

    fn next_tag(&mut self) -> Tag<'static> {
        self.tag += 1;
        Tag::try_from(format!("b{}", self.tag)).expect("letters and digits are a tag")
    }

    /// Runs one command. `Ok(Err(text))`: the server refused it, saying `text`.
    fn exec(&mut self, body: CommandBody<'static>) -> Result<std::result::Result<Vec<Answer>, String>> {
        self.stop_idle()?;
        let tag = self.next_tag();
        self.client.enqueue_command(Command { tag: tag.clone(), body });
        let mut answers = Vec::new();
        loop {
            match self.next(ANSWER)?.ok_or_else(lost)? {
                Event::DataReceived { data } => answers.push(Answer::Data(data)),
                Event::StatusReceived { status: Status::Untagged(body) } => answers.extend(body.code.map(Answer::Code)),
                Event::StatusReceived { status: Status::Tagged(done) } if done.tag == tag => {
                    return Ok(match done.body.kind {
                        StatusKind::Ok => {
                            answers.extend(done.body.code.map(Answer::Code));
                            Ok(answers)
                        }
                        StatusKind::No | StatusKind::Bad => Err(done.body.text.to_string()),
                    });
                }
                Event::StatusReceived { status: Status::Bye(_) } => return Err(lost()),
                Event::CommandRejected { status, .. } => return Ok(Err(status_text(&status))),
                _ => {}
            }
        }
    }

    fn run(&mut self, body: CommandBody<'static>) -> Result<Vec<Answer>> {
        let name = body.name();
        self.exec(body)?.map_err(|text| {
            log::debug!("imap: {name} refused: {text}");
            Error::Other(format!("The mail server refused {name}: {text}"))
        })
    }

    fn capabilities(&mut self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for answer in self.run(CommandBody::Capability)? {
            if let Answer::Data(Data::Capability(list)) = answer {
                out.extend(list.as_ref().iter().map(|c| c.to_string()));
            }
        }
        Ok(out)
    }

    /// SASL XOAUTH2 with the initial response. `true`: refused.
    fn xoauth2(&mut self, user: &str, token: &str) -> Result<bool> {
        let sasl = format!("user={user}\x01auth=Bearer {token}\x01\x01").into_bytes();
        let tag = self.next_tag();
        let body = CommandBody::Authenticate { mechanism: AuthMechanism::XOAuth2, initial_response: Some(Hidden::new(Cow::Owned(sasl))) };
        self.client.enqueue_command(Command { tag, body });
        loop {
            match self.next(ANSWER)?.ok_or_else(lost)? {
                // A refusal comes first as a challenge holding the reason; the empty answer
                // asks for the final NO.
                Event::AuthenticateContinuationRequestReceived { .. } => {
                    let _ = self.client.set_authenticate_data(AuthenticateData::Continue(Hidden::new(Cow::Borrowed(b""))));
                }
                Event::AuthenticateStatusReceived { status, .. } => {
                    return Ok(!matches!(&status, Status::Tagged(done) if done.body.kind == StatusKind::Ok));
                }
                Event::StatusReceived { status: Status::Bye(_) } => return Err(lost()),
                _ => {}
            }
        }
    }

    pub(crate) fn folders(&mut self) -> Result<Vec<RemoteFolder>> {
        let mut out = Vec::new();
        for answer in self.run(CommandBody::list("", "*").expect("a plain LIST"))? {
            let Answer::Data(Data::List { items, delimiter, mailbox }) = answer else { continue };
            let marks: Vec<String> = items.iter().map(|i| i.to_string().to_ascii_lowercase()).collect();
            if marks.iter().any(|m| m == "\\noselect" || m == "\\nonexistent") {
                continue;
            }
            let path = mailbox_path(&mailbox);
            let name = match delimiter {
                Some(d) => path.rsplit(d.inner()).next().unwrap_or(&path).to_string(),
                None => path.clone(),
            };
            out.push(RemoteFolder { name: decode_utf7(&name), role: role(&path, &marks), path });
        }
        // Gmail marks All Mail \All and has no \Archive: archiving there is leaving INBOX.
        if !out.iter().any(|f| f.role == Role::Archive) {
            if let Some(all) = out.iter_mut().find(|f| f.role == Role::Other && f.path.ends_with("All Mail")) {
                all.role = Role::Archive;
            }
        }
        Ok(out)
    }

    fn select(&mut self, path: &str) -> Result<Selected> {
        if let Some(selected) = self.selected.as_ref().filter(|s| s.path == path) {
            return Ok(selected.clone());
        }
        self.reselect(path)
    }

    /// SELECT even when `path` is selected: its UIDNEXT and HIGHESTMODSEQ are read again.
    fn reselect(&mut self, path: &str) -> Result<Selected> {
        self.selected = None;
        let parameters = if self.has("CONDSTORE") { vec![SelectParameter::CondStore] } else { Vec::new() };
        let mut selected = Selected { path: path.into(), validity: 0, next: 0, modseq: 0 };
        for answer in self.run(CommandBody::Select { mailbox: mailbox(path)?, parameters })? {
            match answer {
                Answer::Code(Code::UidValidity(n)) => selected.validity = n.get(),
                Answer::Code(Code::UidNext(n)) => selected.next = n.get(),
                Answer::Code(Code::HighestModSeq(n)) => selected.modseq = n.get(),
                _ => {}
            }
        }
        if selected.validity == 0 {
            return Err(Error::Other("The mail server gave no UIDVALIDITY".into()));
        }
        self.selected = Some(selected.clone());
        Ok(selected)
    }

    pub(crate) fn changes(&mut self, folder: &str, cursor: Option<&str>) -> Result<Changes> {
        let now = self.reselect(folder)?;
        let before = cursor.and_then(Cursor::parse);
        let mut changes = Changes::default();
        let mut uids = Vec::new();
        match before.clone().filter(|c| c.validity == now.validity) {
            Some(before) => {
                if now.next == 0 || now.next > before.next {
                    let new = self.headers(&format!("{}:*", before.next))?;
                    changes.new = new.into_iter().filter(|m| m.uid.parse::<u32>().is_ok_and(|uid| uid >= before.next)).collect();
                }
                let known = before.uids;
                uids.clone_from(&known);
                if let Some(&low) = known.first() {
                    let range = format!("{low}:{}", before.next - 1);
                    changes.changed = match NonZeroU64::new(before.modseq).filter(|_| self.has("CONDSTORE")) {
                        Some(since) if now.modseq > before.modseq => self.flags(&range, Some(since))?,
                        Some(_) => Vec::new(),
                        // ponytail: without CONDSTORE every known message's flags come each
                        // time; the sync writes the ones that differ.
                        None => self.flags(&range, None)?,
                    };
                    let present: HashSet<u32> = self.search(SearchKey::Uid(sequence(&range)?))?.into_iter().collect();
                    uids.retain(|u| present.contains(u));
                    changes.removed = known.into_iter().filter(|u| !present.contains(u)).map(|u| u.to_string()).collect();
                }
            }
            None => {
                changes.reset = before.is_some();
                let since = SystemTime::now().checked_sub(FIRST_SYNC).unwrap_or(SystemTime::UNIX_EPOCH);
                let secs = since.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let day = chrono::DateTime::from_timestamp(secs as i64, 0).unwrap_or_default().date_naive();
                let uids = self.search(SearchKey::Since(day.try_into().map_err(|_| lost())?))?;
                for chunk in uids.chunks(CHUNK) {
                    changes.new.extend(self.headers(&uid_set(chunk))?);
                }
            }
        }
        uids.extend(changes.new.iter().filter_map(|m| m.uid.parse::<u32>().ok()));
        uids.sort_unstable();
        uids.dedup();
        let newest = uids.last().map_or(0, |u| u + 1);
        let next = now.next.max(newest).max(before.filter(|b| b.validity == now.validity).map_or(0, |b| b.next));
        changes.cursor = format!("{}:{next}:{}:{}", now.validity, now.modseq, uid_set(&uids));
        Ok(changes)
    }

    fn search(&mut self, key: SearchKey<'static>) -> Result<Vec<u32>> {
        let mut uids = Vec::new();
        for answer in self.run(CommandBody::search(None, Vec1::from(key), true))? {
            if let Answer::Data(Data::Search(found, _)) = answer {
                uids.extend(found.iter().map(|u| u.get()));
            }
        }
        uids.sort_unstable();
        Ok(uids)
    }

    fn fetch(&mut self, set: &str, items: Vec<MessageDataItemName<'static>>, modifiers: Vec<FetchModifier>) -> Result<Vec<Vec<MessageDataItem<'static>>>> {
        let body = CommandBody::Fetch {
            sequence_set: sequence(set)?,
            macro_or_item_names: MacroOrMessageDataItemNames::MessageDataItemNames(items),
            uid: true,
            modifiers,
        };
        let answers = self.run(body)?;
        Ok(answers
            .into_iter()
            .filter_map(|a| match a {
                Answer::Data(Data::Fetch { items, .. }) => Some(items.into_inner()),
                _ => None,
            })
            .collect())
    }

    fn headers(&mut self, set: &str) -> Result<Vec<RemoteMessage>> {
        let items = vec![MessageDataItemName::Uid, MessageDataItemName::Flags, MessageDataItemName::Rfc822Header];
        let mut messages: Vec<RemoteMessage> = self
            .fetch(set, items, Vec::new())?
            .into_iter()
            .filter_map(|items| {
                let mut message = RemoteMessage { uid: String::new(), flags: Flags::default(), header: Vec::new(), thread: None };
                for item in items {
                    match item {
                        MessageDataItem::Uid(uid) => message.uid = uid.to_string(),
                        MessageDataItem::Flags(flags) => message.flags = flags_of(&flags),
                        MessageDataItem::Rfc822Header(header) => message.header = header.into_option().unwrap_or_default().into_owned(),
                        _ => {}
                    }
                }
                (!message.uid.is_empty()).then_some(message)
            })
            .collect();
        if self.has("X-GM-EXT-1") && !messages.is_empty() {
            let threads = self.gmail_threads(set)?;
            for message in &mut messages {
                message.thread = threads.get(&message.uid).cloned();
            }
        }
        Ok(messages)
    }

    /// Gmail's conversation id (`X-GM-THRID`) of each uid in `set`. imap-codec has no Gmail
    /// extension, so the command goes out as bytes, and each answer comes back as a response
    /// the codec cannot read, whose bytes are read here.
    fn gmail_threads(&mut self, set: &str) -> Result<std::collections::HashMap<String, String>> {
        if set.is_empty() || !set.bytes().all(|b| b.is_ascii_digit() || b":,*".contains(&b)) {
            return Err(Error::Other(format!("not a uid set: {set}")));
        }
        self.stop_idle()?;
        let tag = self.next_tag();
        let command = format!("{} UID FETCH {set} (UID X-GM-THRID)\r\n", tag.inner());
        self.conn.write_all(command.as_bytes()).and_then(|()| self.conn.flush()).map_err(|_| lost())?;
        self.skipped = Some(Vec::new());
        let done = loop {
            match self.next(ANSWER) {
                Ok(Some(Event::StatusReceived { status: Status::Tagged(done) })) if done.tag == tag => break Ok(done.body.kind),
                Ok(Some(Event::StatusReceived { status: Status::Bye(_) })) | Ok(None) => break Err(lost()),
                Ok(Some(_)) => {}
                Err(e) => break Err(e),
            }
        };
        let skipped = self.skipped.take().unwrap_or_default();
        if done? != StatusKind::Ok {
            return Ok(Default::default());
        }
        Ok(skipped.iter().filter_map(|line| gmail_thread(line)).collect())
    }

    fn flags(&mut self, set: &str, since: Option<NonZeroU64>) -> Result<Vec<(String, Flags)>> {
        let modifiers = since.map(FetchModifier::ChangedSince).into_iter().collect();
        let items = vec![MessageDataItemName::Uid, MessageDataItemName::Flags];
        Ok(self
            .fetch(set, items, modifiers)?
            .into_iter()
            .filter_map(|items| {
                let uid = items.iter().find_map(|i| if let MessageDataItem::Uid(u) = i { Some(u.to_string()) } else { None })?;
                let flags = items.iter().find_map(|i| if let MessageDataItem::Flags(f) = i { Some(flags_of(f)) } else { None })?;
                Some((uid, flags))
            })
            .collect())
    }

    pub(crate) fn fetch_body(&mut self, folder: &str, uid: &str) -> Result<Vec<u8>> {
        self.select(folder)?;
        let peek = MessageDataItemName::BodyExt { section: None, partial: None, peek: true };
        for items in self.fetch(&number(uid)?.to_string(), vec![MessageDataItemName::Uid, peek], Vec::new())? {
            for item in items {
                if let MessageDataItem::BodyExt { section: None, data, .. } = item {
                    return Ok(data.into_option().unwrap_or_default().into_owned());
                }
            }
        }
        Err(Error::Other("The message is no longer on the server".into()))
    }

    pub(crate) fn apply(&mut self, folder: &str, uid: &str, op: &Op) -> Result<()> {
        self.select(folder)?;
        let set = number(uid)?.to_string();
        let store = |kind, flag| CommandBody::store(sequence(&set)?, kind, StoreResponse::Silent, vec![flag], true).map_err(|_| lost());
        let on = |yes: bool| if yes { StoreType::Add } else { StoreType::Remove };
        match op {
            Op::Seen(yes) => drop(self.run(store(on(*yes), Flag::Seen)?)?),
            Op::Flagged(yes) => drop(self.run(store(on(*yes), Flag::Flagged)?)?),
            Op::Move(to) if self.has("MOVE") => drop(self.run(CommandBody::Move { sequence_set: sequence(&set)?, mailbox: mailbox(to)?, uid: true })?),
            Op::Move(to) => {
                self.run(CommandBody::Copy { sequence_set: sequence(&set)?, mailbox: mailbox(to)?, uid: true })?;
                self.expunge(store(StoreType::Add, Flag::Deleted)?, &set)?;
            }
            Op::Delete => self.expunge(store(StoreType::Add, Flag::Deleted)?, &set)?,
        }
        Ok(())
    }

    fn expunge(&mut self, mark: CommandBody<'static>, set: &str) -> Result<()> {
        self.run(mark)?;
        // ponytail: without UIDPLUS, EXPUNGE also removes what another client marked \Deleted.
        let expunge = if self.has("UIDPLUS") { CommandBody::ExpungeUid { sequence_set: sequence(set)? } } else { CommandBody::Expunge };
        self.run(expunge)?;
        Ok(())
    }

    pub(crate) fn append(&mut self, folder: &str, raw: &[u8], flags: Flags) -> Result<()> {
        let mut list = Vec::new();
        list.extend(flags.seen.then_some(Flag::Seen));
        list.extend(flags.flagged.then_some(Flag::Flagged));
        list.extend(flags.answered.then_some(Flag::Answered));
        let body = CommandBody::append(folder.to_string(), list, None, raw.to_vec())
            .map_err(|_| Error::Other("The message holds bytes IMAP cannot store".into()))?;
        self.run(body)?;
        Ok(())
    }

    /// IDLE on `folder` until the server reports a change or `timeout` passes. The IDLE stays
    /// open after a timeout, so the sync thread can wait in short steps, between which it
    /// takes the user's actions; the next command ends it.
    pub(crate) fn wait(&mut self, folder: &str, timeout: Duration) -> Result<bool> {
        if !self.has("IDLE") {
            // ponytail: a server without IDLE is asked every `timeout`; INBOX is polled as other
            // folders are, only more often.
            std::thread::sleep(timeout);
            return Ok(true);
        }
        if self.idle.as_ref().is_some_and(|(_, since)| since.elapsed() > IDLE_RENEW || self.selected.as_ref().is_none_or(|s| s.path != folder)) {
            self.stop_idle()?;
        }
        if self.idle.is_none() {
            self.select(folder)?;
            let tag = self.next_tag();
            self.client.enqueue_command(Command { tag: tag.clone(), body: CommandBody::Idle });
            loop {
                match self.next(ANSWER)?.ok_or_else(lost)? {
                    Event::IdleAccepted { .. } => break,
                    Event::IdleRejected { status, .. } => return Err(Error::Other(format!("The mail server refused IDLE: {}", status_text(&status)))),
                    Event::DataReceived { .. } => return self.stop_idle().map(|()| true),
                    _ => {}
                }
            }
            self.idle = Some((tag, Instant::now()));
        }
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(false);
            }
            match self.next(left)? {
                None => return Ok(false),
                Some(Event::DataReceived { data: Data::Exists(_) | Data::Expunge(_) | Data::Fetch { .. } | Data::Vanished { .. } }) => return Ok(true),
                Some(Event::StatusReceived { status: Status::Bye(_) }) => return Err(lost()),
                Some(_) => {}
            }
        }
    }

    fn stop_idle(&mut self) -> Result<()> {
        let Some((tag, _)) = self.idle.take() else { return Ok(()) };
        let _ = self.client.set_idle_done();
        loop {
            match self.next(ANSWER)?.ok_or_else(lost)? {
                Event::StatusReceived { status: Status::Tagged(done) } if done.tag == tag => return Ok(()),
                Event::StatusReceived { status: Status::Bye(_) } => return Err(lost()),
                _ => {}
            }
        }
    }
}

enum Answer {
    Data(Data<'static>),
    Code(Code<'static>),
}

/// The uid and the thread id of `* n FETCH (UID u X-GM-THRID t)`, the items in any order.
fn gmail_thread(line: &[u8]) -> Option<(String, String)> {
    let line = std::str::from_utf8(line).ok()?.to_ascii_uppercase();
    let (_, items) = line.split_once(" FETCH (")?;
    let number = |name: &str| {
        let at = items.find(name)? + name.len();
        let digits: String = items[at..].chars().take_while(char::is_ascii_digit).collect();
        digits.parse::<u64>().ok().map(|n| n.to_string())
    };
    Some((number("UID ")?, number("X-GM-THRID ")?))
}

fn status_text(status: &Status) -> String {
    match status {
        Status::Untagged(body) => body.text.to_string(),
        Status::Tagged(done) => done.body.text.to_string(),
        Status::Bye(bye) => bye.text.to_string(),
    }
}

/// What a refused LOGIN means. Providers that turned passwords off say so in the text
/// (Gmail: "Application-specific password required", Microsoft: "BasicAuthBlocked").
fn sign_in_error(text: &str) -> Error {
    let text = text.to_ascii_lowercase();
    let off = ["application-specific password", "app password", "app-specific password", "web browser", "basicauthblocked", "basic auth", "weblogin"];
    if off.iter().any(|phrase| text.contains(phrase)) {
        Error::BasicAuthOff
    } else {
        Error::SignIn
    }
}

fn number(uid: &str) -> Result<u32> {
    uid.parse().map_err(|_| Error::Other(format!("{uid} is not an IMAP UID")))
}

fn sequence(set: &str) -> Result<SequenceSet> {
    SequenceSet::try_from(set).map_err(|_| Error::Other(format!("{set} is not a UID set")))
}

fn mailbox(path: &str) -> Result<Mailbox<'static>> {
    Mailbox::try_from(path.to_string()).map_err(|_| Error::Other(format!("{path} is not a folder name")))
}

fn mailbox_path(mailbox: &Mailbox) -> String {
    match mailbox {
        Mailbox::Inbox => "INBOX".into(),
        Mailbox::Other(other) => String::from_utf8_lossy(other.inner().as_ref()).into_owned(),
    }
}

/// Sorted UIDs as ranges: `1:3,7,9:10`.
fn uid_set(uids: &[u32]) -> String {
    let mut out = String::new();
    let mut at = 0;
    while at < uids.len() {
        let mut end = at;
        while end + 1 < uids.len() && uids[end + 1] == uids[end] + 1 {
            end += 1;
        }
        if !out.is_empty() {
            out.push(',');
        }
        out += &if end > at { format!("{}:{}", uids[at], uids[end]) } else { uids[at].to_string() };
        at = end + 1;
    }
    out
}

/// The inverse of [`uid_set`]; `None` for anything else.
fn parse_uid_set(set: &str) -> Option<Vec<u32>> {
    let mut out = Vec::new();
    for part in set.split(',').filter(|p| !p.is_empty()) {
        match part.split_once(':') {
            Some((a, b)) => out.extend(a.parse::<u32>().ok()?..=b.parse::<u32>().ok()?),
            None => out.push(part.parse().ok()?),
        }
    }
    Some(out)
}

fn flags_of(flags: &[FlagFetch]) -> Flags {
    let has = |flag: Flag| flags.iter().any(|f| matches!(f, FlagFetch::Flag(x) if *x == flag));
    Flags { seen: has(Flag::Seen), flagged: has(Flag::Flagged), answered: has(Flag::Answered) }
}

/// The folder's role from its RFC 6154 special-use marks (lower case).
fn role(path: &str, marks: &[String]) -> Role {
    if path.eq_ignore_ascii_case("INBOX") {
        return Role::Inbox;
    }
    for mark in marks {
        match mark.as_str() {
            "\\drafts" => return Role::Drafts,
            "\\sent" => return Role::Sent,
            "\\archive" => return Role::Archive,
            "\\junk" => return Role::Junk,
            "\\trash" => return Role::Trash,
            _ => {}
        }
    }
    Role::Other
}

/// IMAP's modified UTF-7 (RFC 3501 §5.1.3) to text: `Entw&APw-rfe` is "Entwürfe". What does
/// not decode stays as it was.
fn decode_utf7(name: &str) -> String {
    use base64::{alphabet::IMAP_MUTF7, engine::GeneralPurpose, engine::GeneralPurposeConfig, Engine};
    let engine = GeneralPurpose::new(&IMAP_MUTF7, GeneralPurposeConfig::new().with_encode_padding(false).with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent));
    let mut out = String::new();
    let mut rest = name;
    while let Some(at) = rest.find('&') {
        out += &rest[..at];
        let Some(end) = rest[at..].find('-') else { return name.into() };
        let encoded = &rest[at + 1..at + end];
        if encoded.is_empty() {
            out.push('&');
        } else {
            let Ok(bytes) = engine.decode(encoded) else { return name.into() };
            let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|p| u16::from_be_bytes(*p)).collect();
            let Ok(text) = String::from_utf16(&units) else { return name.into() };
            out += &text;
        }
        rest = &rest[at + end + 1..];
    }
    out + rest
}

/// Makes `socket` TLS, by STARTTLS first where the port wants it. The certificate and host name
/// are checked by the system trust store; there is no other verifier and no way past a failure.
pub(crate) fn secure<S: Conn>(mut socket: S, host: &str, tls: Tls) -> Result<StreamOwned<ClientConnection, S>> {
    if tls == Tls::StartTls {
        starttls(&mut socket)?;
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .and_then(|builder| builder.with_platform_verifier())
        .map_err(|_| Error::NoTls)?
        .with_no_client_auth();
    let name = ServerName::try_from(host.to_string()).map_err(|_| Error::Other("That is not a server name".into()))?;
    let connection = ClientConnection::new(Arc::new(config), name).map_err(|_| Error::NoTls)?;
    let mut stream = StreamOwned::new(connection, socket);
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock).map_err(|e| {
            log::debug!("imap: TLS failed: {e}");
            Error::NoTls
        })?;
    }
    Ok(stream)
}

/// The plain-text part of a STARTTLS port: the greeting, then STARTTLS, which must be accepted.
/// Read a byte at a time so nothing sent after the OK, before TLS, is ever read as the server's.
fn starttls(socket: &mut impl Conn) -> Result<()> {
    let greeting = line(socket)?;
    if !greeting.starts_with("* OK") {
        return Err(Error::NoTls);
    }
    socket.write_all(b"a0 STARTTLS\r\n").map_err(|_| lost())?;
    loop {
        let reply = line(socket)?;
        if let Some(status) = reply.strip_prefix("a0 ") {
            return if status.to_ascii_uppercase().starts_with("OK") { Ok(()) } else { Err(Error::NoTls) };
        }
    }
}

fn line(socket: &mut impl Conn) -> Result<String> {
    let mut out = Vec::new();
    let mut byte = [0];
    while !out.ends_with(b"\r\n") {
        if out.len() > 8192 || socket.read(&mut byte).map_err(|_| lost())? == 0 {
            return Err(lost());
        }
        out.push(byte[0]);
    }
    Ok(String::from_utf8_lossy(&out[..out.len() - 2]).into_owned())
}

#[cfg(test)]
mod tests;
