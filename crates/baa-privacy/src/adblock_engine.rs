//! uBlock Origin's default lists on Brave's `adblock` engine (PLAN.md §5b). Alloy OSR cannot load
//! extensions, so uBO runs as a library: network checks here, cosmetics injected by `baa-web`.

use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use adblock::lists::{FilterSet, ParseOptions};
use adblock::request::Request;
use adblock::resources::{PermissionMask, Resource};
use adblock::Engine;
use base64::Engine as _;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use crate::ResourceType;

pub struct FilterList {
    pub id: &'static str,
    pub title: &'static str,
    /// Refresh source; its host is in [`crate::PROXY_ALLOWLIST`].
    pub url: &'static str,
    bundled: &'static [u8],
    /// uBO's own lists may use privileged (`trusted-*`) scriptlets, as in uBO.
    trusted: bool,
}

macro_rules! list {
    ($id:literal, $title:literal, $url:literal, $trusted:literal) => {
        FilterList {
            id: $id,
            title: $title,
            url: $url,
            trusted: $trusted,
            bundled: include_bytes!(concat!("../assets/filters/", $id, ".txt.gz")),
        }
    };
}

/// uBO's default selection, all on by default. Bundled copies are gzip snapshots (2026-10-22)
/// used until the first refresh lands.
pub const LISTS: &[FilterList] = &[
    list!("ublock-filters", "uBlock filters – Ads", "https://ublockorigin.github.io/uAssets/filters/filters.min.txt", true),
    list!("ublock-badware", "uBlock filters – Badware risks", "https://ublockorigin.github.io/uAssets/filters/badware.min.txt", true),
    list!("ublock-privacy", "uBlock filters – Privacy", "https://ublockorigin.github.io/uAssets/filters/privacy.min.txt", true),
    list!("ublock-quick-fixes", "uBlock filters – Quick fixes", "https://ublockorigin.github.io/uAssets/filters/quick-fixes.min.txt", true),
    list!("ublock-unbreak", "uBlock filters – Unbreak", "https://ublockorigin.github.io/uAssets/filters/unbreak.min.txt", true),
    list!("easylist", "EasyList", "https://easylist.to/easylist/easylist.txt", false),
    list!("easyprivacy", "EasyPrivacy", "https://easylist.to/easylist/easyprivacy.txt", false),
    list!("plowe", "Peter Lowe's Ad and tracking server list", "https://pgl.yoyo.org/adservers/serverlist.php?hostformat=adblockplus&showintro=1&mimetype=plaintext", false),
    list!("urlhaus", "Online Malicious URL Blocklist", "https://malware-filter.gitlab.io/urlhaus-filter/urlhaus-filter-online.txt", false),
];

// ponytail: only uBO's trivial redirect bodies (noop.js, 1x1.gif, ...) are bundled, so other
// `$redirect=` rules fall back to a plain block and `+js()` scriptlets inject nothing. Run
// assets/filters/gen-resources.mjs against a uBO checkout to bundle the full set.
const RESOURCES: &[u8] = include_bytes!("../assets/filters/resources.json.gz");
const TRUSTED: PermissionMask = PermissionMask::from_bits(1);

const REFRESH_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Block,
    /// Answer with this body instead (neutered script, 1x1 gif) via a `CefResourceHandler`;
    /// Chromium refuses redirects of subresources to `data:` URLs.
    Redirect { mime: String, body: Vec<u8> },
    /// Load this URL instead (`$removeparam` stripped tracking parameters).
    Rewrite(String),
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    disabled_lists: BTreeSet<String>,
    /// Hosts the user turned blocking off for (address-pill toggle).
    allowed_sites: BTreeSet<String>,
    my_filters: String,
}

/// Share as `Arc<Adblock>`: CEF's IO thread reads while the UI toggles. Per-Space on/off is
/// `baa_model::Space::adblock`; when it is off, do not call in at all.
///
/// `load`, `refresh`, `set_list_enabled` and `set_my_filters` build the engine, which takes about a
/// second with the default lists: call them off the UI thread.
pub struct Adblock {
    dir: PathBuf,
    engine: RwLock<Engine>,
    settings: RwLock<Settings>,
    // Two concurrent rebuilds could otherwise install the one built from older settings last.
    rebuilding: Mutex<()>,
}

impl Adblock {
    /// `dir` holds `adblock.json` and the refreshed lists.
    pub fn load(dir: &Path) -> Adblock {
        let settings: Settings = fs::read(dir.join("adblock.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Adblock {
            dir: dir.to_owned(),
            engine: RwLock::new(build(dir, &settings)),
            settings: RwLock::new(settings),
            rebuilding: Mutex::new(()),
        }
    }

    /// For `OnBeforeResourceLoad`. `source_url` is the requesting frame's URL (the URL itself for
    /// a main-frame load).
    // ponytail: the per-site switch is checked against the requesting frame, not the tab, so
    // third-party iframes on an allowed site are still filtered.
    pub fn should_block(&self, url: &str, source_url: &str, resource_type: ResourceType) -> Decision {
        if !self.site_enabled(&host(source_url)) {
            return Decision::Allow;
        }
        let Ok(request) = Request::new(url, source_url, adblock_type(resource_type), "GET") else {
            return Decision::Allow;
        };
        let r = self.engine.read().check_network_request(&request);
        if r.should_block() {
            return r.redirect.as_deref().and_then(decode_data_url).unwrap_or(Decision::Block);
        }
        r.rewritten_url.map_or(Decision::Allow, Decision::Rewrite)
    }

    /// JS to run in the frame's main world on load start: a stylesheet hiding site-specific
    /// elements plus uBO scriptlets. Empty when there is nothing to do.
    // ponytail: procedural cosmetics (`:has-text()`, `:remove()`) are dropped; they need uBO's
    // JS procedural engine.
    pub fn cosmetics_for(&self, url: &str) -> String {
        if !self.site_enabled(&host(url)) {
            return String::new();
        }
        let r = self.engine.read().url_cosmetic_resources(url);
        let mut js = String::new();
        if !r.hide_selectors.is_empty() {
            let css = hide_css(r.hide_selectors.iter());
            // adoptedStyleSheets exists before <head> does, and replaceSync drops an invalid
            // selector's rule without losing the rest.
            js += &format!(
                "(()=>{{const s=new CSSStyleSheet();s.replaceSync({});document.adoptedStyleSheets=[...document.adoptedStyleSheets,s];}})();\n",
                serde_json::to_string(&css).unwrap()
            );
        }
        if !r.injected_script.is_empty() {
            // uBO's scriptlets read a `scriptletGlobals` binding its own injector defines.
            js += &format!("(function(){{const scriptletGlobals={{}};\n{}\n}})();\n", r.injected_script);
        }
        js
    }

    /// CSS for generic `##.ad`/`###ad` rules matching the classes and ids seen in a page. Needs a
    /// page-side observer that reports them back (PLAN.md §5b), which `baa-web` owns.
    pub fn generic_css_for(&self, url: &str, classes: &[String], ids: &[String]) -> String {
        if !self.site_enabled(&host(url)) {
            return String::new();
        }
        let engine = self.engine.read();
        let r = engine.url_cosmetic_resources(url);
        if r.generichide {
            return String::new();
        }
        hide_css(engine.hidden_class_id_selectors(classes, ids, &r.exceptions).iter())
    }

    pub fn site_enabled(&self, host: &str) -> bool {
        !self.settings.read().allowed_sites.contains(host)
    }

    pub fn set_site_enabled(&self, host: &str, on: bool) {
        self.update(|s| {
            if on {
                s.allowed_sites.remove(host);
            } else {
                s.allowed_sites.insert(host.to_owned());
            }
        });
    }

    pub fn list_enabled(&self, id: &str) -> bool {
        !self.settings.read().disabled_lists.contains(id)
    }

    pub fn set_list_enabled(&self, id: &str, on: bool) {
        self.update(|s| {
            if on {
                s.disabled_lists.remove(id);
            } else {
                s.disabled_lists.insert(id.to_owned());
            }
        });
        self.rebuild();
    }

    pub fn my_filters(&self) -> String {
        self.settings.read().my_filters.clone()
    }

    pub fn set_my_filters(&self, text: &str) {
        self.update(|s| s.my_filters = text.to_owned());
        self.rebuild();
    }

    fn stale_lists(&self) -> Vec<&'static FilterList> {
        let now = SystemTime::now();
        LISTS
            .iter()
            .filter(|l| self.list_enabled(l.id))
            .filter(|l| {
                fs::metadata(cache_path(&self.dir, l))
                    .and_then(|m| m.modified())
                    .map_or(true, |t| now.duration_since(t).is_ok_and(|age| age >= REFRESH_EVERY))
            })
            .collect()
    }

    /// Call from a timer (hourly is fine; fresh lists are skipped). `fetch` does a blocking GET and
    /// returns the body; a failed fetch is retried on the next call. Rebuilds once if anything
    /// changed and returns how many lists were updated.
    pub fn refresh(&self, mut fetch: impl FnMut(&str) -> Option<String>) -> usize {
        let mut updated = 0;
        for list in self.stale_lists() {
            let Some(body) = fetch(list.url) else { continue };
            // A captive portal or error page is not a filter list.
            if body.trim().is_empty() || body.trim_start().starts_with('<') {
                log::warn!("adblock: {} returned something that is not a filter list", list.url);
                continue;
            }
            match write_atomic(&cache_path(&self.dir, list), body.as_bytes()) {
                Ok(()) => updated += 1,
                Err(e) => log::warn!("adblock: saving {}: {e}", list.id),
            }
        }
        if updated > 0 {
            self.rebuild();
        }
        updated
    }

    fn update(&self, f: impl FnOnce(&mut Settings)) {
        let mut s = self.settings.write();
        f(&mut s);
        let json = serde_json::to_vec_pretty(&*s).unwrap();
        if let Err(e) = write_atomic(&self.dir.join("adblock.json"), &json) {
            log::warn!("adblock: saving settings: {e}");
        }
    }

    fn rebuild(&self) {
        let _guard = self.rebuilding.lock();
        let engine = build(&self.dir, &self.settings.read());
        *self.engine.write() = engine;
    }
}

fn build(dir: &Path, s: &Settings) -> Engine {
    let mut set = FilterSet::new(false);
    for list in LISTS.iter().filter(|l| !s.disabled_lists.contains(l.id)) {
        let text = fs::read_to_string(cache_path(dir, list)).unwrap_or_else(|_| gunzip(list.bundled));
        let permissions = if list.trusted { TRUSTED } else { PermissionMask::default() };
        set.add_filter_list(text, ParseOptions { permissions, ..Default::default() });
    }
    // Untrusted, as uBO's "My filters" are by default: `trusted-*` scriptlets there do nothing.
    set.add_filter_list(s.my_filters.clone(), ParseOptions::default());
    let mut engine = Engine::new_with_filter_set(set);
    let resources: Vec<Resource> =
        serde_json::from_str(&gunzip(RESOURCES)).expect("bundled resources.json.gz is valid");
    engine.use_resources(resources);
    engine
}

fn cache_path(dir: &Path, list: &FilterList) -> PathBuf {
    dir.join("filters").join(format!("{}.txt", list.id))
}

fn gunzip(bytes: &[u8]) -> String {
    let mut out = String::new();
    flate2::read::GzDecoder::new(bytes).read_to_string(&mut out).expect("bundled asset is valid gzip UTF-8");
    out
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(tmp, path)
}

fn host(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default()
}

fn hide_css<'a>(selectors: impl Iterator<Item = &'a String>) -> String {
    selectors.map(|s| format!("{s}{{display:none!important}}\n")).collect()
}

// The engine hands redirects back as `data:<mime>;base64,<body>`.
fn decode_data_url(url: &str) -> Option<Decision> {
    let (mime, b64) = url.strip_prefix("data:")?.split_once(";base64,")?;
    let body = base64::prelude::BASE64_STANDARD.decode(b64).ok()?;
    Some(Decision::Redirect { mime: mime.to_owned(), body })
}

fn adblock_type(t: ResourceType) -> &'static str {
    use ResourceType::*;
    match t {
        MainFrame | NavigationPreloadMainFrame => "document",
        SubFrame | NavigationPreloadSubFrame => "subdocument",
        Stylesheet => "stylesheet",
        Script | Worker | SharedWorker | ServiceWorker => "script",
        Image | Favicon => "image",
        Font => "font",
        Object | PluginResource => "object",
        Media => "media",
        // Chromium reports fetch() as XHR too.
        Xhr => "xmlhttprequest",
        Ping => "ping",
        CspReport => "csp_report",
        SubResource | Prefetch => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An `Adblock` with every bundled list off, so tests build in milliseconds.
    fn bare(name: &str, my_filters: &str) -> Adblock {
        let dir = std::env::temp_dir().join(format!("baa-privacy-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let s = Settings {
            disabled_lists: LISTS.iter().map(|l| l.id.to_owned()).collect(),
            my_filters: my_filters.to_owned(),
            ..Default::default()
        };
        write_atomic(&dir.join("adblock.json"), &serde_json::to_vec(&s).unwrap()).unwrap();
        Adblock::load(&dir)
    }

    #[test]
    fn bundled_lists_block_known_ad_urls() {
        let dir = std::env::temp_dir().join(format!("baa-privacy-bundled-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let ab = Adblock::load(&dir);
        for url in [
            "https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js",
            "https://securepubads.g.doubleclick.net/tag/js/gpt.js",
            "https://www.google-analytics.com/analytics.js",
        ] {
            let d = ab.should_block(url, "https://example.com/", ResourceType::Script);
            assert!(matches!(d, Decision::Block | Decision::Redirect { .. }), "{url}: {d:?}");
        }
        assert_eq!(
            ab.should_block("https://example.com/app.js", "https://example.com/", ResourceType::Script),
            Decision::Allow
        );
        assert_eq!(ab.stale_lists().len(), LISTS.len());
    }

    #[test]
    fn site_toggle_allows_and_persists() {
        let ab = bare("site", "||ads.example^");
        let ad = || ab.should_block("https://ads.example/a.js", "https://news.example/", ResourceType::Script);
        assert_eq!(ad(), Decision::Block);
        ab.set_site_enabled("news.example", false);
        assert_eq!(ad(), Decision::Allow);
        assert!(!Adblock::load(&ab.dir).site_enabled("news.example"));
        ab.set_site_enabled("news.example", true);
        assert_eq!(ad(), Decision::Block);
    }

    #[test]
    fn redirect_serves_bundled_body() {
        let ab = bare("redirect", "||ads.example^$script,redirect=noop.js");
        let d = ab.should_block("https://ads.example/a.js", "https://news.example/", ResourceType::Script);
        let Decision::Redirect { mime, body } = d else { panic!("{d:?}") };
        assert_eq!(mime, "application/javascript");
        assert!(String::from_utf8(body).unwrap().contains("use strict"));
    }

    #[test]
    fn removeparam_rewrites() {
        let ab = bare("removeparam", "$removeparam=utm_source");
        let d = ab.should_block("https://shop.example/x?utm_source=a&id=1", "https://shop.example/", ResourceType::Xhr);
        assert_eq!(d, Decision::Rewrite("https://shop.example/x?id=1".into()));
    }

    #[test]
    fn cosmetics_hide_site_selectors() {
        let ab = bare("cosmetic", "news.example##.ad-banner");
        assert!(ab.cosmetics_for("https://news.example/story").contains(".ad-banner{display:none!important}"));
        assert!(ab.cosmetics_for("https://other.example/").is_empty());
    }

    #[test]
    fn generic_css_matches_seen_classes() {
        let ab = bare("generic", "##.sponsored-box");
        let css = ab.generic_css_for("https://news.example/", &["sponsored-box".into()], &[]);
        assert!(css.contains(".sponsored-box{display:none!important}"));
    }

    #[test]
    fn my_filters_rebuild() {
        let ab = bare("myfilters", "");
        let req = || ab.should_block("https://tracker.example/p", "https://a.example/", ResourceType::Image);
        assert_eq!(req(), Decision::Allow);
        ab.set_my_filters("||tracker.example^");
        assert_eq!(req(), Decision::Block);
        assert_eq!(ab.my_filters(), "||tracker.example^");
    }

    #[test]
    fn refresh_stores_and_rebuilds() {
        let ab = bare("refresh", "");
        ab.set_list_enabled("plowe", true);
        assert_eq!(ab.stale_lists().len(), 1);
        assert_eq!(ab.refresh(|_| None), 0);
        assert_eq!(ab.refresh(|_| Some("<html>portal</html>".into())), 0);
        assert_eq!(ab.refresh(|_| Some("||fresh.example^\n".into())), 1);
        assert!(ab.stale_lists().is_empty());
        assert_eq!(
            ab.should_block("https://fresh.example/x", "https://a.example/", ResourceType::Script),
            Decision::Block
        );
    }

    #[test]
    fn list_hosts_are_allowlisted() {
        for l in LISTS {
            let host = url::Url::parse(l.url).unwrap().host_str().unwrap().to_owned();
            assert!(crate::PROXY_ALLOWLIST.contains(&host.as_str()), "{host}");
        }
    }
}
