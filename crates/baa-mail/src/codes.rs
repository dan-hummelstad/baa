//! Verification codes in just-arrived mail (PLAN-mail.md §7, steps 4 and 5). The input is a
//! message's text, never rendered; nothing here logs or stores a code, a sender or an address.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::Code;

#[derive(Deserialize)]
struct Patterns {
    phrases: Vec<String>,
    /// A sender's site, and the other sites whose codes it sends.
    aliases: HashMap<String, Vec<String>>,
}

impl Code {
    /// Whether this code may be offered on a page whose site (registrable domain) is
    /// `page_site`. `site_of` is the engine's registrable-domain function
    /// (`WebHost::site_of`), so a site is Chrome's notion and baa keeps no list of its own.
    pub fn offered_on(&self, page_site: &str, site_of: &dyn Fn(&str) -> String) -> bool {
        if self.dmarc_failed || page_site.is_empty() {
            return false;
        }
        if let Some(host) = &self.bound_host {
            return site_of(host).eq_ignore_ascii_case(page_site);
        }
        let Some((_, domain)) = self.sender.rsplit_once('@') else { return false };
        let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
        if !valid_host(&domain) {
            return false;
        }
        let sender_site = site_of(&domain);
        sender_site.eq_ignore_ascii_case(page_site)
            || patterns().aliases.get(&sender_site).is_some_and(|sites| sites.iter().any(|s| s.eq_ignore_ascii_case(page_site)))
    }
}

fn patterns() -> &'static Patterns {
    static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| {
        serde_json::from_str(include_str!("codes.json")).expect("codes.json is valid")
    });
    &PATTERNS
}

const TEXT_CHARS: usize = 64 * 1024;
const SUBJECT_CHARS: usize = 1000;
const AFTER: usize = 80;
const BEFORE: usize = 40;
const SIGNS: [char; 4] = ['$', '€', '£', '¥'];
const CURRENCY_CODES: [&str; 7] = ["kr", "nok", "usd", "eur", "sek", "dkk", "gbp"];

/// The code a message carries, and the host an origin-bound code is for. In this order: the
/// origin-bound last line, then the OTP-Token header, then a phrase from codes.json.
pub(crate) fn extract(
    subject: &str,
    text: &str,
    otp_token: Option<&str>,
) -> Option<(String, Option<String>)> {
    if let Some(found) = origin_bound(text) {
        return Some(found);
    }
    if let Some(code) = otp_token.and_then(header_code) {
        return Some((code, None));
    }
    phrase_code(subject, text).map(|code| (code, None))
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && !host.contains("..")
}

fn valid_code(code: &str) -> bool {
    (1..=32).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn origin_bound(text: &str) -> Option<(String, Option<String>)> {
    let line = text.lines().rev().find(|l| !l.trim().is_empty())?.trim();
    // A valid line is two hosts and a code at most; this keeps a huge last line from being split.
    if line.len() > 600 || !line.starts_with('@') {
        return None;
    }
    let mut parts = line.split_whitespace();
    let host = parts.next()?.strip_prefix('@')?;
    let code = parts.next()?.strip_prefix('#')?;
    if let Some(embedded) = parts.next() {
        if !valid_host(embedded.strip_prefix('@')?) {
            return None;
        }
    }
    if parts.next().is_some() || !valid_host(host) || !valid_code(code) {
        return None;
    }
    Some((code.to_owned(), Some(host.to_ascii_lowercase())))
}

fn header_code(value: &str) -> Option<String> {
    let v = value.trim();
    let ok = (4..=12).contains(&v.len())
        && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !v.starts_with('-')
        && !v.ends_with('-');
    ok.then(|| v.to_owned())
}

// Keeps one char per char so indices match the original text.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

struct Candidate {
    start: usize,
    end: usize,
    code: String,
}

fn phrase_code(subject: &str, text: &str) -> Option<String> {
    let orig: Vec<char> = subject
        .chars()
        .take(SUBJECT_CHARS)
        .chain(std::iter::once('\n'))
        .chain(text.chars().take(TEXT_CHARS))
        .collect();
    let low: Vec<char> = orig.iter().copied().map(fold).collect();
    let cands = candidates(&orig, &low);
    if cands.is_empty() {
        return None;
    }
    let mut best: Option<(usize, &Candidate)> = None;
    for phrase in &patterns().phrases {
        let p: Vec<char> = phrase.chars().map(fold).collect();
        if p.is_empty() || p.len() > low.len() {
            continue;
        }
        for at in 0..=low.len() - p.len() {
            let end = at + p.len();
            if low[at..end] != p[..] {
                continue;
            }
            if at > 0 && low[at - 1].is_alphabetic() {
                continue;
            }
            if p.len() <= 4 && low.get(end).is_some_and(|c| c.is_alphabetic()) {
                continue;
            }
            // Candidates do not overlap, so the nearest on each side is the only one to look at.
            let after = cands
                .get(cands.partition_point(|c| c.start < end))
                .filter(|c| c.start - end <= AFTER)
                .map(|c| (c.start - end, c));
            let before = cands
                .partition_point(|c| c.end <= at)
                .checked_sub(1)
                .map(|j| &cands[j])
                .filter(|c| at - c.end <= BEFORE)
                .map(|c| (at - c.end, c));
            for hit in [after, before].into_iter().flatten() {
                if best.is_none_or(|(d, _)| hit.0 < d) {
                    best = Some(hit);
                }
            }
        }
    }
    best.map(|(_, c)| c.code.clone())
}

fn candidates(orig: &[char], low: &[char]) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < orig.len() {
        if !orig[i].is_alphanumeric() {
            i += 1;
            continue;
        }
        let j = run_end(orig, i);
        let run = &orig[i..j];
        let n = run.len();
        if run.iter().all(char::is_ascii_digit) {
            if let Some(k) = split_end(orig, j).filter(|_| (3..=4).contains(&n)) {
                if standalone(orig, low, i, k) {
                    let code = orig[i..j].iter().chain(&orig[j + 1..k]).collect();
                    out.push(Candidate { start: i, end: k, code });
                }
                i = k;
                continue;
            }
            if (4..=8).contains(&n) && standalone(orig, low, i, j) {
                out.push(Candidate { start: i, end: j, code: run.iter().collect() });
            }
        } else if (4..=10).contains(&n)
            && run.iter().all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
            && run.iter().any(char::is_ascii_digit)
        {
            out.push(Candidate { start: i, end: j, code: run.iter().collect() });
        }
        i = j;
    }
    out
}

fn run_end(h: &[char], start: usize) -> usize {
    h[start..]
        .iter()
        .position(|c| !c.is_alphanumeric())
        .map_or(h.len(), |p| start + p)
}

/// The end of a second half of 3-4 digits after the one space or hyphen at `j`.
fn split_end(h: &[char], j: usize) -> Option<usize> {
    if !matches!(h.get(j), Some(' ' | '-')) || !h.get(j + 1).is_some_and(|c| c.is_alphanumeric()) {
        return None;
    }
    let k = run_end(h, j + 1);
    ((3..=4).contains(&(k - j - 1)) && h[j + 1..k].iter().all(char::is_ascii_digit)).then_some(k)
}

/// False for a year, and for a number that is part of a phone number, date, time, decimal or price.
fn standalone(h: &[char], low: &[char], s: usize, e: usize) -> bool {
    let year = e - s == 4
        && h[s..e]
            .iter()
            .collect::<String>()
            .parse::<u32>()
            .is_ok_and(|y| (1900..=2099).contains(&y));
    let sep = |c: char| "-.,/: ".contains(c);
    let digit_before = s >= 2 && sep(h[s - 1]) && h[s - 2].is_ascii_digit();
    let digit_after = e + 1 < h.len() && sep(h[e]) && h[e + 1].is_ascii_digit();
    let plus = s > 0 && h[s - 1] == '+';
    !(year || plus || digit_before || digit_after || next_to_money(low, s, e))
}

fn next_to_money(low: &[char], s: usize, e: usize) -> bool {
    let mut b = s;
    if b > 0 && low[b - 1] == ' ' {
        b -= 1;
    }
    let mut k = b;
    while k > 0 && low[k - 1].is_alphabetic() {
        k -= 1;
    }
    let before_word: String = low[k..b].iter().collect();
    let a = if low.get(e) == Some(&' ') { e + 1 } else { e };
    let after_word: String = low
        .iter()
        .skip(a)
        .take_while(|c| c.is_alphabetic())
        .collect();
    (b > 0 && SIGNS.contains(&low[b - 1]))
        || low.get(a).is_some_and(|c| SIGNS.contains(c))
        || CURRENCY_CODES.contains(&before_word.as_str())
        || CURRENCY_CODES.contains(&after_word.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(subject: &str, text: &str) -> Option<String> {
        extract(subject, text, None).map(|(c, _)| c)
    }

    #[test]
    fn origin_bound_line_wins_over_a_phrase_elsewhere() {
        let text = "Your verification code is 111111\n\n@example.com #222222\n";
        assert_eq!(
            extract("", text, None),
            Some(("222222".into(), Some("example.com".into())))
        );
    }

    #[test]
    fn origin_bound_host_is_lowercased_and_embedded_host_is_allowed() {
        let text = "Hi\n@Example.COM #AB-12 @embed.example.org";
        assert_eq!(
            extract("", text, None),
            Some(("AB-12".into(), Some("example.com".into())))
        );
    }

    #[test]
    fn malformed_origin_bound_line_falls_through_to_the_phrase() {
        for last in [
            "@example.com 222222",
            "@.example.com #1",
            "@example.com #",
            "@a.com #1 @b.com extra",
            "@a.com #1 b.com",
        ] {
            let text = format!("Your verification code is 111111\n{last}");
            assert_eq!(code("", &text).as_deref(), Some("111111"), "{last}");
        }
    }

    #[test]
    fn otp_token_header_wins_over_a_phrase() {
        let got = extract("", "Your verification code is 111111", Some(" 987654 "));
        assert_eq!(got, Some(("987654".into(), None)));
    }

    #[test]
    fn a_bad_otp_token_header_is_ignored() {
        let got = extract("", "Your verification code is 111111", Some("12"));
        assert_eq!(got, Some(("111111".into(), None)));
    }

    #[test]
    fn plain_english_code() {
        assert_eq!(code("", "Your verification code is 482913.").as_deref(), Some("482913"));
    }

    #[test]
    fn code_before_the_phrase() {
        assert_eq!(
            code("", "G-482913 is your Google verification code").as_deref(),
            Some("482913")
        );
    }

    #[test]
    fn digits_split_unevenly_are_not_a_code() {
        assert_eq!(code("", "Your code: 4829 13"), None);
    }

    #[test]
    fn german_code() {
        assert_eq!(code("", "Ihr Bestätigungscode lautet: 5531").as_deref(), Some("5531"));
    }

    #[test]
    fn norwegian_code_split_by_space_or_hyphen() {
        assert_eq!(code("", "Din engangskode er 774 201").as_deref(), Some("774201"));
        assert_eq!(code("", "Din engangskode er 774-201").as_deref(), Some("774201"));
    }

    #[test]
    fn alphanumeric_code() {
        assert_eq!(code("", "Your login code is K7Q2ZP").as_deref(), Some("K7Q2ZP"));
    }

    #[test]
    fn subject_can_hold_the_code() {
        assert_eq!(code("Your security code: 665544", "Hello").as_deref(), Some("665544"));
    }

    #[test]
    fn nearest_candidate_wins() {
        let text = "Order 99887766 shipped. Your verification code is 123456";
        assert_eq!(code("", text).as_deref(), Some("123456"));
    }

    #[test]
    fn numbers_that_are_not_codes_give_none() {
        for text in [
            "verification code ... \u{a9} 2026",
            "verification code, your order total of $123456",
            "Your verification code expires at 10:30",
            "call +47 912 34 567 for your security code",
            "verification code sent on 2026-11-22",
            "verification code sent on 22.11.2026",
            "verification code, \u{20ac} 4500",
            "verification code, a price of 1,299 kr",
            "verification code, a price of 12.500000",
            "verification code, 4500 NOK",
            "verification code, 4500 usd",
            "verification code, 4500\u{20ac}",
        ] {
            assert_eq!(code("", text), None, "{text}");
        }
    }

    #[test]
    fn a_lone_number_without_a_phrase_gives_none() {
        assert_eq!(code("", "482913"), None);
        assert_eq!(code("", "Order 482913 shipped"), None);
    }

    #[test]
    fn a_phrase_inside_a_word_does_not_count() {
        assert_eq!(code("", "hotpot 482913"), None);
    }

    #[test]
    fn garbage_and_long_input_do_not_panic() {
        let digits = "1".repeat(1_000_000);
        assert_eq!(code("", &digits), None);
        let _ = code("", &format!("verification code {digits}"));
        let _ = code("", &format!("{digits}\n@example.com #1"));
        let _ = code("\u{fffd}\u{0}", "\u{202e}\u{fffd} verification code \u{0} 12\u{301}3456");
        let _ = code("", "verification code");
        let _ = code("", "\u{130}\u{130}\u{130} otp");
        let _ = code("", "@");
        let _ = code("", "");
        let emoji = code("", "\u{1f510}\u{1f510} Your verification code is \u{1f510}482913\u{1f510}");
        assert_eq!(emoji.as_deref(), Some("482913"));
        let many = "otp 123456 ".repeat(20_000);
        assert_eq!(code("", &many).as_deref(), Some("123456"));
    }
}
