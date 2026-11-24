//! Reading untrusted mail (PLAN-mail.md §4, WP-B). Nothing here fetches, opens or resolves
//! anything a message names, and nothing is rendered: the HTML goes to Chromium as it is,
//! inside the sandboxed document baa-web builds (§6).

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use mail_parser::{AddressList, Message, MessageParser, MessagePart, PartId, PartKind};

use crate::{Address, Attachment, Body};

/// What the list and the threading need from a header block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Envelope {
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub subject: String,
    /// Unix seconds; 0 when the message gave none that parses.
    pub date: u64,
    pub message_id: Option<String>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    /// A `multipart/mixed` top level: the list's paper clip, before the body is here.
    pub attachments: bool,
    pub dmarc_failed: bool,
}

pub(crate) fn envelope(header: &[u8]) -> Envelope {
    let Some(message) = MessageParser::new().parse_headers(header) else {
        return Envelope::default();
    };
    let ids = |list: Option<mail_parser::TextList>| list.map(|l| l.iter().map(str::to_owned).collect()).unwrap_or_default();
    Envelope {
        from: addresses(message.from()).into_iter().next().unwrap_or_default(),
        to: addresses(message.to()),
        cc: addresses(message.cc()),
        reply_to: addresses(message.reply_to()),
        subject: message.subject().unwrap_or_default().to_owned(),
        date: message.date().map(|d| d.to_timestamp().max(0) as u64).unwrap_or(0),
        message_id: message.message_id().map(str::to_owned),
        in_reply_to: ids(message.in_reply_to()),
        references: ids(message.references()),
        attachments: message.is_content_type("multipart", "mixed"),
        dmarc_failed: dmarc_failed(&message),
    }
}

pub(crate) fn addresses(list: Option<AddressList>) -> Vec<Address> {
    let Some(list) = list else { return Vec::new() };
    list.mailboxes()
        .filter_map(|m| {
            let email = m.address()?.trim().to_owned();
            Some(Address { name: m.name().unwrap_or_default().trim().to_owned(), email })
        })
        .collect()
}

/// The unfolded value of the first header `name` in a header block.
pub(crate) fn header_text(header: &[u8], name: &str) -> Option<String> {
    let message = MessageParser::new().parse_headers(header)?;
    let value = message.headers().get(name)?.raw_value();
    Some(String::from_utf8_lossy(value).split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Whether the topmost `Authentication-Results` says DMARC failed. Only the topmost counts:
/// the user's own provider adds it last, on top, and a sender can write any number below it.
// ponytail: the authserv-id is not checked against the account's provider, which baa does not
// know. A provider that adds no such header leaves the sender's own on top; a wrong code is
// then the worst outcome, since the sender must still match the page (PLAN-mail.md §7).
fn dmarc_failed(message: &mail_parser::Message) -> bool {
    let Some(top) = message.headers().all("Authentication-Results").next() else { return false };
    let value = String::from_utf8_lossy(top.raw_value());
    without_comments(&value).split(';').skip(1).any(|result| {
        let result = result.trim().to_ascii_lowercase();
        let Some(verdict) = result.strip_prefix("dmarc") else { return false };
        verdict.trim_start().strip_prefix('=').is_some_and(|v| v.split_whitespace().next() == Some("fail"))
    })
}

/// RFC 8601 allows `(comments)` anywhere, nested; a comment reading "dmarc=pass" must not count.
fn without_comments(value: &str) -> String {
    let mut depth = 0usize;
    let mut out = String::with_capacity(value.len());
    let mut escaped = false;
    for c in value.chars() {
        match c {
            _ if escaped => escaped = false,
            '\\' if depth > 0 => escaped = true,
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// What a message may have inlined into its HTML, in base64 bytes; further `cid:` references
/// stay as written, since a hostile message can name one big image many times.
const MAX_INLINED: usize = 32 << 20;
const SNIPPET_CHARS: usize = 200;

pub(crate) fn body(raw: &[u8]) -> Body {
    let Some(message) = MessageParser::new().parse(raw) else {
        return Body { html: String::new(), text: String::new(), attachments: Vec::new(), has_remote_images: false };
    };
    let text = text_of(&message);
    let html_part = message.html_body().find(|p| matches!(p.kind(), PartKind::Html)).and_then(|p| p.text());
    let mut inlined = Vec::new();
    let (html, has_remote_images) = match html_part {
        Some(html) => (inline_cids(&html, &message, &mut inlined), has_remote_images(&html)),
        None => (format!("<pre style=\"white-space:pre-wrap\">{}</pre>", escape(&text)), false),
    };
    let attachments = message
        .attachments()
        .enumerate()
        .filter(|(_, p)| !inlined.contains(&p.id()))
        .map(|(part, p)| {
            let name = match p.attachment_name() {
                Some(name) => name.to_owned(),
                None if p.is_content_type("message", "rfc822") => "message.eml".to_owned(),
                None => "attachment".to_owned(),
            };
            let mime = mime_of(&p);
            Attachment { part, dangerous: dangerous(&name, &mime), name, mime, size: p.decoded_len() as u64 }
        })
        .collect();
    Body { html, text, attachments, has_remote_images }
}

pub(crate) fn snippet(body: &Body) -> String {
    let mut out = String::new();
    for word in body.text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
        if out.chars().nth(SNIPPET_CHARS).is_some() {
            break;
        }
    }
    out.chars().take(SNIPPET_CHARS).collect()
}

pub(crate) fn plain_text(raw: &[u8]) -> String {
    MessageParser::new().parse(raw).map(|m| text_of(&m)).unwrap_or_default()
}

/// Every attachment, as [`attachment`] gives one.
pub(crate) fn attachments(raw: &[u8]) -> Vec<(String, Vec<u8>)> {
    let Some(message) = MessageParser::new().parse(raw) else { return Vec::new() };
    message.attachments().map(|p| (p.attachment_name().unwrap_or("attachment").to_owned(), p.decoded().into_owned())).collect()
}

pub(crate) fn attachment(raw: &[u8], part: usize) -> Option<(String, Vec<u8>)> {
    let message = MessageParser::new().parse(raw)?;
    let p = message.attachments().nth(part)?;
    Some((p.attachment_name().unwrap_or("attachment").to_owned(), p.decoded().into_owned()))
}

pub(crate) fn dangerous(name: &str, mime: &str) -> bool {
    const EXTENSIONS: &[&str] = &[
        "app", "pkg", "mpkg", "dmg", "command", "sh", "bash", "zsh", "csh", "scpt", "scptd", "terminal", "workflow", "action", "bat",
        "cmd", "com", "exe", "msi", "scr", "pif", "lnk", "ps1", "vbs", "vbe", "wsf", "jar", "js", "jse", "hta", "reg", "dll",
    ];
    const MIMES: &[&str] = &[
        "application/x-apple-diskimage",
        "application/x-msdownload",
        "application/x-msdos-program",
        "application/x-dosexec",
        "application/vnd.microsoft.portable-executable",
        "application/x-msi",
        "application/x-executable",
        "application/x-mach-binary",
        "application/x-sh",
        "application/x-shellscript",
        "text/x-shellscript",
        "application/java-archive",
    ];
    let extension = name.trim_end_matches(|c: char| c == '.' || c.is_whitespace()).rsplit_once('.').map_or(String::new(), |(_, e)| e.to_ascii_lowercase());
    let mime = mime.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    EXTENSIONS.contains(&extension.as_str()) || MIMES.contains(&mime.as_str())
}

fn text_of(message: &Message) -> String {
    message.text_bodies().collect::<Vec<_>>().join("\n")
}

fn mime_of(part: &MessagePart) -> String {
    let Some(content_type) = part.content_type() else { return "application/octet-stream".to_owned() };
    let mime = format!("{}/{}", content_type.ctype(), content_type.subtype().unwrap_or("octet-stream")).to_ascii_lowercase();
    // The type goes into a data: URL inside an attribute, so nothing but token characters stays.
    if mime.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'+' | b'-' | b'.')) { mime } else { "application/octet-stream".to_owned() }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Replaces `cid:x` after a quote, `(` or `=` with a data: URL of the part whose Content-ID is
/// `<x>`; everything else is copied as it was. The parts used go into `inlined`.
fn inline_cids(html: &str, message: &Message, inlined: &mut Vec<PartId>) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let (mut copied, mut from, mut budget) = (0, 0, MAX_INLINED);
    while let Some(found) = lower[from..].find("cid:") {
        let start = from + found;
        let id_start = start + 4;
        from = id_start;
        if start == 0 || !matches!(html.as_bytes()[start - 1], b'"' | b'\'' | b'(' | b'=') {
            continue;
        }
        let id_end = html[id_start..].find(|c: char| matches!(c, '"' | '\'' | ')' | '>') || c.is_whitespace()).map_or(html.len(), |n| id_start + n);
        let id = &html[id_start..id_end];
        let Some(part) = message.parts().find(|p| !p.is_multipart() && p.content_id().is_some_and(|c| c.trim() == id)) else { continue };
        let data = part.decoded();
        let size = data.len().div_ceil(3) * 4;
        if size > budget {
            continue;
        }
        budget -= size;
        out.push_str(&html[copied..start]);
        out.push_str("data:");
        out.push_str(&mime_of(&part));
        out.push_str(";base64,");
        base64(&data, &mut out);
        copied = id_end;
        from = id_end;
        inlined.push(part.id());
    }
    out.push_str(&html[copied..]);
    out
}

fn base64(data: &[u8], out: &mut String) {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
}

// ponytail: a scan, not an HTML parse. It misses a remote address behind an entity
// (`&#104;ttp:`), a second `srcset` candidate and `@import`/`image-set()` in CSS, and it
// counts `data-src=` and a `src=` in text. It only decides whether to offer "load remote
// images"; what a page may load is the document's own policy (PLAN-mail.md §6).
fn has_remote_images(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    let remote = |rest: &str| {
        let rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '"' || c == '\'');
        ["http:", "https:", "//"].iter().any(|p| rest.starts_with(p))
    };
    let attribute = |name: &str| {
        lower.match_indices(name).any(|(i, _)| lower[i + name.len()..].trim_start().strip_prefix('=').is_some_and(remote))
    };
    ["src", "srcset", "background", "poster"].iter().any(|n| attribute(n)) || lower.match_indices("url(").any(|(i, _)| remote(&lower[i + 4..]))
}

/// The sender's name for a file, made safe to save under: its last component only, with no
/// separator, NUL or control character, no leading dot (no hidden file) and no `..`.
pub(crate) fn file_name(sender: &str) -> String {
    let last = sender.rsplit(['/', '\\']).next().unwrap_or_default();
    // A ':' is a '/' to Finder.
    let clean: String = last.chars().filter(|c| !c.is_control()).map(|c| if c == ':' { '_' } else { c }).collect();
    let clean = clean.trim_start_matches(|c: char| c == '.' || c.is_whitespace()).trim_end();
    let mut name = String::new();
    for c in clean.chars() {
        if name.len() + c.len_utf8() > 200 {
            break;
        }
        name.push(c);
    }
    if name.is_empty() {
        "attachment".into()
    } else {
        name
    }
}

/// Writes `bytes` into `dir` under `name` (from [`file_name`]), or "name (2).ext" and so on
/// when that is taken: an existing file is never overwritten. The file is quarantined as a
/// browser download is, so Gatekeeper checks it before it opens.
pub(crate) fn save(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    for n in 1..1000 {
        let path = dir.join(if n == 1 { name.to_owned() } else { format!("{stem} ({n}){extension}") });
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                quarantine(&path)?;
                file.write_all(bytes)?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "too many files of that name"))
}

#[cfg(target_os = "macos")]
fn quarantine(path: &Path) -> std::io::Result<()> {
    use std::{
        ffi::{c_char, c_int, c_void, CString},
        os::unix::ffi::OsStrExt,
    };
    extern "C" {
        fn setxattr(path: *const c_char, name: *const c_char, value: *const c_void, size: usize, position: u32, options: c_int) -> c_int;
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    // The flags Chrome writes for a download.
    let value = format!("0081;{secs:08x};baa;");
    let path = CString::new(path.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
    // SAFETY: both strings are NUL-terminated and outlive the call; `value` is read for `size` bytes.
    let done = unsafe { setxattr(path.as_ptr(), c"com.apple.quarantine".as_ptr(), value.as_ptr().cast(), value.len(), 0, 0) };
    if done == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn quarantine(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
pub(crate) fn corpus()-> Vec<(&'static str, Vec<u8>)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mail");
    let mut files: Vec<_> = std::fs::read_dir(&dir).expect("tests/mail").filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "eml")).collect();
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let name: &'static str = Box::leak(p.file_name().unwrap().to_string_lossy().into_owned().into_boxed_str());
            (name, std::fs::read(&p).unwrap())
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn mutate(bytes: &[u8], seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_add(1).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = move |below: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % below.max(1) as u64) as usize
    };
    let mut out = bytes.to_vec();
    for _ in 0..1 + next(4) {
        if out.is_empty() {
            break;
        }
        let at = next(out.len());
        match next(5) {
            0 => out[at] ^= 1 << next(8),
            1 => out.insert(at, next(256) as u8),
            2 => {
                out.remove(at);
            }
            3 => {
                let end = (at + 1 + next(64)).min(out.len());
                let slice = out[at..end].to_vec();
                out.splice(at..at, slice);
            }
            _ => out.truncate(at),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_envelope_has_who_what_when_and_the_thread_headers() {
        let header = b"From: \"Ola N.\" <ola@example.com>\r\nTo: dan@example.com, Kari <kari@example.org>\r\n\
            Subject: =?utf-8?q?R=C3=B8d?= sheet\r\nDate: Sun, 22 Nov 2026 10:00:00 +0000\r\n\
            Message-ID: <b@example.com>\r\nIn-Reply-To: <a@example.com>\r\nReferences: <root@example.com> <a@example.com>\r\n\
            Content-Type: multipart/mixed; boundary=x\r\n\r\n";
        let e = envelope(header);
        assert_eq!(e.from, Address { name: "Ola N.".into(), email: "ola@example.com".into() });
        assert_eq!(e.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["dan@example.com", "kari@example.org"]);
        assert_eq!((e.subject.as_str(), e.date), ("Rød sheet", 1_795_341_600));
        assert_eq!(e.message_id.as_deref(), Some("b@example.com"));
        assert_eq!((e.in_reply_to, e.references), (vec!["a@example.com".to_owned()], vec!["root@example.com".to_owned(), "a@example.com".to_owned()]));
        assert!(e.attachments && !e.dmarc_failed);
        assert_eq!(envelope(b"\xff\x00garbage"), Envelope { subject: String::new(), ..envelope(b"\xff\x00garbage") });
    }

    #[test]
    fn only_the_topmost_authentication_results_decides_dmarc() {
        let failed = b"Authentication-Results: mx.provider.example; spf=pass smtp.mailfrom=x.example;\r\n dmarc=fail (p=REJECT) header.from=bank.example\r\n\
            Authentication-Results: forged.example; dmarc=pass header.from=bank.example\r\nFrom: a@bank.example\r\n\r\n";
        assert!(envelope(failed).dmarc_failed);
        let forged_below = b"Authentication-Results: mx.provider.example; dmarc=pass header.from=bank.example\r\n\
            Authentication-Results: mx.provider.example; dmarc=fail header.from=bank.example\r\n\r\n";
        assert!(!envelope(forged_below).dmarc_failed, "a header below the provider's is the sender's");
        let in_comment = b"Authentication-Results: mx.provider.example; spf=pass (dmarc=fail; not really)\r\n\r\n";
        assert!(!envelope(in_comment).dmarc_failed);
        let id_only = b"Authentication-Results: dmarc=fail.example; spf=pass\r\n\r\n";
        assert!(!envelope(id_only).dmarc_failed, "the authserv-id is not a result");
        assert!(!envelope(b"From: a@b.example\r\n\r\n").dmarc_failed);
    }
}

#[cfg(test)]
mod body_tests {
    use super::*;

    fn file(name: &str) -> Vec<u8> {
        corpus().into_iter().find(|(n, _)| *n == name).unwrap().1
    }

    #[test]
    fn a_nested_message_gives_the_html_alternative_with_its_cid_image_inlined() {
        let body = body(&file("nested-multipart.eml"));
        assert!(body.html.contains("HTML version of the report."), "{}", body.html);
        assert!(body.html.contains("src=\"data:image/png;base64,iVBORw0KGgo"), "{}", body.html);
        assert!(!body.html.contains("cid:"));
        assert_eq!(body.text.trim(), "Plain version of the report.");
        let names: Vec<_> = body.attachments.iter().map(|a| (a.name.as_str(), a.mime.as_str(), a.size, a.dangerous)).collect();
        assert_eq!(names, [("report.pdf", "application/pdf", 28, false)], "the inlined logo is not an attachment");
        assert!(!body.has_remote_images);
    }

    #[test]
    fn the_part_of_an_attachment_gives_its_bytes_and_a_missing_part_gives_none() {
        let raw = file("nested-multipart.eml");
        let part = body(&raw).attachments[0].part;
        let (name, bytes) = attachment(&raw, part).unwrap();
        assert_eq!(name, "report.pdf");
        assert_eq!(bytes, b"%PDF-1.4\nbaa test pdf\n%%EOF\n");
        assert!(attachment(&raw, 99).is_none());
        assert!(attachment(b"", 0).is_none());
    }

    #[test]
    fn a_text_only_message_is_escaped_inside_a_pre() {
        let body = body(&file("plain-text.eml"));
        let (open, close) = ("<pre style=\"white-space:pre-wrap\">", "</pre>");
        assert!(body.html.starts_with(open) && body.html.ends_with(close), "{}", body.html);
        let inner = &body.html[open.len()..body.html.len() - close.len()];
        assert!(!inner.contains('<') && !inner.contains('>') && !inner.contains('"') && !inner.contains('\''), "{inner}");
        assert!(inner.contains("&lt;b&gt;bold&lt;/b&gt; &amp; &quot;quotes&quot; and &#39;ticks&#39;, then &lt;/pre&gt;&lt;script&gt;"), "{inner}");
        assert!(body.text.contains("<b>bold</b>"));
        assert!(!body.has_remote_images && body.attachments.is_empty());
    }

    #[test]
    fn an_html_only_message_keeps_its_html_untouched_and_reports_remote_images() {
        let raw = file("html-only.eml");
        let body = body(&raw);
        assert!(body.html.contains("<script>alert(1)</script>"));
        assert!(body.html.contains("onerror=\"alert(2)\"") && body.html.contains("<form action=\"https://evil.example/\">"));
        assert!(body.has_remote_images);
        assert_eq!(body.text.trim(), "Hello");
        assert_eq!(plain_text(&raw).trim(), "Hello");
    }

    #[test]
    fn a_declared_charset_that_does_not_exist_still_gives_text() {
        let body = body(&file("bad-charset.eml"));
        assert!(body.text.contains("still readable."), "{:?}", body.text);
        assert!(body.html.contains("still readable."));
    }

    #[test]
    fn a_subject_folded_in_the_middle_of_a_word_reads_whole() {
        let subject = envelope(&file("folded-mid-word.eml")).subject;
        assert!(subject.starts_with("Quarterly rep"), "{subject:?}");
        assert!(subject.contains("ort for the whole") && subject.ends_with("Rød sheet"), "{subject:?}");
    }

    #[test]
    fn a_forged_authentication_results_below_the_providers_does_not_hide_a_dmarc_fail() {
        assert!(envelope(&file("forged-auth-results.eml")).dmarc_failed);
    }

    #[test]
    fn a_truncated_multipart_gives_the_whole_part_before_the_cut() {
        let body = body(&file("truncated.eml"));
        assert!(body.text.contains("The first part is whole."));
        assert!(body.attachments.len() <= 1);
    }

    #[test]
    fn hostile_file_names_come_out_as_the_sender_wrote_them() {
        let body = body(&file("hostile-filename.eml"));
        let names: Vec<_> = body.attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["../../.ssh/authorized_keys", "..\\..\\evil.bat", "nul\0.txt", ".hidden", "invoice.pdf.app"]);
        let dangerous: Vec<_> = body.attachments.iter().map(|a| a.dangerous).collect();
        assert_eq!(dangerous, [false, true, false, false, true]);
    }

    #[test]
    fn dangerous_types_are_apps_installers_scripts_and_disk_images() {
        for name in ["a.bat", "a.app", "invoice.pdf.app", "a.command", "a.pkg", "a.dmg", "a.sh", "a.exe", "a.jar", "a.scpt", "a.terminal", "a.workflow", "A.EXE", "a.sh."] {
            assert!(dangerous(name, "application/octet-stream"), "{name}");
        }
        assert!(dangerous("noname", "application/x-apple-diskimage"));
        assert!(dangerous("noname", "Application/X-Apple-DiskImage; x=y"));
        for name in ["a.pdf", "a.png", "a.txt", ".hidden", "app", "exe"] {
            assert!(!dangerous(name, "application/pdf"), "{name}");
        }
    }

    #[test]
    fn an_attachment_of_ten_megabytes_parses_and_reports_its_size() {
        let data = vec![0xA5u8; 10 << 20];
        let mut encoded = String::new();
        base64(&data, &mut encoded);
        let mut raw = String::from(
            "From: a@example.com\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nhi\r\n--b\r\n\
             Content-Type: application/zip\r\nContent-Transfer-Encoding: base64\r\nContent-Disposition: attachment; filename=\"big.zip\"\r\n\r\n",
        );
        for line in encoded.as_bytes().chunks(76) {
            raw.push_str(std::str::from_utf8(line).unwrap());
            raw.push_str("\r\n");
        }
        raw.push_str("--b--\r\n");
        let body = body(raw.as_bytes());
        assert_eq!((body.attachments.len(), body.attachments[0].size), (1, 10 << 20));
        assert_eq!(attachment(raw.as_bytes(), 0).unwrap().1, data);
    }

    #[test]
    fn base64_matches_the_standard_for_every_remainder() {
        for (input, want) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            let mut out = String::new();
            base64(input.as_bytes(), &mut out);
            assert_eq!(out, want);
        }
    }

    #[test]
    fn only_cid_references_are_rewritten_whatever_their_case_and_quotes() {
        let raw = b"Content-Type: multipart/related; boundary=r\r\n\r\n--r\r\nContent-Type: text/html\r\n\r\n\
            <img src='CID:a@x'><img src=\"cid:a@x\"><img src=\"cid:missing\"><a href=\"http://x.example/cid:a@x\">cid:a@x</a><div style=\"background:url(cid:a@x)\">\r\n\
            --r\r\nContent-Type: image/gif\r\nContent-ID: <a@x>\r\nContent-Transfer-Encoding: base64\r\n\r\nR0lGODlhAQABAAAAACw=\r\n--r--\r\n";
        let html = body(raw).html;
        assert_eq!(html.matches("data:image/gif;base64,R0lGODlhAQABAAAAACw=").count(), 3, "{html}");
        assert!(html.contains("src=\"cid:missing\"") && html.contains("href=\"http://x.example/cid:a@x\">cid:a@x</a>"), "{html}");
    }

    #[test]
    fn remote_images_are_found_in_attributes_and_css() {
        for html in [
            "<img src=\"https://x.example/a.png\">",
            "<IMG SRC = 'http://x.example/a.png'>",
            "<img src=//x.example/a.png>",
            "<td background=\"https://x.example/a.png\">",
            "<video poster=\"https://x.example/a.png\">",
            "<img srcset=\"https://x.example/a.png 2x\">",
            "<div style=\"background: URL( 'https://x.example/a.png')\">",
        ] {
            assert!(has_remote_images(html), "{html}");
        }
        for html in ["<img src=\"data:image/png;base64,AAAA\">", "<img src=\"cid:a\">", "<a href=\"https://x.example/\">x</a>"] {
            assert!(!has_remote_images(html), "{html}");
        }
    }

    #[test]
    fn a_snippet_collapses_whitespace_and_is_cut_on_a_char_boundary() {
        let mut b = body(&file("plain-text.eml"));
        b.text = "  Hello \r\n\t world \n\n  again ".to_owned();
        assert_eq!(snippet(&b), "Hello world again");
        b.text = "å ".repeat(300);
        let s = snippet(&b);
        assert_eq!(s.chars().count(), 200);
        assert!(s.starts_with("å å"));
        b.text = "x".repeat(1000);
        assert_eq!(snippet(&b).len(), 200);
        b.text.clear();
        assert_eq!(snippet(&b), "");
    }

    #[test]
    fn nothing_panics_on_a_mutated_corpus() {
        for (_, raw) in corpus() {
            for seed in 0..300 {
                let raw = mutate(&raw, seed);
                let b = body(&raw);
                let _ = (snippet(&b), plain_text(&raw), envelope(&raw));
                for part in 0..3 {
                    let _ = attachment(&raw, part);
                }
            }
        }
    }

    #[test]
    fn the_mutation_changes_bytes_and_repeats_for_a_seed() {
        let raw = file("nested-multipart.eml");
        assert_eq!(mutate(&raw, 7), mutate(&raw, 7));
        assert!((0..20).any(|seed| mutate(&raw, seed) != raw));
        assert!(mutate(b"", 1).is_empty());
    }
}
