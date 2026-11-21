//! Switches, prefs and the allowlist for baa's own Chromium (PLAN.md §5).

/// What the own Chromium build (BaaCore) still needs on its command line; the rest is
/// compiled out or patched in the tree (PLAN-chromium-phase4.md, step 7).
pub const CORE_SWITCHES: &[&str] = &["disable-background-networking", "no-pings"];

/// Features the own build still turns off, in `--disable-features`. `HttpsFirstDialogUi` puts
/// the warning before an http-only page in a dialog on Chrome's browser window, which baa does
/// not have: Chromium aborts (`CHECK(host)` in `TabDialogManager`, 0.1.8 on 2026-11-21). Off,
/// the same warning is a page in the tab.
pub const CORE_DISABLED_FEATURES: &[&str] = &["MediaRouter", "OptimizationHints", "AutofillServerCommunication", "AimEnabled", "HttpsFirstDialogUi"];

/// Features the own build turns on, in `--enable-features`. Auto Picture in Picture for any
/// playing video, not only pages that register the media session's `enterpictureinpicture`
/// (as Arc does); the AUTO_PICTURE_IN_PICTURE content setting still gates it.
pub const CORE_ENABLED_FEATURES: &[&str] = &["BrowserInitiatedAutomaticPictureInPicture"];

/// Boolean prefs set to `false` on every profile.
pub const PREFS_OFF: &[&str] = &[
    "alternate_error_pages.enabled",
    "enable_a_ping",
    "safebrowsing.enabled",
    "spellcheck.use_spelling_service",
    "translate.enabled",
    "url_keyed_anonymized_data_collection.enabled",
    "privacy_sandbox.m1.topics_enabled",
    "privacy_sandbox.m1.fledge_enabled",
    "privacy_sandbox.m1.ad_measurement_enabled",
];

/// Hosts the §5.3 CI proxy test tolerates; anything else contacted fails the run.
/// `cargo run -p baa-privacy --example proxy_allowlist` prints one per line.
pub const PROXY_ALLOWLIST: &[&str] = &[
    "localhost",
    "127.0.0.1",
    "::1",
    // Extension update checks (remove-helium-services.patch); Settings › Privacy turns them off.
    "clients2.google.com",
    // CRLSet, and Widevine once opted in (widevine-opt-in.patch); each has a setting.
    "update.googleapis.com",
];
