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
    GitHub(GitHub),
    /// An RSS or Atom feed, or a page that links to one.
    Feed { url: String },
}

/// Open pull requests or issues, as Arc's GitHub live folders list them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GitHub {
    pub kind: GitHubKind,
    /// Each is its own search (GitHub's search ANDs qualifiers); the folder shows them all.
    pub filters: Vec<GitHubFilter>,
    /// Qualifiers on every search: `repo:o/r`, or a list page's own query. (Folders from
    /// before filters stored their whole search here as `query`.)
    #[serde(alias = "query")]
    pub scope: String,
    /// `owner/repo`s left out of the folder.
    pub hidden_repos: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitHubKind {
    #[default]
    PullRequests,
    Issues,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitHubFilter {
    CreatedByMe,
    ReviewRequests,
    TeamReviewRequests,
    AssignedToMe,
    MentioningMe,
}

impl GitHubFilter {
    /// The ones a folder of `kind` offers, in menu order.
    pub fn for_kind(kind: GitHubKind) -> &'static [GitHubFilter] {
        use GitHubFilter::*;
        match kind {
            GitHubKind::PullRequests => &[CreatedByMe, ReviewRequests, TeamReviewRequests],
            GitHubKind::Issues => &[CreatedByMe, AssignedToMe, MentioningMe],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GitHubFilter::CreatedByMe => "Created by Me",
            GitHubFilter::ReviewRequests => "Review Requests",
            GitHubFilter::TeamReviewRequests => "Team Review Requests",
            GitHubFilter::AssignedToMe => "Assigned to Me",
            GitHubFilter::MentioningMe => "Mentioning Me",
        }
    }

    fn qualifier(self) -> &'static str {
        match self {
            GitHubFilter::CreatedByMe => "author:@me",
            GitHubFilter::ReviewRequests => "user-review-requested:@me",
            // review-requested also matches requests made to your teams.
            GitHubFilter::TeamReviewRequests => "review-requested:@me -user-review-requested:@me",
            GitHubFilter::AssignedToMe => "assignee:@me",
            GitHubFilter::MentioningMe => "mentions:@me",
        }
    }
}

impl GitHub {
    /// The searches the folder runs.
    pub fn queries(&self) -> Vec<String> {
        let has = |q: &str| self.scope.contains(q);
        let mut base = Vec::new();
        // The search API answers 422 without a type qualifier.
        if !(has("is:pr") || has("is:issue") || has("type:")) {
            base.push(if self.kind == GitHubKind::PullRequests { "is:pr" } else { "is:issue" });
        }
        if !(has("is:open") || has("is:closed") || has("is:merged") || has("state:")) {
            base.push("is:open");
        }
        if !has("archived:") {
            base.push("archived:false");
        }
        let base = base.join(" ");
        let one = |filter: &str| {
            [base.as_str(), filter, self.scope.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
        };
        match self.filters.as_slice() {
            [] => vec![one("")],
            filters => filters.iter().map(|f| one(f.qualifier())).collect(),
        }
    }

    /// Checks or unchecks `filter`. False (no change) for unchecking the last one of a folder
    /// not limited to a repo, owner or query, which would list every open item on GitHub.
    pub fn toggle(&mut self, filter: GitHubFilter) -> bool {
        if let Some(i) = self.filters.iter().position(|f| *f == filter) {
            if self.filters.len() == 1 && self.scope.trim().is_empty() {
                return false;
            }
            self.filters.remove(i);
        } else {
            self.filters.push(filter);
            let order = GitHubFilter::for_kind(self.kind);
            self.filters.sort_by_key(|f| order.iter().position(|o| o == f));
        }
        true
    }

    pub fn set_repo_hidden(&mut self, repo: &str, hidden: bool) {
        self.hidden_repos.retain(|r| r != repo);
        if hidden {
            self.hidden_repos.push(repo.into());
        }
    }
}

/// `owner/repo` of a github.com URL.
pub fn repo_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://github.com/")?;
    let mut parts = rest.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    (!owner.is_empty() && !repo.is_empty()).then(|| format!("{owner}/{repo}"))
}

/// github.com paths that are not `owner/repo`.
const GITHUB_PAGES: &[&str] = &[
    "pulls", "issues", "settings", "notifications", "orgs", "organizations", "marketplace", "explore", "topics",
    "search", "sponsors", "features", "login", "new", "codespaces", "dashboard", "trending", "collections",
    "enterprise", "pricing", "apps", "users", "copilot",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveItem {
    pub url: String,
    pub title: String,
}

const MAX_ITEMS: usize = 25;

impl LiveSource {
    /// The live folder a page stands for, with a name. Any github.com page is a GitHub
    /// folder that picks itself: a repo's issues or pull requests, a list page's own query,
    /// or (elsewhere on GitHub) your own. Any other web page is taken as a feed.
    pub fn from_page(page: &str) -> Option<(String, LiveSource)> {
        let u = url::Url::parse(page).ok().filter(|u| is_web(u.as_str()))?;
        let host = u.host_str()?.trim_start_matches("www.");
        if host != "github.com" {
            return Some((host.into(), LiveSource::Feed { url: page.into() }));
        }
        let segs: Vec<&str> = u.path_segments()?.filter(|s| !s.is_empty()).collect();
        let (repo, rest) = match segs.as_slice() {
            [owner, repo, rest @ ..] if !GITHUB_PAGES.contains(owner) => (Some(format!("{owner}/{repo}")), rest),
            all => (None, all),
        };
        let kind = if rest.first() == Some(&"issues") { GitHubKind::Issues } else { GitHubKind::PullRequests };
        // A list page's own search, unless it is a single item (`issues/12`).
        let list = matches!(rest, ["pulls" | "issues"] | ["pulls" | "issues", _] if repo.is_none() || rest.len() == 1);
        let typed = u.query_pairs().find(|(k, _)| k == "q").map(|(_, v)| v.into_owned()).filter(|_| list);
        let mut gh = GitHub { kind, ..Default::default() };
        match (&repo, typed) {
            (Some(r), q) => gh.scope = [q.unwrap_or_default(), format!("repo:{r}")].join(" ").trim().to_string(),
            (None, Some(q)) => gh.scope = q,
            (None, None) => {
                use GitHubFilter::*;
                gh.filters = match rest {
                    [_, "review-requested"] => vec![ReviewRequests],
                    [_, "assigned"] => vec![AssignedToMe],
                    [_, "mentioned"] => vec![MentioningMe],
                    ["pulls" | "issues", ..] => vec![CreatedByMe],
                    _ => GitHubFilter::for_kind(kind).to_vec(),
                };
            }
        }
        let label = if kind == GitHubKind::Issues { "Issues" } else { "Pull Requests" };
        let name = repo.map_or(label.to_string(), |r| format!("{r} {label}"));
        Some((name, LiveSource::GitHub(gh)))
    }

    /// `items` without the ones the folder hides.
    pub fn visible(&self, items: &[LiveItem]) -> Vec<LiveItem> {
        let hidden = match self {
            LiveSource::GitHub(gh) => gh.hidden_repos.as_slice(),
            LiveSource::Feed { .. } => &[],
        };
        items.iter().filter(|i| repo_of(&i.url).is_none_or(|r| !hidden.contains(&r))).cloned().collect()
    }
}

/// The source's current items, newest or most relevant first, without duplicates.
/// Hidden repos are still in; see [`LiveSource::visible`].
pub fn fetch(source: &LiveSource) -> Result<Vec<LiveItem>, String> {
    let mut items = match source {
        LiveSource::GitHub(gh) => github(&gh.queries())?,
        LiveSource::Feed { url } => feed(url)?,
    };
    let mut seen = std::collections::HashSet::new();
    items.retain(|i| seen.insert(i.url.clone()));
    if let LiveSource::Feed { .. } = source {
        items.truncate(MAX_ITEMS);
    }
    Ok(items)
}

/// Every search's items, most recently updated first.
fn github(queries: &[String]) -> Result<Vec<LiveItem>, String> {
    let token = github_token();
    if token.is_none() && queries.iter().any(|q| q.contains("@me")) {
        return Err("Sign in to GitHub: run `gh auth login`, or set GH_TOKEN".into());
    }
    let mut headers = vec!["Accept: application/vnd.github+json".to_string()];
    headers.extend(token.map(|t| format!("Authorization: Bearer {t}")));
    let mut all = Vec::new();
    for query in queries {
        let q: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let url = format!("https://api.github.com/search/issues?q={q}&sort=updated&per_page={MAX_ITEMS}");
        all.extend(github_items(&curl(&url, &headers)?)?);
    }
    // ISO 8601 times sort as text.
    all.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(all.into_iter().map(|(_, item)| item).collect())
}

/// (updated_at, item) per search result.
fn github_items(body: &str) -> Result<Vec<(String, LiveItem)>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("GitHub: {e}"))?;
    let items = v["items"]
        .as_array()
        .ok_or_else(|| format!("GitHub: {}", v["message"].as_str().unwrap_or("unexpected answer")))?;
    Ok(items
        .iter()
        .filter_map(|i| {
            let url = i["html_url"].as_str()?.into();
            let updated = i["updated_at"].as_str().unwrap_or_default().into();
            Some((updated, LiveItem { url, title: i["title"].as_str().unwrap_or_default().into() }))
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
    Ok(parse_feed(&curl(&feed_url(url, &href)?, &[])?))
}

/// The page's feed link resolved against it. The page chooses the href, so an absolute one
/// could name any scheme curl speaks (file:, gopher:, dict:…).
fn feed_url(page: &str, href: &str) -> Result<String, String> {
    let next = url::Url::parse(page).and_then(|u| u.join(href)).map_err(|e| e.to_string())?;
    is_web(next.as_str()).then(|| next.into()).ok_or_else(|| "Feed link is not http(s)".into())
}

/// curl's argv for a GET of `url`. `--proto` holds curl to http(s) for the URL itself and
/// `--proto-redir` for every redirect, whatever protocols the system curl was built with.
fn curl_args(url: &str) -> [&str; 14] {
    [
        "-sSL", "--compressed", "--max-time", "20", "--proto", "=http,https", "--proto-redir", "=http,https", "-A",
        "baa", "-H", "@-", "--", url,
    ]
}

/// GET `url`. Headers go in on stdin so a token never shows in the process list.
fn curl(url: &str, headers: &[String]) -> Result<String, String> {
    if !is_web(url) {
        return Err("Not an http(s) URL".into());
    }
    let mut child = Command::new("curl")
        .args(curl_args(url))
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
    fn feed_links_resolve_to_http_only() {
        let page = "https://blog.example/post";
        let resolve = |href: &str| {
            let html = format!(r#"<link rel="alternate" type="application/rss+xml" href="{href}">"#);
            feed_url(page, &feed_link(&html).expect("link found"))
        };
        for href in [
            "gopher://127.0.0.1:6379/_FLUSHALL%0D%0A",
            "file:///etc/passwd",
            "dict://127.0.0.1:11211/stats",
            "javascript:alert(1)",
            "&#103;opher://127.0.0.1/_x",
            "FILE:///etc/passwd",
            " smb://lan/share",
        ] {
            assert_eq!(resolve(href), Err("Feed link is not http(s)".into()), "{href}");
        }
        // Protocol-relative and relative hrefs keep the page's own web scheme.
        assert_eq!(resolve("//feeds.example/rss").as_deref(), Ok("https://feeds.example/rss"));
        assert_eq!(resolve("/feed.xml").as_deref(), Ok("https://blog.example/feed.xml"));
        assert_eq!(feed_url("http://a.example/", "//evil/x").as_deref(), Ok("http://evil/x"));
        assert!(curl("file:///etc/passwd", &[]).is_err());
    }

    #[test]
    fn curl_is_held_to_http() {
        let args = curl_args("https://a.example/");
        let flag = |f: &str| args.windows(2).any(|w| w == [f, "=http,https"]);
        assert!(flag("--proto") && flag("--proto-redir"), "{args:?}");
        let dash = args.iter().position(|a| *a == "--").unwrap();
        assert!(args.iter().position(|a| *a == "--proto-redir").unwrap() < dash);
        assert_eq!(args[dash + 1..], ["https://a.example/"]);
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
    fn github_pages_pick_their_folder() {
        use GitHubFilter::*;
        let gh = |page: &str| match LiveSource::from_page(page) {
            Some((name, LiveSource::GitHub(gh))) => (name, gh),
            other => panic!("{other:?}"),
        };
        let (name, home) = gh("https://github.com/");
        assert_eq!(name, "Pull Requests");
        assert_eq!(home.filters, [CreatedByMe, ReviewRequests, TeamReviewRequests], "GitHub's home: all of yours");
        assert_eq!(home.queries()[0], "is:pr is:open archived:false author:@me");
        assert_eq!(home.queries()[2], "is:pr is:open archived:false review-requested:@me -user-review-requested:@me");
        assert_eq!(gh("https://github.com/pulls/review-requested").1.filters, [ReviewRequests]);
        let (name, issues) = gh("https://github.com/issues/assigned");
        assert_eq!((name.as_str(), issues.kind, issues.filters.as_slice()), ("Issues", GitHubKind::Issues, &[AssignedToMe][..]));

        let (name, repo) = gh("https://github.com/o/r");
        assert_eq!((name.as_str(), repo.queries()), ("o/r Pull Requests", vec!["is:pr is:open archived:false repo:o/r".to_string()]));
        assert_eq!(gh("https://github.com/o/r/pull/7").0, "o/r Pull Requests");
        let (name, bugs) = gh("https://github.com/o/r/issues?q=is%3Aissue+label%3Abug");
        assert_eq!(name, "o/r Issues");
        assert_eq!(bugs.queries(), ["is:open archived:false is:issue label:bug repo:o/r"]);
        assert_eq!(gh("https://github.com/o/r/issues/12?q=x").1.scope, "repo:o/r", "an issue page has no list query");

        assert_eq!(
            LiveSource::from_page("https://www.example.com/blog"),
            Some(("example.com".into(), LiveSource::Feed { url: "https://www.example.com/blog".into() }))
        );
        assert_eq!(LiveSource::from_page("baa://settings"), None);
        let body = r#"{"items":[{"html_url":"https://github.com/o/r/pull/1","title":"Fix","updated_at":"2026-11-07T00:00:00Z"}]}"#;
        assert_eq!(github_items(body), Ok(vec![("2026-11-07T00:00:00Z".into(), item("https://github.com/o/r/pull/1", "Fix"))]));
        assert_eq!(github_items(r#"{"message":"Bad credentials"}"#), Err("GitHub: Bad credentials".into()));
    }

    #[test]
    fn filters_and_hidden_repos() {
        use GitHubFilter::*;
        let mut mine = GitHub { filters: vec![ReviewRequests], ..Default::default() };
        assert!(mine.toggle(CreatedByMe));
        assert_eq!(mine.filters, [CreatedByMe, ReviewRequests], "kept in menu order");
        assert!(mine.toggle(CreatedByMe));
        assert!(!mine.toggle(ReviewRequests), "the last filter stays: it would be all of GitHub");
        let mut repo = GitHub { scope: "repo:o/r".into(), filters: vec![CreatedByMe], ..Default::default() };
        assert!(repo.toggle(CreatedByMe), "a repo folder may show all its open items");

        mine.set_repo_hidden("o/r", true);
        let source = LiveSource::GitHub(mine);
        let items = [item("https://github.com/o/r/pull/1", "a"), item("https://github.com/o/s/pull/2", "b")];
        assert_eq!(source.visible(&items), [items[1].clone()]);
        assert_eq!(repo_of("https://github.com/o/r/pull/1").as_deref(), Some("o/r"));

        // A folder saved before filters existed still loads, with its search as the scope.
        let old: LiveSource = serde_json::from_str(r#"{"GitHub":{"query":"is:pr is:open author:@me archived:false"}}"#).unwrap();
        let LiveSource::GitHub(old) = old else { panic!() };
        assert_eq!(old.queries(), ["is:pr is:open author:@me archived:false"]);
    }
}
