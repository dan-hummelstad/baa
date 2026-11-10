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

/// Where a link another app opens goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ExternalLinks {
    #[default]
    LilBaa,
    Tab,
}

/// Light or dark: the prefers-color-scheme websites see (`page_scheme`), or
/// baa's own look (`appearance`). `System` is macOS's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PageScheme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Cookies {
    AllowAll,
    #[default]
    BlockThirdParty,
    BlockAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Https {
    Off,
    /// Upgrade when possible (Helium's default).
    #[default]
    Balanced,
    /// Always, warn before falling back.
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Preload {
    #[default]
    Off,
    Standard,
    Extended,
}

impl SecureDns {
    pub const CLOUDFLARE: &'static str = "https://chrome.cloudflare-dns.com/dns-query";
    pub const QUAD9: &'static str = "https://dns.quad9.net/dns-query";

    /// A DNS-over-HTTPS template Chrome can use: https and a host.
    pub fn valid_template(t: &str) -> bool {
        t.strip_prefix("https://")
            .and_then(|rest| rest.split(['/', '?']).next())
            .is_some_and(|host| !host.is_empty() && !host.contains(char::is_whitespace))
            && !t.contains(char::is_whitespace)
    }
}

/// Cloudflare and Quad9 are `Custom` with their template.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SecureDns {
    Off,
    #[default]
    Automatic,
    Custom(String),
}

/// Browser-behaviour settings that baa pushes to Chrome's prefs in every profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WebPrefs {
    /// `None`: Chrome's own (the system Downloads folder).
    pub download_dir: Option<std::path::PathBuf>,
    pub ask_where_to_save: bool,
    /// Comma-separated language codes; `None` is the system's, and nothing is pushed.
    pub languages: Option<String>,
    pub spellcheck: bool,
    /// 1.0 is 100%.
    pub default_zoom: f64,
    pub font_size: u32,
    /// 0 is off.
    pub min_font_size: u32,
    pub focus_highlight: bool,
    pub caret_browsing: bool,
    pub cookies: Cookies,
    pub clear_cookies_on_quit: bool,
    pub https: Https,
    pub preload: Preload,
    pub secure_dns: SecureDns,
    pub do_not_track: bool,
    pub global_privacy_control: bool,
    /// Applies when baa restarts.
    pub hardware_acceleration: bool,
    /// Off: it discards hidden tabs, which baa's page views do not expect yet.
    pub memory_saver: bool,
    /// Extension developer mode (load unpacked, reload).
    pub developer_mode: bool,
    /// Only the kinds the user changed; the others are Chrome's ([`crate::Permission::chrome_default`]).
    pub site_defaults: Vec<(crate::Permission, crate::PermissionState)>,
}

impl Default for WebPrefs {
    fn default() -> Self {
        WebPrefs {
            download_dir: None,
            ask_where_to_save: false,
            languages: None,
            spellcheck: true,
            default_zoom: 1.0,
            font_size: 16,
            min_font_size: 0,
            focus_highlight: false,
            caret_browsing: false,
            cookies: Cookies::default(),
            clear_cookies_on_quit: false,
            https: Https::default(),
            preload: Preload::default(),
            secure_dns: SecureDns::default(),
            do_not_track: false,
            global_privacy_control: false,
            hardware_acceleration: true,
            memory_saver: false,
            developer_mode: false,
            site_defaults: Vec::new(),
        }
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
    /// Extension ids every Space has: baa installs them into each Space's profile.
    pub shared_extensions: Vec<String>,
    pub devtools_dock: DevToolsDock,
    /// false archives Today tabs at startup instead of restoring them.
    pub restore_today: bool,
    pub external_links: ExternalLinks,
    /// Links that open a new tab show in Peek first.
    pub peek_links: bool,
    /// A playing video floats in Picture in Picture when you switch tab or Space
    /// (Chrome's auto PiP content setting at Allow, and a Space switch asks for it).
    pub auto_pip: bool,
    pub page_scheme: PageScheme,
    /// baa's own windows; websites keep `page_scheme`.
    pub appearance: PageScheme,
    /// Engines the user added, beside [`SEARCH_ENGINES`].
    pub search_engines: Vec<SearchEngine>,
    pub web: WebPrefs,
    /// A site's local, staging and production origins: the header names the one a tab is on,
    /// and ⌥⌘E moves the tab to the next.
    pub environments: Vec<Project>,
    /// A red count of console errors on localhost tabs' rows (E4).
    pub console_badge: bool,
    pub claude: Claude,
}

/// Claude Code's access to the browser (PLAN-m4-features D). Off by default: nothing is
/// reachable until the user turns it on in Settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Claude {
    pub enabled: bool,
    /// The Spaces Claude can see. Ephemeral Spaces never, whatever this says.
    pub allowed_spaces: Vec<crate::SpaceId>,
    /// The user's path to `claude`, used over `found_path`.
    pub claude_path: Option<String>,
    /// Where `$SHELL -lc 'command -v claude'` found it, cached because a login shell is slow.
    pub found_path: Option<String>,
    /// The `evaluate` tool (any JavaScript); even then every call asks.
    pub evaluate_allowed: bool,
    /// Hex of 32 random bytes that `baa mcp` must present (as `BAA_AGENT_TOKEN`) before the
    /// connect sheet; baa-ui makes a new one on every off→on and clears it on off. Empty matches nothing.
    pub token: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub name: String,
    pub envs: Vec<Env>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Env {
    pub label: String,
    /// `scheme://host[:port]`, as [`Env::origin_of`] wrote it.
    pub origin: String,
}

impl Env {
    /// Typed text as an env origin: http(s) with a host, and nothing after it. A bare host gets
    /// `https://`, or `http://` on loopback, as the command bar does.
    pub fn origin_of(text: &str) -> Option<String> {
        let text = text.trim();
        let typed = if text.contains("://") { text.to_string() } else { format!("https://{text}") };
        let mut u = url::Url::parse(&typed).ok()?;
        let loopback = match u.host()? {
            url::Host::Domain(d) => d == "localhost",
            url::Host::Ipv4(ip) => ip.is_loopback(),
            url::Host::Ipv6(ip) => ip.is_loopback(),
        };
        if loopback && !text.contains("://") {
            u.set_scheme("http").ok()?;
        }
        let bare = u.username().is_empty() && u.password().is_none() && u.path() == "/" && u.query().is_none() && u.fragment().is_none();
        (crate::store::is_web(u.as_str()) && bare).then(|| u.origin().ascii_serialization())
    }

    fn origin(&self) -> Option<url::Origin> {
        url::Url::parse(&self.origin).ok().map(|u| u.origin())
    }
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
            shared_extensions: Vec::new(),
            devtools_dock: DevToolsDock::default(),
            restore_today: true,
            external_links: ExternalLinks::default(),
            peek_links: true,
            auto_pip: true,
            page_scheme: PageScheme::default(),
            appearance: PageScheme::default(),
            search_engines: Vec::new(),
            web: WebPrefs::default(),
            environments: Vec::new(),
            console_badge: true,
            claude: Claude::default(),
        }
    }
}

impl Settings {
    /// The env whose origin `url` is on, and its project.
    pub fn env_of(&self, url: &str) -> Option<(&Project, &Env)> {
        let origin = url::Url::parse(url).ok()?.origin();
        self.environments.iter().find_map(|p| p.envs.iter().find(|e| e.origin() == Some(origin.clone())).map(|e| (p, e)))
    }

    /// `url`'s path and query on another env of its project: the one labelled `label`, or the next.
    pub fn env_url(&self, url: &str, label: Option<&str>) -> Option<String> {
        let (project, here) = self.env_of(url)?;
        let to = match label {
            Some(l) => project.envs.iter().find(|e| e.label.eq_ignore_ascii_case(l.trim()))?,
            None => {
                let i = project.envs.iter().position(|e| std::ptr::eq(e, here))?;
                &project.envs[(i + 1) % project.envs.len()]
            }
        };
        let from = url::Url::parse(url).ok()?;
        let mut next = url::Url::parse(&to.origin).ok()?;
        next.set_path(from.path());
        next.set_query(from.query());
        Some(next.into())
    }

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
    fn dns_templates_need_https_and_a_host() {
        assert!(SecureDns::valid_template(SecureDns::CLOUDFLARE) && SecureDns::valid_template("https://dns.example/q{?dns}"));
        for bad in ["", "https://", "http://dns.example", "dns.example", "https://a b/"] {
            assert!(!SecureDns::valid_template(bad), "{bad:?}");
        }
    }

    #[test]
    fn missing_fields_default() {
        let s: Settings = serde_json::from_str(r#"{"reduce_motion":true}"#).unwrap();
        assert!(s.reduce_motion);
        assert_eq!(s.search_engine.name, "Google");
        assert_eq!(s.archive_after_hours, 12);
        assert!(s.console_badge, "the console badge is on for settings saved before it");
        assert_eq!(s.claude, Claude::default());
        assert!(!s.claude.enabled && !s.claude.evaluate_allowed && s.claude.allowed_spaces.is_empty());
        assert!(s.claude.token.is_empty(), "settings saved before pairing have no token, so nothing pairs");
    }

    #[test]
    fn old_json_loads_and_new_fields_round_trip() {
        let s: Settings = serde_json::from_str(r#"{"search_engine":{"name":"Kagi","url":"https://kagi.com/?q=%s"},"chrome":"Ascii"}"#).unwrap();
        assert_eq!(s.web, WebPrefs::default());
        assert!(s.restore_today && s.peek_links && s.auto_pip);
        assert_eq!((s.appearance, s.page_scheme), (PageScheme::System, PageScheme::System), "baa's look follows the system until set");
        let mut s = Settings::default();
        s.web.secure_dns = SecureDns::Custom("https://dns.quad9.net/dns-query".into());
        s.web.site_defaults = vec![(crate::Permission::Popups, crate::PermissionState::Allow)];
        s.web.developer_mode = true;
        s.auto_pip = false;
        s.appearance = PageScheme::Dark;
        s.web.download_dir = Some("/tmp/dl".into());
        s.search_engines = vec![SearchEngine { name: "X".into(), url: "https://x/?q=%s".into() }];
        s.environments = vec![Project { name: "baa".into(), envs: vec![Env { label: "Local".into(), origin: "http://localhost:3000".into() }] }];
        s.claude = Claude { enabled: true, allowed_spaces: vec![crate::SpaceId(3)], claude_path: Some("/x/claude".into()), found_path: None, evaluate_allowed: true, token: "ab".repeat(32) };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn env_origins_are_bare_web_origins() {
        assert_eq!(Env::origin_of("localhost:3000").as_deref(), Some("http://localhost:3000"));
        assert_eq!(Env::origin_of("127.0.0.1:8080/").as_deref(), Some("http://127.0.0.1:8080"));
        assert_eq!(Env::origin_of("staging.example.com").as_deref(), Some("https://staging.example.com"));
        assert_eq!(Env::origin_of(" https://Example.com:443 ").as_deref(), Some("https://example.com"));
        assert_eq!(Env::origin_of("https://localhost:3000").as_deref(), Some("https://localhost:3000"), "a typed scheme stays");
        for bad in ["", "file:///etc", "javascript:alert(1)", "https://a.com/path", "https://a.com/?q", "https://u:p@a.com", "ftp://a.com", "rust lang"] {
            assert_eq!(Env::origin_of(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn env_url_keeps_path_and_query_and_cycles() {
        let env = |label: &str, origin: &str| Env { label: label.into(), origin: origin.into() };
        let s = Settings {
            environments: vec![Project {
                name: "shop".into(),
                envs: vec![env("Local", "http://localhost:3000"), env("Staging", "https://staging.shop.com"), env("Prod", "https://shop.com")],
            }],
            ..Default::default()
        };
        assert_eq!(s.env_of("https://shop.com/cart?id=1").map(|(p, e)| (p.name.as_str(), e.label.as_str())), Some(("shop", "Prod")));
        assert_eq!(s.env_of("https://shop.com:8443/"), None, "another port is another origin");
        assert_eq!(s.env_of("http://shop.com/"), None, "and so is another scheme");
        assert_eq!(s.env_url("http://localhost:3000/cart?id=1#top", None).as_deref(), Some("https://staging.shop.com/cart?id=1"));
        assert_eq!(s.env_url("https://shop.com/a", None).as_deref(), Some("http://localhost:3000/a"), "wraps around");
        assert_eq!(s.env_url("https://staging.shop.com/a", Some("prod")).as_deref(), Some("https://shop.com/a"));
        assert_eq!(s.env_url("https://staging.shop.com/a", Some("qa")), None);
        assert_eq!(s.env_url("https://other.com/a", None), None);
    }
}
