/// Process-wide browser runtime settings.
#[derive(Clone, Debug)]
pub struct RuntimeOptions {
    /// `--remote-debugging-port`. When set, Chrome DevTools can attach at
    /// `http://127.0.0.1:<port>`. macOS (CEF) only.
    pub remote_debugging_port: Option<u16>,
    /// `CefSettings.root_cache_path`: every [`crate::Profile`]'s `cache_path`
    /// must sit below it. The global context itself stays in memory. `None`
    /// keeps everything in memory.
    pub root_cache_path: Option<std::path::PathBuf>,
    /// Extra Chromium switches, `(name, value)`, appended to the browser
    /// process command line. macOS (CEF) only.
    pub switches: Vec<(String, Option<String>)>,
    pub log_to_stderr: bool,
    /// Where CEF writes its log file. Chromium CHECK failures land here, which
    /// makes this the first place to look when the process dies without a
    /// Rust-side panic.
    pub log_file: Option<std::path::PathBuf>,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            remote_debugging_port: None,
            root_cache_path: None,
            switches: Vec::new(),
            log_to_stderr: cfg!(debug_assertions),
            log_file: None,
        }
    }
}

/// Settings for a single webview.
#[derive(Clone, Debug)]
pub struct WebviewOptions {
    /// The URL to open first.
    pub url: String,
    /// Make the page background transparent, so gpui shows through.
    pub transparent: bool,
    /// Rounds the page's corners, in logical pixels, to match the element it
    /// sits in. macOS (CEF) only: there the page is a native view that gpui's
    /// own clipping cannot reach.
    pub corner_radius: f32,
    /// The storage profile (cookies, cache, local storage) to load in. `None`
    /// is the global, in-memory one.
    pub profile: Option<crate::Profile>,
}

impl Default for WebviewOptions {
    fn default() -> Self {
        Self {
            url: "about:blank".into(),
            transparent: false,
            corner_radius: 0.,
            profile: None,
        }
    }
}

impl WebviewOptions {
    pub fn url(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Default::default()
        }
    }
}

/// A cookie to put in the browser's store, for [`crate::Runtime::set_cookies`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookieSpec {
    /// A URL the cookie belongs to, such as `https://github.com/`; the store
    /// checks the cookie against it.
    pub url: String,
    pub name: String,
    pub value: String,
    /// The domain, with a leading dot for a domain cookie; empty for a
    /// host-only cookie.
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Seconds since the Unix epoch; `None` for a session cookie.
    pub expires: Option<i64>,
    pub same_site: SameSite,
}

/// A cookie's `SameSite` policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SameSite {
    #[default]
    Unspecified,
    None,
    Lax,
    Strict,
}

/// One network request, as [`ProfileOptions::filter`] sees it.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub url: &'a str,
    /// The requesting frame's URL; the URL itself for a main-frame load. Empty
    /// when no page is involved.
    pub source_url: &'a str,
    pub resource_type: baa_privacy::ResourceType,
    /// A frame or an initiator origin asked for it, rather than the browser.
    pub page_initiated: bool,
}

/// `true` cancels the request. Called on CEF's IO thread.
pub type FilterHook = std::sync::Arc<dyn Fn(&Request) -> bool + Send + Sync>;

/// Settings for one storage profile, see [`crate::Profile::new`].
#[derive(Clone)]
pub struct ProfileOptions {
    /// Below [`RuntimeOptions::root_cache_path`]. `None` keeps the profile in
    /// memory (incognito).
    pub cache_path: Option<std::path::PathBuf>,
    pub filter: FilterHook,
    /// Boolean Chromium prefs set to false once the context is up. One this
    /// build does not register is logged and skipped.
    pub prefs_off: &'static [&'static str],
}
