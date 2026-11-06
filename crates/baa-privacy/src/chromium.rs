//! Switches, prefs and the allowlist for baa's own Chromium (PLAN.md §5).

/// What the own Chromium build (BaaCore) still needs on its command line; the rest is
/// compiled out or patched in the tree (PLAN-chromium-phase4.md, step 7).
pub const CORE_SWITCHES: &[&str] = &["disable-background-networking", "no-pings"];

/// Features the own build still turns off, in `--disable-features`.
pub const CORE_DISABLED_FEATURES: &[&str] = &["MediaRouter", "OptimizationHints", "AutofillServerCommunication", "AimEnabled"];

/// Boolean prefs set to `false` on every profile.
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

/// Hosts the §5.3 CI proxy test tolerates; anything else contacted fails the run.
/// `cargo run -p baa-privacy --example proxy_allowlist` prints one per line.
pub const PROXY_ALLOWLIST: &[&str] = &[
    "localhost",
    "127.0.0.1",
    "::1",
];
