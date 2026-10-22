//! Runtime de-Googling of stock CEF Alloy (PLAN.md §5): switches, prefs, host blocklist, and
//! the allowlist the CI proxy test enforces.

use crate::ResourceType;

/// Appended in `OnBeforeCommandLineProcessing`; `name=value` entries carry a value. `disable-features` is not here because CEF or the
/// embedder may already have set it; use [`merge_disabled_features`].
pub const SWITCHES: &[&str] = &[
    // Covers variations seed fetch, component updater, translate ranking, GCM checkin and
    // other background fetches in one go.
    "disable-background-networking",
    "disable-component-update",
    // CEF registers components regardless of the switch above (seen in the CEF log), so the
    // update check is pointed at a dead local port instead. 127.0.0.1 is on PROXY_ALLOWLIST.
    "component-updater=url-source=http://127.0.0.1:9/,disable-pings",
    "disable-field-trial-config",
    "disable-domain-reliability",
    "disable-breakpad",
    "disable-client-side-phishing-detection",
    "disable-default-apps",
    "disable-sync",
    "no-default-browser-check",
    "no-first-run",
    // <a ping> hyperlink auditing.
    "no-pings",
];

// Unknown names are ignored by Chromium, so a feature renamed in a later CEF only stops working;
// the CI proxy test is what catches that.
const DISABLED_FEATURES: &[&str] = &[
    "MediaRouter",
    "DialMediaRouteProvider",
    "CastMediaRouteProvider",
    "NetworkTimeServiceQuerying",
    "OptimizationHints",
    "OptimizationHintsFetching",
    "OptimizationGuideModelDownloading",
    "AutofillServerCommunication",
    "BrowsingTopics",
    "Fledge",
    "InterestGroupStorage",
];

/// Value for `--disable-features`: the current value (empty if unset) plus ours, deduped.
pub fn merge_disabled_features(existing: &str) -> String {
    let mut out: Vec<&str> = existing.split(',').filter(|f| !f.is_empty()).collect();
    for f in DISABLED_FEATURES {
        if !out.contains(f) {
            out.push(f);
        }
    }
    out.join(",")
}

/// Boolean prefs set to `false` on every request context with `SetPreference`. Alloy registers
/// only part of Chrome's prefs, so a failed set means the feature is absent there; log it and
/// move on.
pub const PREFS_OFF: &[&str] = &[
    "alternate_error_pages.enabled",
    "enable_a_ping",
    "safebrowsing.enabled",
    "search.suggest_enabled",
    "spellcheck.use_spelling_service",
    "translate.enabled",
    "url_keyed_anonymized_data_collection.enabled",
    "privacy_sandbox.m1.topics_enabled",
    "privacy_sandbox.m1.fledge_enabled",
    "privacy_sandbox.m1.ad_measurement_enabled",
];

// gvt1/gvt2: component and connectivity-check CDNs (redirector.gvt1.com, *.gvt2.com).
const BROWSER_BLOCKED_DOMAINS: &[&str] =
    &["google.com", "googleapis.com", "gstatic.com", "doubleclick.net", "gvt1.com", "gvt2.com"];

/// Host blocklist for requests Chromium makes on its own (§5 mechanism 2). `is_page_initiated`:
/// a frame or initiator origin asked for it. Page content is never blocked here, and neither
/// are top-level navigations (the user typed google.com) or favicons of the page being shown.
pub fn block_browser_request(url: &str, resource_type: ResourceType, is_page_initiated: bool) -> bool {
    use ResourceType::*;
    if is_page_initiated
        || matches!(resource_type, MainFrame | NavigationPreloadMainFrame | Favicon)
    {
        return false;
    }
    let Some(parsed) = url::Url::parse(url).ok() else {
        return false;
    };
    let Some(host) = parsed.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    let host = host.trim_end_matches('.');
    // Chrome Web Store extension installs and updates. The CRX itself then comes from
    // clients2.googleusercontent.com, which is not on the list.
    if host == "clients2.google.com" && parsed.path() == "/service/update2/crx" {
        return false;
    }
    // Search suggestions in the command bar, for a Google default search engine.
    if host == "suggestqueries.google.com" && parsed.path() == "/complete/search" {
        return false;
    }
    BROWSER_BLOCKED_DOMAINS
        .iter()
        .any(|d| host == *d || host.strip_suffix(d).is_some_and(|p| p.ends_with('.')))
}

/// Hosts the §5.3 CI proxy test tolerates; anything else contacted fails the run.
/// `cargo run -p baa-privacy --example proxy_allowlist` prints one per line.
pub const PROXY_ALLOWLIST: &[&str] = &[
    "localhost",
    "127.0.0.1",
    "::1",
];

#[cfg(test)]
mod tests {
    use super::*;
    use ResourceType::*;

    #[test]
    fn blocks_browser_initiated_google() {
        for url in [
            "https://update.googleapis.com/service/update2",
            "https://clients2.google.com/time/1/current",
            "https://www.gstatic.com/generate_204",
            "https://redirector.gvt1.com/edgedl/x",
            "https://GOOGLE.com./x",
            "https://stats.g.doubleclick.net/x",
        ] {
            assert!(block_browser_request(url, SubResource, false), "{url}");
        }
    }

    #[test]
    fn allows_extension_downloads() {
        assert!(!block_browser_request(
            "https://suggestqueries.google.com/complete/search?client=firefox&q=a",
            SubResource,
            false
        ));
        let crx = "https://clients2.google.com/service/update2/crx?response=redirect&x=id%3Dabc";
        assert!(!block_browser_request(crx, SubResource, false));
        assert!(block_browser_request("https://clients2.google.com/service/update2/json", SubResource, false));
    }

    #[test]
    fn never_blocks_page_content_or_navigation() {
        assert!(!block_browser_request("https://fonts.gstatic.com/a.woff2", Font, true));
        assert!(!block_browser_request("https://www.google.com/", MainFrame, false));
        assert!(!block_browser_request("https://www.google.com/favicon.ico", Favicon, false));
    }

    #[test]
    fn suffix_match_is_on_label_boundary() {
        assert!(!block_browser_request("https://notgoogle.com/x", SubResource, false));
        assert!(!block_browser_request("https://google.com.evil.example/x", SubResource, false));
        assert!(!block_browser_request("not a url", SubResource, false));
    }

    #[test]
    fn merges_features_without_duplicates() {
        let v = merge_disabled_features("Foo,MediaRouter");
        assert!(v.starts_with("Foo,MediaRouter,"));
        assert_eq!(v.split(',').filter(|f| *f == "MediaRouter").count(), 1);
        assert_eq!(merge_disabled_features("").split(',').count(), DISABLED_FEATURES.len());
    }
}
