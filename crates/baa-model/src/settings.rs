use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SidebarSide {
    #[default]
    Left,
    Right,
}

/// Window chrome look: Arc's Space gradients, or a monospace terminal frame
/// (pen.dev "1. ASCII Chrome").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ChromeStyle {
    #[default]
    Arc,
    Ascii,
}

/// Where DevTools opens: docked beside or below the page, as Chrome does, or
/// in a window of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DevToolsDock {
    #[default]
    Right,
    Bottom,
    Left,
    Window,
}

impl DevToolsDock {
    /// DevTools' own name for a docked side; `None` for a window.
    pub fn side(self) -> Option<&'static str> {
        match self {
            Self::Right => Some("right"),
            Self::Bottom => Some("bottom"),
            Self::Left => Some("left"),
            Self::Window => None,
        }
    }

    /// From DevTools' name; "undocked" (or anything else) is a window.
    pub fn from_side(side: &str) -> Self {
        match side {
            "right" => Self::Right,
            "bottom" => Self::Bottom,
            "left" => Self::Left,
            _ => Self::Window,
        }
    }
}

/// `url` holds `%s` where the encoded query goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchEngine {
    pub name: String,
    pub url: String,
}

/// Choices offered in settings; the first is the default. Users may also enter their own.
pub const SEARCH_ENGINES: &[(&str, &str)] = &[
    ("Google", "https://www.google.com/search?q=%s"),
    ("DuckDuckGo", "https://duckduckgo.com/?q=%s"),
    ("Brave Search", "https://search.brave.com/search?q=%s"),
    ("Startpage", "https://www.startpage.com/do/search?q=%s"),
    ("Kagi", "https://kagi.com/search?q=%s"),
    ("Bing", "https://www.bing.com/search?q=%s"),
];

impl Default for SearchEngine {
    fn default() -> Self {
        let (name, url) = SEARCH_ENGINES[0];
        SearchEngine { name: name.into(), url: url.into() }
    }
}

/// Global settings. Stored as JSON so adding a field needs no migration; missing
/// fields take their defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub search_engine: SearchEngine,
    /// Search-as-you-type requests to the engine (Arc does this); turning it
    /// off means typing contacts nobody (PLAN.md §5).
    pub search_suggest: bool,
    pub sidebar_side: SidebarSide,
    /// Forces reduced motion; the UI also honours the OS setting.
    pub reduce_motion: bool,
    /// Today tabs idle this long are archived (PLAN.md §3); 0 = never.
    pub archive_after_hours: u32,
    pub chrome: ChromeStyle,
    /// Extension ids kept out of the toolbar; they stay in the extensions menu.
    pub unpinned_extensions: Vec<String>,
    pub devtools_dock: DevToolsDock,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            search_engine: SearchEngine::default(),
            search_suggest: true,
            sidebar_side: SidebarSide::Left,
            reduce_motion: false,
            archive_after_hours: 12,
            chrome: ChromeStyle::Arc,
            unpinned_extensions: Vec::new(),
            devtools_dock: DevToolsDock::default(),
        }
    }
}

impl Settings {
    /// Command-bar text → URL to load: a URL as typed, a bare host gets `https://`
    /// (`http://` for loopback, where dev servers rarely have TLS), anything else is a search.
    // ponytail: heuristic only (no public-suffix list); "1.5" counts as an IPv4 host.
    pub fn resolve_input(&self, text: &str) -> String {
        let text = text.trim();
        if !text.contains(char::is_whitespace) {
            if text.contains("://") || ["about:", "data:", "file:", "mailto:"].iter().any(|p| text.starts_with(p)) {
                return text.to_string();
            }
            if let Ok(mut u) = url::Url::parse(&format!("https://{text}")) {
                let (hosty, loopback) = match u.host() {
                    Some(url::Host::Domain(d)) => (
                        d == "localhost"
                            || d.rsplit_once('.').is_some_and(|(_, tld)| !tld.is_empty() && tld.chars().all(char::is_alphabetic)),
                        d == "localhost",
                    ),
                    Some(url::Host::Ipv4(ip)) => (true, ip.is_loopback()),
                    Some(url::Host::Ipv6(ip)) => (true, ip.is_loopback()),
                    None => (false, false),
                };
                if loopback {
                    u.set_scheme("http").expect("https -> http is allowed");
                }
                if hosty {
                    return u.into();
                }
            }
        }
        let q: String = url::form_urlencoded::byte_serialize(text.as_bytes()).collect();
        self.search_engine.url.replace("%s", &q)
    }

    /// The search engine's suggestion endpoint for `query` (OpenSearch JSON:
    /// `[query, [suggestion, ...]]`), for engines that have a known one.
    pub fn suggest_url(&self, query: &str) -> Option<String> {
        let host = url::Url::parse(&self.search_engine.url).ok()?.host_str()?.to_string();
        let endpoint = match host.trim_start_matches("www.") {
            "google.com" => "https://suggestqueries.google.com/complete/search?client=firefox&q=%s",
            "duckduckgo.com" => "https://duckduckgo.com/ac/?type=list&q=%s",
            "search.brave.com" => "https://search.brave.com/api/suggest?q=%s",
            "bing.com" => "https://api.bing.com/osjson.aspx?query=%s",
            _ => return None,
        };
        let q: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        Some(endpoint.replace("%s", &q))
    }
}

/// Suggestions from an OpenSearch response, at most `max`, without `query` itself.
pub fn parse_suggestions(body: &str, query: &str, max: usize) -> Vec<String> {
    let Ok(serde_json::Value::Array(parts)) = serde_json::from_str(body) else {
        return Vec::new();
    };
    let Some(serde_json::Value::Array(list)) = parts.get(1) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|v| v.as_str())
        .filter(|s| !s.eq_ignore_ascii_case(query.trim()))
        .take(max)
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_input() {
        let s = Settings::default();
        assert_eq!(s.resolve_input("example.com"), "https://example.com/");
        assert_eq!(s.resolve_input("localhost:3000/x"), "http://localhost:3000/x");
        assert_eq!(s.resolve_input("127.0.0.1:8080"), "http://127.0.0.1:8080/");
        assert_eq!(s.resolve_input("1.1.1.1"), "https://1.1.1.1/");
        assert_eq!(s.resolve_input("http://a.b"), "http://a.b");
        assert_eq!(s.resolve_input("rust lang"), "https://www.google.com/search?q=rust+lang");
        assert_eq!(s.resolve_input("foo"), "https://www.google.com/search?q=foo");
        assert_eq!(s.resolve_input("c++"), "https://www.google.com/search?q=c%2B%2B");
    }

    #[test]
    fn suggestions() {
        let s = Settings::default();
        assert_eq!(
            s.suggest_url("rust lang").unwrap(),
            "https://suggestqueries.google.com/complete/search?client=firefox&q=rust+lang"
        );
        let body = r#"["rust",["rust","rust lang","rustup"]]"#;
        assert_eq!(parse_suggestions(body, "rust", 4), ["rust lang", "rustup"]);
        assert!(parse_suggestions("not json", "x", 4).is_empty());
    }

    #[test]
    fn missing_fields_default() {
        let s: Settings = serde_json::from_str(r#"{"reduce_motion":true}"#).unwrap();
        assert!(s.reduce_motion);
        assert_eq!(s.search_engine.name, "Google");
        assert_eq!(s.archive_after_hours, 12);
    }
}
