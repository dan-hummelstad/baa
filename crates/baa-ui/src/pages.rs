//! Where a `baa://` (or `chrome://`) address goes: baa's own settings window,
//! history, downloads, or Chrome's WebUI in a normal tab. One table serves
//! routing and the command bar's autofill. Routing runs only where an
//! address enters (command bar, links, tabs Chrome opened), never on a page's
//! own navigation, so a WebUI page's links keep working inside it.

use crate::settings::Tab;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Route {
    Settings(Tab, Option<&'static str>),
    Extensions,
    History,
    Downloads,
    /// Chrome draws it, in a normal tab.
    WebUi,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Page {
    pub url: &'static str,
    pub label: &'static str,
    pub keywords: &'static str,
    pub route: Route,
}

const fn page(url: &'static str, label: &'static str, keywords: &'static str, route: Route) -> Page {
    Page { url, label, keywords, route }
}

use Route::{Settings as S, WebUi};
use Tab::*;

pub(crate) static PAGES: &[Page] = &[
    page("baa://settings", "Settings", "preferences options", S(General, None)),
    page("baa://settings/search", "Settings: Search", "engine", S(Search, None)),
    page("baa://settings/privacy", "Settings: Privacy and Security", "cookies https", S(Privacy, None)),
    page("baa://settings/clearBrowserData", "Clear Browsing Data", "history cache cookies delete", S(Privacy, Some("clear_data"))),
    page("baa://settings/content", "Settings: Site Settings", "permissions", S(Sites, None)),
    page("baa://settings/appearance", "Settings: Appearance", "theme zoom fonts", S(Appearance, None)),
    page("baa://settings/accessibility", "Settings: Accessibility", "caret focus", S(Appearance, Some("focus_highlight"))),
    page("baa://settings/downloads", "Settings: Downloads", "folder", S(General, Some("download_dir"))),
    page("baa://settings/autofill", "Settings: Passwords and Autofill", "", S(Passwords, None)),
    page("baa://settings/system", "Settings: System", "performance", S(System, None)),
    page("baa://settings/reset", "Reset Settings", "defaults", S(System, Some("reset"))),
    page("baa://settings/defaultBrowser", "Default Browser", "", S(General, Some("default_browser"))),
    page("baa://settings/onStartup", "On Launch", "startup", S(General, Some("on_launch"))),
    page("baa://extensions", "Extensions", "add-ons developer", Route::Extensions),
    page("baa://history", "History", "visited", Route::History),
    page("baa://downloads", "Downloads", "files", Route::Downloads),
    page("baa://settings/languages", "Languages and Spell Check", "", WebUi),
    page("baa://settings/fonts", "Fonts", "", WebUi),
    page("baa://settings/content/all", "All Sites and Storage", "site data", WebUi),
    page("baa://settings/certificates", "Certificates", "", WebUi),
    page("baa://flags", "Chrome Flags", "experiments", WebUi),
    page("baa://version", "About baa", "version", WebUi),
    page("baa://credits", "Credits", "licenses", WebUi),
    page("baa://gpu", "GPU Internals", "", WebUi),
    page("baa://net-internals", "Net Internals", "", WebUi),
    page("baa://net-export", "Net Export", "", WebUi),
    page("baa://extensions-internals", "Extensions Internals", "", WebUi),
    page("baa://inspect", "Inspect Devices", "devtools", WebUi),
    page("baa://chrome-urls", "All Chrome Pages", "", WebUi),
    page("baa://components", "Components", "", WebUi),
    page("baa://policy", "Policies", "", WebUi),
    page("baa://prefs-internals", "Prefs Internals", "", WebUi),
    page("baa://discards", "Discards", "memory tabs", WebUi),
    page("baa://process-internals", "Process Internals", "", WebUi),
    page("baa://media-internals", "Media Internals", "", WebUi),
    page("baa://webrtc-internals", "WebRTC Internals", "", WebUi),
    page("baa://site-engagement", "Site Engagement", "", WebUi),
    page("baa://sandbox", "Sandbox", "", WebUi),
    page("baa://accessibility", "Accessibility Internals", "", WebUi),
    page("baa://crashes", "Crashes", "", WebUi),
    page("baa://management", "Management", "", WebUi),
    page("baa://certificate-manager", "Certificate Manager", "", WebUi),
    page("baa://bookmarks", "Bookmarks", "", WebUi),
];

/// The per-kind Site Settings pages, made from the one table of site kinds.
fn site_pages() -> &'static [Page] {
    static PAGES: std::sync::LazyLock<Vec<Page>> = std::sync::LazyLock::new(|| {
        crate::settings::rows::SITE_KINDS
            .iter()
            .map(|k| Page {
                url: Box::leak(format!("baa://settings/content/{}", k.path).into_boxed_str()),
                label: Box::leak(format!("Site Settings: {}", crate::settings::rows::label(k.row)).into_boxed_str()),
                keywords: "",
                route: Route::Settings(Tab::Sites, Some(k.row)),
            })
            .collect()
    });
    &PAGES
}

/// Every page: the table, then the site kinds.
pub(crate) fn all() -> impl Iterator<Item = &'static Page> {
    PAGES.iter().chain(site_pages())
}

/// `chrome://` and `baa://` are the same page; the query and fragment are not part of it.
pub(crate) fn normalize(url: &str) -> String {
    let url = url.trim();
    let url = match url.strip_prefix("chrome:") {
        Some(rest) => format!("baa:{rest}"),
        None => url.to_string(),
    };
    let end = url.find(['?', '#']).unwrap_or(url.len());
    url[..end].trim_end_matches('/').to_string()
}

/// A page baa draws itself. `None` for Chrome's WebUI and everything else: those load as tabs.
pub(crate) fn route(url: &str) -> Option<Route> {
    let url = normalize(url);
    if url == "baa://extensions" || url.starts_with("baa://extensions/") {
        return Some(Route::Extensions);
    }
    all().find(|p| p.url == url).map(|p| p.route).filter(|r| *r != WebUi)
}

/// `baa://extensions/?id=…`
pub(crate) fn extension_id(url: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("id="))
        .filter(|id| !id.is_empty())
        .map(String::from)
}

/// Whether the command bar should autofill: it starts like an address. A bare "baa" is a search.
pub(crate) fn looks_like_page(query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.starts_with("baa:") || q.starts_with("chrome:")
}

/// Pages whose address starts with the query: exact first, then shorter, at most `max`.
pub(crate) fn complete(query: &str, max: usize) -> Vec<&'static Page> {
    let q = query.trim().to_lowercase();
    let q = match q.strip_prefix("chrome:") {
        Some(rest) => format!("baa:{rest}"),
        None => q,
    };
    let mut hits: Vec<&Page> = all().filter(|p| p.url.to_lowercase().starts_with(&q)).collect();
    // Stable: equal lengths keep table order.
    hits.sort_by_key(|p| (p.url.to_lowercase() != q.trim_end_matches('/'), p.url.len()));
    hits.truncate(max);
    hits
}

impl<H: baa_web::WebHost> crate::Browser<H> {
    /// Opens `url` as baa's own page when it is one; false when it should load as a tab.
    pub(crate) fn open_own_page(&mut self, url: &str, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> bool {
        let Some(route) = route(url) else { return false };
        self.close_overlays(window, cx);
        match route {
            Route::Settings(tab, row) => self.open_settings_at(Some(tab), row, cx),
            Route::Extensions => {
                self.extensions_page.open = extension_id(url);
                self.open_settings_at(Some(Tab::Extensions), None, cx);
            }
            Route::History => self.open_bar(crate::command_bar::Mode::History, window, cx),
            Route::Downloads => self.open_library(crate::library::Section::Downloads, cx),
            Route::WebUi => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes() {
        assert_eq!(route("baa://settings/privacy"), Some(Route::Settings(Privacy, None)));
        assert_eq!(route("chrome://settings/content/popups"), Some(Route::Settings(Sites, Some("site.popups"))));
        assert_eq!(route("baa://extensions/?id=x"), Some(Route::Extensions));
        assert_eq!(route("chrome://extensions/shortcuts"), Some(Route::Extensions));
        assert_eq!(route("baa://history"), Some(Route::History));
        for webui in ["baa://flags", "baa://settings/fonts", "baa://settings/unknown", "https://example.com/extensions", "baa://nope"] {
            assert_eq!(route(webui), None, "{webui}");
        }
        assert_eq!(extension_id("baa://extensions/?id=abc&x=1").as_deref(), Some("abc"));
        assert_eq!(extension_id("baa://extensions"), None);
    }

    #[test]
    fn completion_puts_the_exact_page_then_the_shortest() {
        let urls = |q| complete(q, 6).into_iter().map(|p| p.url).collect::<Vec<_>>();
        assert_eq!(urls("baa://se")[0], "baa://settings");
        assert!(urls("baa://se").iter().all(|u| u.starts_with("baa://se")));
        assert_eq!(urls("chrome://extensions")[0], "baa://extensions");
        assert_eq!(urls("baa://settings/content")[0], "baa://settings/content");
        assert!(looks_like_page("baa:") && looks_like_page("baa:/") && looks_like_page("chrome://fl"));
        assert!(!looks_like_page("baa") && !looks_like_page("bank") && !looks_like_page("example.com"));
    }

    #[test]
    fn every_first_party_row_exists() {
        for p in all() {
            if let Route::Settings(tab, Some(row)) = p.route {
                assert_eq!(crate::settings::rows::find(row).map(|r| r.tab), Some(tab), "{}", p.url);
            }
        }
    }
}
