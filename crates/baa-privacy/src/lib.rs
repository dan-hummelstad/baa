//! De-Googling (PLAN.md §5): what baa's own Chromium build still needs on its command line,
//! the prefs it pins off, and the allowlist the CI proxy test enforces. Helium's patches do
//! the rest. No gpui here.

mod chromium;

pub use chromium::{CORE_DISABLED_FEATURES, CORE_SWITCHES, PREFS_OFF, PROXY_ALLOWLIST};
