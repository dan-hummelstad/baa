//! Outgoing mail (PLAN-mail.md §11 WP-B): a draft to RFC 822, and the drafts of a reply or a
//! forward.

// ponytail: the bytes are for WP-D's send, which lands after this. Remove once it calls outgoing.
#![allow(dead_code)]

use crate::{store::Store, AccountId, Address, Draft, Error, MessageId, Result};
use mail_builder::{MessageBuilder, headers::address::Address as BuilderAddress};
use std::collections::HashSet;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The message a reply or forward is made from, as the store has it.
pub(crate) struct Original {
    pub from: Address,
    pub reply_to: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub subject: String,
    /// Unix seconds.
    pub date: u64,
    /// The readable text, for the quote. Empty when the body is not here yet.
    pub text: String,
    /// As `Mail::body` gives it. Untrusted: it goes into the editor document, whose policy
    /// keeps it inert (PLAN-mail.md §6).
    pub html: String,
}

/// The thread headers of the message a draft answers.
pub(crate) struct Answered {
    pub message_id: String,
    pub references: Vec<String>,
}

fn builder_address(a: &Address) -> BuilderAddress<'static> {
    let name = (!a.name.is_empty()).then(|| a.name.clone());
    BuilderAddress::new_address(name, a.email.clone())
}

fn builder_list(list: &[Address]) -> BuilderAddress<'static> {
    BuilderAddress::new_list(list.iter().map(builder_address).collect())
}

/// Ids go between angle brackets unencoded, so one that could end the bracket or the line is
/// left out.
fn header_safe(id: &str) -> bool {
    !id.is_empty() && !id.chars().any(|c| c.is_control() || c.is_whitespace() || c == '<' || c == '>')
}

fn new_message_id(domain: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let state = std::collections::hash_map::RandomState::new();
    let part = |salt: u64| {
        let mut h = state.build_hasher();
        h.write_u128(nanos);
        h.write_u64(count);
        h.write_u64(salt);
        h.finish()
    };
    format!("{:016x}.{:016x}@{domain}", part(1), part(2))
}

fn sender_domain(from: &Address) -> &str {
    from.email
        .rsplit_once('@')
        .map(|(_, d)| d)
        .filter(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
        .unwrap_or("localhost")
}

/// What the outbox takes for `draft`: its RFC 822 bytes, the sender's address, and every
/// recipient (Bcc too, which the bytes do not name).
pub(crate) fn outgoing(store: &Store, draft: &Draft, now: SystemTime) -> Result<(Vec<u8>, String, Vec<String>)> {
    let account = store.account(draft.from)?.ok_or_else(|| Error::Other("The account of this draft is gone".into()))?;
    let answered = match draft.in_reply_to {
        Some(id) => store
            .envelope(id)?
            .and_then(|(_, e)| Some(Answered { message_id: e.message_id?, references: e.references })),
        None => None,
    };
    let from = Address { name: account.name, email: account.address };
    let raw = rfc822(draft, &from, answered.as_ref(), now)?;
    let to = draft.to.iter().chain(&draft.cc).chain(&draft.bcc).map(|a| a.email.clone()).collect();
    Ok((raw, from.email, to))
}

/// Multipart/alternative of the draft's text and HTML (one part when the other is empty), its
/// files attached under their last path component, a fresh Message-ID on the sender's domain,
/// and the thread headers of `answered`. Bcc is never written into the bytes.
// ponytail: every attachment is application/octet-stream; the ceiling is a type table, which
// a recipient's client mostly does not need since it goes by the file name.
pub(crate) fn rfc822(
    draft: &Draft,
    from: &Address,
    answered: Option<&Answered>,
    now: SystemTime,
) -> Result<Vec<u8>> {
    let secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut msg = MessageBuilder::new()
        .message_id(new_message_id(sender_domain(from)))
        .date(secs)
        .from(builder_address(from))
        .subject(draft.subject.clone());
    if !draft.to.is_empty() {
        msg = msg.to(builder_list(&draft.to));
    }
    if !draft.cc.is_empty() {
        msg = msg.cc(builder_list(&draft.cc));
    }
    if let Some(a) = answered.filter(|a| header_safe(&a.message_id)) {
        let refs: Vec<&str> = a
            .references
            .iter()
            .map(String::as_str)
            .filter(|r| header_safe(r))
            .chain([a.message_id.as_str()])
            .collect();
        msg = msg.in_reply_to(a.message_id.as_str()).references(refs.as_slice());
    }
    if !draft.text.is_empty() || draft.html.is_empty() {
        msg = msg.text_body(draft.text.clone());
    }
    if !draft.html.is_empty() {
        msg = msg.html_body(draft.html.clone());
    }
    for path in &draft.attachments {
        let name = path
            .file_name()
            .map_or_else(|| "attachment".into(), |n| n.to_string_lossy().into_owned());
        let bytes = std::fs::read(path)
            .map_err(|e| Error::Other(format!("The attachment {name} cannot be read: {e}")))?;
        msg = msg.attachment("application/octet-stream", name, bytes);
    }
    msg.write_to_vec()
        .map_err(|e| Error::Other(format!("The message cannot be written: {e}")))
}

fn display(a: &Address) -> String {
    if a.name.is_empty() {
        a.email.clone()
    } else {
        format!("{} <{}>", a.name, a.email)
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn date_text(secs: u64) -> String {
    mail_parser::DateTime::from_timestamp(secs as i64).to_rfc822()
}

fn with_prefix(subject: &str, own: &str, known: &[&str]) -> String {
    let lower = subject.trim_start().to_lowercase();
    if known.iter().any(|k| lower.starts_with(k)) {
        subject.to_string()
    } else {
        format!("{own} {subject}")
    }
}

fn empty_draft(account: AccountId, subject: String) -> Draft {
    Draft {
        id: None,
        from: account,
        to: Vec::new(),
        cc: Vec::new(),
        bcc: Vec::new(),
        subject,
        html: String::new(),
        text: String::new(),
        attachments: Vec::new(),
        in_reply_to: None,
    }
}

/// A reply from `account`, whose address is `me`, to message `id`: to its Reply-To, else its
/// From; with `all`, also every To and Cc but `me`, each once.
pub(crate) fn reply(
    original: &Original,
    id: MessageId,
    account: AccountId,
    me: &str,
    all: bool,
) -> Draft {
    let subject = with_prefix(&original.subject, "Re:", &["re:", "sv:", "aw:"]);
    let mut draft = empty_draft(account, subject);
    draft.to = if original.reply_to.is_empty() {
        vec![original.from.clone()]
    } else {
        original.reply_to.clone()
    };
    if all {
        let mut seen: HashSet<String> = draft.to.iter().map(|a| a.email.to_lowercase()).collect();
        let mut fresh = |list: &[Address]| -> Vec<Address> {
            list.iter()
                .filter(|a| !a.email.eq_ignore_ascii_case(me) && seen.insert(a.email.to_lowercase()))
                .cloned()
                .collect()
        };
        draft.to.extend(fresh(&original.to));
        draft.cc = fresh(&original.cc);
    }
    let who = display_name(&original.from);
    let date = date_text(original.date);
    let quoted: String = original.text.lines().map(|l| format!("> {l}\n")).collect();
    draft.text = format!("\n\nOn {date}, {who} wrote:\n{quoted}");
    draft.html = format!(
        "<br><br><div>{}</div><blockquote type=\"cite\">{}</blockquote>",
        escape_html(&format!("On {date}, {who} wrote:")),
        original.html
    );
    draft.in_reply_to = Some(id);
    draft
}

fn display_name(a: &Address) -> &str {
    if a.name.is_empty() { &a.email } else { &a.name }
}

// ponytail: the original's attachments are not carried; the ceiling is copying them to temp
// files for Draft::attachments.
pub(crate) fn forward(original: &Original, account: AccountId) -> Draft {
    let subject = with_prefix(&original.subject, "Fwd:", &["fwd:", "fw:", "wg:"]);
    let mut draft = empty_draft(account, subject);
    let from = display(&original.from);
    let date = date_text(original.date);
    let to = original.to.iter().map(display).collect::<Vec<_>>().join(", ");
    draft.text = format!(
        "\n\n---------- Forwarded message ----------\nFrom: {from}\nDate: {date}\nSubject: {}\nTo: {to}\n\n{}",
        original.subject, original.text
    );
    draft.html = format!(
        "<br><br><div>---------- Forwarded message ----------<br>From: {}<br>Date: {}<br>Subject: {}<br>To: {}</div><br>{}",
        escape_html(&from),
        escape_html(&date),
        escape_html(&original.subject),
        escape_html(&to),
        original.html
    );
    draft
}

/// What the user typed in To, Cc or Bcc: `Name <a@b>`, `"Last, First" <a@b>` or `a@b`, split
/// at commas and semicolons. An entry with no `@` is dropped.
pub fn parse_addresses(input: &str) -> Vec<Address> {
    let mut entries = Vec::new();
    let (mut start, mut quoted, mut angle, mut escaped) = (0, false, false, false);
    for (i, c) in input.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            ',' | ';' if !quoted && !angle => {
                entries.push(&input[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    entries.push(&input[start..]);
    entries.into_iter().filter_map(parse_one).collect()
}

fn parse_one(entry: &str) -> Option<Address> {
    let entry = entry.trim();
    let (name, email) = match entry.rfind('<') {
        Some(lt) => {
            let gt = lt + entry[lt..].find('>')?;
            let name = entry[..lt].trim();
            let name = name.strip_prefix('"').unwrap_or(name);
            let name = name.strip_suffix('"').unwrap_or(name).replace("\\\"", "\"");
            (name, entry[lt + 1..gt].trim())
        }
        None => (String::new(), entry),
    };
    let bad = |c: char| c.is_control() || c.is_whitespace() || "<>\",;".contains(c);
    (email.contains('@') && !email.chars().any(bad)).then(|| Address {
        name,
        email: email.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_parser::MessageParser;
    use std::path::PathBuf;
    use std::time::Duration;

    fn addr(name: &str, email: &str) -> Address {
        Address {
            name: name.into(),
            email: email.into(),
        }
    }

    fn draft() -> Draft {
        Draft {
            to: vec![addr("Bo Ek", "bo@there.org")],
            cc: vec![addr("", "cy@there.org")],
            bcc: vec![addr("", "secret@hidden.example")],
            subject: "Hello".into(),
            text: "plain".into(),
            html: "<p>rich</p>".into(),
            ..empty_draft(AccountId(1), String::new())
        }
    }

    fn original() -> Original {
        Original {
            from: addr("Ann", "ann@here.org"),
            reply_to: Vec::new(),
            to: vec![addr("", "Me@Here.org"), addr("", "bo@there.org")],
            cc: vec![addr("", "BO@there.org"), addr("", "cy@there.org")],
            subject: "Re: Re: x".into(),
            date: 1_700_000_000,
            text: "line one\nline two".into(),
            html: "<p>hi</p>".into(),
        }
    }

    fn now() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    }

    #[test]
    fn rfc822_parses_back_with_every_part_and_no_bcc() {
        let path: PathBuf = std::env::temp_dir().join(format!("baa-compose-{}.pdf", std::process::id()));
        std::fs::write(&path, b"%PDF").unwrap();
        let mut d = draft();
        d.attachments = vec![path.clone()];
        let bytes = rfc822(&d, &addr("Me", "me@here.org"), None, now()).unwrap();
        std::fs::remove_file(&path).unwrap();

        let text = String::from_utf8_lossy(&bytes).to_lowercase();
        assert!(!text.contains("bcc") && !text.contains("secret@hidden"));
        let m = MessageParser::default().parse(&bytes).unwrap();
        assert_eq!(m.subject(), Some("Hello"));
        assert_eq!(m.from().unwrap().first().unwrap().address(), Some("me@here.org"));
        assert_eq!(m.to().unwrap().first().unwrap().address(), Some("bo@there.org"));
        assert_eq!(m.cc().unwrap().first().unwrap().address(), Some("cy@there.org"));
        assert!(m.message_id().unwrap().ends_with("@here.org"));
        assert!(m.date().is_some());
        assert_eq!(m.body_text(0).unwrap().trim(), "plain");
        assert_eq!(m.body_html(0).unwrap().trim(), "<p>rich</p>");
        assert_eq!(m.attachments().len(), 1);
        let name = format!("baa-compose-{}.pdf", std::process::id());
        assert_eq!(m.attachments().next().unwrap().attachment_name(), Some(name.as_str()));
    }

    #[test]
    fn rfc822_names_an_attachment_by_its_last_path_component() {
        let dir = std::env::temp_dir().join(format!("baa-compose-dir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("report.pdf");
        std::fs::write(&path, b"x").unwrap();
        let mut d = draft();
        d.attachments = vec![path];
        let bytes = rfc822(&d, &addr("", "me@here.org"), None, now()).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let m = MessageParser::default().parse(&bytes).unwrap();
        assert_eq!(m.attachments().next().unwrap().attachment_name(), Some("report.pdf"));
        assert!(!String::from_utf8_lossy(&bytes).contains(dir.to_str().unwrap()));
    }

    #[test]
    fn rfc822_without_html_has_one_text_part() {
        let mut d = draft();
        d.html.clear();
        let bytes = rfc822(&d, &addr("", "me@here.org"), None, now()).unwrap();
        let text = String::from_utf8_lossy(&bytes).to_lowercase();
        assert!(!text.contains("multipart") && !text.contains("text/html"));
    }

    #[test]
    fn rfc822_of_an_unreadable_attachment_is_an_error() {
        let mut d = draft();
        d.attachments = vec![PathBuf::from("/nonexistent/baa/report.pdf")];
        assert!(rfc822(&d, &addr("", "me@here.org"), None, now()).is_err());
    }

    #[test]
    fn rfc822_threads_on_the_answered_message() {
        let answered = Answered {
            message_id: "c@x".into(),
            references: vec!["a@x".into(), "b@x".into()],
        };
        let bytes = rfc822(&draft(), &addr("", "me@here.org"), Some(&answered), now()).unwrap();
        let m = MessageParser::default().parse(&bytes).unwrap();
        assert_eq!(m.in_reply_to().unwrap().iter().collect::<Vec<_>>(), ["c@x"]);
        let refs: Vec<&str> = m.references().unwrap().iter().collect();
        assert_eq!(refs, ["a@x", "b@x", "c@x"]);
    }

    #[test]
    fn rfc822_message_ids_differ() {
        let a = rfc822(&draft(), &addr("", "me@here.org"), None, now()).unwrap();
        let b = rfc822(&draft(), &addr("", "me@here.org"), None, now()).unwrap();
        let id = |bytes: &[u8]| MessageParser::default().parse(bytes).unwrap().message_id().unwrap().to_string();
        assert_ne!(id(&a), id(&b));
    }

    #[test]
    fn rfc822_line_breaks_in_subject_name_and_address_start_no_header() {
        let mut d = draft();
        d.subject = "hi\r\nBcc: evil@x".into();
        d.to = vec![addr("Bo\r\nBcc: evil@x", "bo@there.org"), addr("", "a@b\r\nBcc: evil@x")];
        let bytes = rfc822(&d, &addr("Me\nBcc: evil@x", "me@here.org"), None, now()).unwrap();
        let m = MessageParser::default().parse(&bytes).unwrap();
        assert!(m.bcc().is_none());
        assert!(m.headers().get("Bcc").is_none());
        assert!(!String::from_utf8_lossy(&bytes).lines().any(|l| l.to_lowercase().starts_with("bcc")));
    }

    #[test]
    fn reply_goes_to_reply_to_when_present() {
        let mut o = original();
        o.reply_to = vec![addr("", "list@here.org")];
        let d = reply(&o, MessageId(7), AccountId(1), "me@here.org", false);
        assert_eq!(d.to, vec![addr("", "list@here.org")]);
        assert!(d.cc.is_empty());
        assert_eq!(d.in_reply_to, Some(MessageId(7)));
        assert_eq!(d.id, None);
    }

    #[test]
    fn reply_all_drops_me_and_duplicates() {
        let d = reply(&original(), MessageId(7), AccountId(1), "me@here.org", true);
        let emails = |l: &[Address]| l.iter().map(|a| a.email.clone()).collect::<Vec<_>>();
        assert_eq!(emails(&d.to), ["ann@here.org", "bo@there.org"]);
        assert_eq!(emails(&d.cc), ["cy@there.org"]);
    }

    #[test]
    fn reply_keeps_one_prefix_and_quotes_each_line() {
        let d = reply(&original(), MessageId(7), AccountId(1), "me@here.org", false);
        assert_eq!(d.subject, "Re: Re: x");
        assert_eq!(d.to, vec![addr("Ann", "ann@here.org")]);
        assert!(d.text.starts_with("\n\nOn "));
        assert!(d.text.contains("Ann wrote:\n> line one\n> line two\n"));
        assert!(d.html.contains("<blockquote type=\"cite\"><p>hi</p></blockquote>"));
        let plain = Original { subject: "x".into(), ..original() };
        assert_eq!(reply(&plain, MessageId(1), AccountId(1), "", false).subject, "Re: x");
        let aw = Original { subject: "AW: x".into(), ..original() };
        assert_eq!(reply(&aw, MessageId(1), AccountId(1), "", false).subject, "AW: x");
    }

    #[test]
    fn reply_escapes_the_attribution_in_html() {
        let mut o = original();
        o.from = addr("<b>Ann</b>", "ann@here.org");
        let d = reply(&o, MessageId(1), AccountId(1), "", false);
        assert!(d.html.contains("&lt;b&gt;Ann&lt;/b&gt; wrote:"));
        assert!(!d.html.contains("<b>Ann"));
    }

    #[test]
    fn forward_has_no_recipients_one_prefix_and_the_forwarded_block() {
        let o = Original { subject: "Fwd: x".into(), ..original() };
        let d = forward(&o, AccountId(1));
        assert!(d.to.is_empty() && d.cc.is_empty() && d.in_reply_to.is_none());
        assert_eq!(d.subject, "Fwd: x");
        assert!(d.text.contains("---------- Forwarded message ----------\nFrom: Ann <ann@here.org>\n"));
        assert!(d.text.contains("Subject: Fwd: x\nTo: Me@Here.org, bo@there.org\n\nline one"));
        assert!(d.html.contains("From: Ann &lt;ann@here.org&gt;<br>"));
        assert!(d.html.ends_with("<p>hi</p>"));
        let plain = Original { subject: "x".into(), ..original() };
        assert_eq!(forward(&plain, AccountId(1)).subject, "Fwd: x");
    }

    #[test]
    fn parse_addresses_reads_each_form() {
        let got = parse_addresses(r#"a@b.c, Ann <ann@x.org>; "Last, First" <lf@x.org> ,nonsense, "Q \"x\"" <q@x.org>"#);
        assert_eq!(
            got,
            [
                addr("", "a@b.c"),
                addr("Ann", "ann@x.org"),
                addr("Last, First", "lf@x.org"),
                addr("Q \"x\"", "q@x.org"),
            ]
        );
    }

    #[test]
    fn parse_addresses_survives_garbage() {
        for s in ["\"\",,;<>@", "\"unbalanced <a@b>", "<a@b", "a@b>", "<<<>>>", "\\", "\"\\"] {
            let _ = parse_addresses(s);
        }
        assert!(parse_addresses("\"\",,;<>@").is_empty());
        let long = "x".repeat(10_000);
        assert!(parse_addresses(&long).is_empty());
        let _ = parse_addresses(&format!("{long}@{long}, <{long}"));
        let _ = parse_addresses("é,ü <ö@å>;\u{0}");
    }
}
