//! Live folders, as in Arc: a folder whose tabs mirror a GitHub search or a feed.
//!
//! Fetching shells out to the system `curl` (macOS and Windows 10+ ship it), so the
//! workspace needs no HTTP client. [`fetch`] blocks; call it off the UI thread.

use std::io::Write;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::is_web;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LiveSource {
    /// A GitHub issue search, e.g. `is:open is:pr author:@me`.
    GitHub { query: String },
    /// An RSS or Atom feed, or a page that links to one.
    Feed { url: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveItem {
    pub url: String,
    pub title: String,
}

const MAX_ITEMS: usize = 25;

impl LiveSource {
    /// The live folder a page stands for, with a name: a GitHub pull request or issue
    /// list keeps its filter; any other web page is taken as a feed.
    pub fn from_page(page: &str) -> Option<(String, LiveSource)> {
        let u = url::Url::parse(page).ok().filter(|u| is_web(u.as_str()))?;
        let host = u.host_str()?.trim_start_matches("www.");
        if host == "github.com" {
            let segs: Vec<&str> = u.path_segments()?.filter(|s| !s.is_empty()).collect();
            let (repo, kind, filter) = match segs.as_slice() {
                [kind @ ("pulls" | "issues")] => (None, *kind, None),
                [kind @ ("pulls" | "issues"), filter] => (None, *kind, Some(*filter)),
                [owner, repo, kind @ ("pulls" | "issues")] => (Some(format!("{owner}/{repo}")), *kind, None),
                _ => return Some((host.into(), LiveSource::Feed { url: page.into() })),
            };
            let pr = kind == "pulls";
            let mine = match filter {
                Some("assigned") => "assignee:@me",
                Some("mentioned") => "mentions:@me",
                Some("review-requested") => "review-requested:@me",
                _ => "author:@me",
            };
            let typed = u.query_pairs().find(|(k, _)| k == "q").map(|(_, v)| v.into_owned());
            let mut query = match (&repo, typed) {
                (_, Some(q)) => q,
                (Some(_), None) => "is:open".into(),
                (None, None) => format!("is:open {mine} archived:false"),
            };
            // The search API answers 422 without a type qualifier.
            if !query.contains("is:pr") && !query.contains("is:issue") && !query.contains("type:") {
                query = format!("{} {query}", if pr { "is:pr" } else { "is:issue" });
            }
            if let Some(r) = repo.as_ref().filter(|_| !query.contains("repo:")) {
                query = format!("{query} repo:{r}");
            }
            let label = match (pr, filter) {
                (true, Some("review-requested")) => "Review Requests",
                (true, _) => "Pull Requests",
                (false, _) => "Issues",
            };
            let name = format!("{} {label}", repo.as_deref().unwrap_or("GitHub"));
            return Some((name, LiveSource::GitHub { query }));
        }
        Some((host.into(), LiveSource::Feed { url: page.into() }))
    }
}

/// The source's current items, newest or most relevant first, without duplicates.
pub fn fetch(source: &LiveSource) -> Result<Vec<LiveItem>, String> {
    let mut items = match source {
        LiveSource::GitHub { query } => github(query)?,
        LiveSource::Feed { url } => feed(url)?,
    };
    let mut seen = std::collections::HashSet::new();
    items.retain(|i| seen.insert(i.url.clone()));
    items.truncate(MAX_ITEMS);
    Ok(items)
}

fn github(query: &str) -> Result<Vec<LiveItem>, String> {
    let token = github_token();
    if token.is_none() && query.contains("@me") {
        return Err("Sign in to GitHub: run `gh auth login`, or set GH_TOKEN".into());
    }
    let q: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
    let url = format!("https://api.github.com/search/issues?q={q}&sort=updated&per_page={MAX_ITEMS}");
    let mut headers = vec!["Accept: application/vnd.github+json".to_string()];
    headers.extend(token.map(|t| format!("Authorization: Bearer {t}")));
    github_items(&curl(&url, &headers)?)
}

fn github_items(body: &str) -> Result<Vec<LiveItem>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("GitHub: {e}"))?;
    let items = v["items"]
        .as_array()
        .ok_or_else(|| format!("GitHub: {}", v["message"].as_str().unwrap_or("unexpected answer")))?;
    Ok(items
        .iter()
        .filter_map(|i| {
            let url = i["html_url"].as_str()?.into();
            Some(LiveItem { url, title: i["title"].as_str().unwrap_or_default().into() })
        })
        .collect())
}

/// `GH_TOKEN`/`GITHUB_TOKEN`, else the GitHub CLI's login.
fn github_token() -> Option<String> {
    let env = ["GH_TOKEN", "GITHUB_TOKEN"].into_iter().find_map(|v| std::env::var(v).ok().filter(|t| !t.is_empty()));
    // Apps started from Finder get a bare PATH, so Homebrew's gh is also tried by path.
    env.or_else(|| {
        ["gh", "/opt/homebrew/bin/gh", "/usr/local/bin/gh"].into_iter().find_map(|gh| {
            let out = Command::new(gh).args(["auth", "token"]).output().ok().filter(|o| o.status.success())?;
            Some(String::from_utf8(out.stdout).ok()?.trim().to_string()).filter(|t| !t.is_empty())
        })
    })
}

// ponytail: a page that only links to its feed costs two fetches every refresh; store the
// discovered feed URL in the source if that matters.
fn feed(url: &str) -> Result<Vec<LiveItem>, String> {
    let body = curl(url, &[])?;
    if body.contains("<rss") || body.contains("<feed") {
        return Ok(parse_feed(&body));
    }
    let href = feed_link(&body).ok_or("No RSS or Atom feed on this page")?;
    let next = url::Url::parse(url).and_then(|u| u.join(&href)).map_err(|e| e.to_string())?;
    Ok(parse_feed(&curl(next.as_str(), &[])?))
}

/// GET `url`. Headers go in on stdin so a token never shows in the process list.
fn curl(url: &str, headers: &[String]) -> Result<String, String> {
    let mut child = Command::new("curl")
        .args(["-sSL", "--compressed", "--max-time", "20", "-A", "baa", "-H", "@-", "--", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    child.stdin.take().expect("piped").write_all(headers.join("\n").as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

// ponytail: string scanning, not an XML parser. Ceiling: feeds that nest <item>/<entry>
// or hide links in namespaced tags; switch to quick-xml if real feeds trip it.
/// RSS `<item>`s or Atom `<entry>`s, in document order.
pub fn parse_feed(xml: &str) -> Vec<LiveItem> {
    let rss = elements(xml, "item");
    let entries = if rss.is_empty() { elements(xml, "entry") } else { rss };
    entries
        .into_iter()
        .filter_map(|(_, entry)| {
            let links = elements(entry, "link");
            // Atom: <link rel="alternate" href=…/>; RSS: <link>url</link>.
            let (attrs, inner) = links.into_iter().find(|(a, _)| matches!(attr(a, "rel"), None | Some("alternate")))?;
            let url = attr(attrs, "href").map(decode).unwrap_or_else(|| text(inner));
            let title = elements(entry, "title").first().map(|(_, t)| text(t)).unwrap_or_default();
            is_web(&url).then_some(LiveItem { url, title })
        })
        .take(MAX_ITEMS)
        .collect()
}

/// An HTML page's `<link rel="alternate" type="application/rss+xml|atom+xml" href>`.
fn feed_link(html: &str) -> Option<String> {
    html.split("<link").skip(1).find_map(|s| {
        let attrs = &s[..s.find('>')?];
        let kind = attr(attrs, "type")?;
        (kind == "application/rss+xml" || kind == "application/atom+xml").then(|| attr(attrs, "href").map(decode))?
    })
}

/// Each `<tag …>inner</tag>` (or `<tag …/>`) as (attributes, inner).
fn elements<'a>(xml: &'a str, tag: &str) -> Vec<(&'a str, &'a str)> {
    let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        // `<link` is not `<linkedin`.
        if !after.starts_with([' ', '>', '/', '\t', '\n', '\r']) {
            rest = after;
            continue;
        }
        let Some(gt) = after.find('>') else { break };
        let attrs = &after[..gt];
        if let Some(attrs) = attrs.strip_suffix('/') {
            out.push((attrs, ""));
            rest = &after[gt + 1..];
            continue;
        }
        let body = &after[gt + 1..];
        let Some(end) = body.find(&close) else { break };
        out.push((attrs, &body[..end]));
        rest = &body[end + close.len()..];
    }
    out
}

fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    ['"', '\''].into_iter().find_map(|q| {
        let key = format!(" {name}={q}");
        let v = &attrs[attrs.find(&key)? + key.len()..];
        Some(&v[..v.find(q)?])
    })
}

fn text(inner: &str) -> String {
    let t = inner.trim();
    match t.strip_prefix("<![CDATA[").and_then(|c| c.strip_suffix("]]>")) {
        Some(raw) => raw.trim().to_string(),
        None => decode(t),
    }
}

fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let ch = rest.find(';').filter(|&e| e <= 10).and_then(|end| {
            let c = match &rest[1..end] {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                e => {
                    let n = e.strip_prefix("#x").map(|h| u32::from_str_radix(h, 16));
                    char::from_u32(n.unwrap_or_else(|| e.strip_prefix('#').unwrap_or("x").parse()).ok()?)?
                }
            };
            Some((c, end))
        });
        match ch {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(url: &str, title: &str) -> LiveItem {
        LiveItem { url: url.into(), title: title.into() }
    }

    #[test]
    fn rss_and_atom() {
        let rss = r#"<rss><channel><title>Blog</title><link>https://blog</link>
            <item><title><![CDATA[First & best]]></title><link> https://blog/1 </link></item>
            <item><title>Tom &amp; Jerry&#39;s &#x263A;</title><link>https://blog/2</link></item>
            <item><title>No link</title></item></channel></rss>"#;
        assert_eq!(parse_feed(rss), [item("https://blog/1", "First & best"), item("https://blog/2", "Tom & Jerry's ☺")]);
        let atom = r#"<feed><title>Ch</title><entry><title type="html">Video</title>
            <link rel="self" href="https://self"/><link rel="alternate" href="https://yt/v?a=1&amp;b=2"/></entry></feed>"#;
        assert_eq!(parse_feed(atom), [item("https://yt/v?a=1&b=2", "Video")]);
        let html = r#"<head><link rel="icon" href="/i"><link rel="alternate" type="application/atom+xml" href="/feed.xml"></head>"#;
        assert_eq!(feed_link(html).as_deref(), Some("/feed.xml"));
    }

    #[test]
    fn github_pages_become_queries() {
        let q = |page: &str| match LiveSource::from_page(page) {
            Some((name, LiveSource::GitHub { query })) => (name, query),
            other => panic!("{other:?}"),
        };
        assert_eq!(q("https://github.com/pulls"), ("GitHub Pull Requests".into(), "is:pr is:open author:@me archived:false".into()));
        assert_eq!(q("https://github.com/pulls/review-requested").1, "is:pr is:open review-requested:@me archived:false");
        assert_eq!(q("https://github.com/o/r/pulls"), ("o/r Pull Requests".into(), "is:pr is:open repo:o/r".into()));
        assert_eq!(q("https://github.com/o/r/issues?q=is%3Aissue+label%3Abug").1, "is:issue label:bug repo:o/r");
        assert_eq!(
            LiveSource::from_page("https://www.example.com/blog"),
            Some(("example.com".into(), LiveSource::Feed { url: "https://www.example.com/blog".into() }))
        );
        assert_eq!(LiveSource::from_page("baa://settings"), None);
        let body = r#"{"items":[{"html_url":"https://github.com/o/r/pull/1","title":"Fix"}]}"#;
        assert_eq!(github_items(body), Ok(vec![item("https://github.com/o/r/pull/1", "Fix")]));
        assert_eq!(github_items(r#"{"message":"Bad credentials"}"#), Err("GitHub: Bad credentials".into()));
    }
}
