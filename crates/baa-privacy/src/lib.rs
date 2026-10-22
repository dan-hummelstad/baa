//! De-Googling (PLAN.md §5). No CEF or gpui here: `baa-web` maps CEF's request types onto
//! [`ResourceType`] and calls in.

mod chromium;

pub use chromium::{block_browser_request, merge_disabled_features, PREFS_OFF, PROXY_ALLOWLIST, SWITCHES};

/// `cef_resource_type_t`, one variant each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceType {
    MainFrame,
    SubFrame,
    Stylesheet,
    Script,
    Image,
    Font,
    SubResource,
    Object,
    Media,
    Worker,
    SharedWorker,
    Prefetch,
    Favicon,
    Xhr,
    Ping,
    ServiceWorker,
    CspReport,
    PluginResource,
    NavigationPreloadMainFrame,
    NavigationPreloadSubFrame,
}
