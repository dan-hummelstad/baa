//! BaaCore's C ABI (`chromium/baa/baa.h`), loaded with dlopen like Chrome's
//! own launcher does, so cargo never links against the Chromium build.

use std::ffi::{c_char, c_int, c_void, CStr, CString};

pub const ABI_VERSION: c_int = 7;

#[repr(C)]
pub struct Profile {
    _private: [u8; 0],
}

/// Never reused within a run; 0 is "none".
pub type TabId = u64;

#[repr(C)]
pub struct TabState {
    pub url: *const c_char,
    pub title: *const c_char,
    pub favicon_url: *const c_char,
    pub view: *mut c_void,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub audible: bool,
    pub secure: bool,
    pub crashed: bool,
}

/// One row of a context menu, depth-first (`BaaMenuItem`).
#[repr(C)]
pub struct MenuItem {
    pub label: *const c_char,
    pub command: c_int,
    pub depth: u8,
    pub kind: u8,
    pub enabled: bool,
    pub checked: bool,
}

// The BAA_* enums of baa.h, one constant each (prefix dropped); the test at
// the bottom checks them against the header itself.
#[allow(dead_code)] // the rest of the enum are mirrored too, for header_matches
pub const MENU_COMMAND: u8 = 0;
pub const MENU_SEPARATOR: u8 = 1;
pub const MENU_SUBMENU: u8 = 2;
pub const MENU_INSPECT: u8 = 3;

pub const PERMISSION_CAMERA: u32 = 1 << 0;
pub const PERMISSION_MICROPHONE: u32 = 1 << 1;
pub const PERMISSION_LOCATION: u32 = 1 << 2;
pub const PERMISSION_NOTIFICATIONS: u32 = 1 << 3;
pub const PERMISSION_CLIPBOARD: u32 = 1 << 4;
pub const PERMISSION_MIDI: u32 = 1 << 5;
pub const PERMISSION_OTHER: u32 = 1 << 6;
pub const PERMISSION_POPUPS: u32 = 1 << 7;
pub const PERMISSION_SOUND: u32 = 1 << 8;
pub const PERMISSION_JAVASCRIPT: u32 = 1 << 9;
pub const PERMISSION_COOKIES: u32 = 1 << 10;
pub const PERMISSION_AUTOMATIC_DOWNLOADS: u32 = 1 << 11;
pub const PERMISSION_INSECURE_CONTENT: u32 = 1 << 12;
pub const PERMISSION_SENSORS: u32 = 1 << 13;
pub const PERMISSION_WINDOW_MANAGEMENT: u32 = 1 << 14;

pub const SITE_UNSET: c_int = -1;
pub const SITE_ASK: c_int = 0;
pub const SITE_ALLOW: c_int = 1;
pub const SITE_BLOCK: c_int = 2;
pub const SITE_SESSION_ONLY: c_int = 3;

pub const CLEAR_HISTORY: u32 = 1 << 0;
pub const CLEAR_DOWNLOADS: u32 = 1 << 1;
pub const CLEAR_SITE_DATA: u32 = 1 << 2;
pub const CLEAR_CACHE: u32 = 1 << 3;

#[allow(dead_code)]
pub const SITE_ACCESS_NONE: c_int = 0;
pub const SITE_ACCESS_ON_CLICK: c_int = 1;
pub const SITE_ACCESS_ON_SPECIFIC_SITES: c_int = 2;
pub const SITE_ACCESS_ON_ALL_SITES: c_int = 3;

pub const EXTENSION_ENABLE: c_int = 0;
pub const EXTENSION_REMOVE: c_int = 1;
pub const EXTENSION_SITE_ACCESS: c_int = 2;
pub const EXTENSION_RELOAD: c_int = 3;

#[allow(dead_code)]
pub const DIALOG_ALERT: c_int = 0;
pub const DIALOG_CONFIRM: c_int = 1;
pub const DIALOG_PROMPT: c_int = 2;
pub const DIALOG_BEFOREUNLOAD: c_int = 3;
pub const DIALOG_BEFOREUNLOAD_RELOAD: c_int = 4;
pub const DIALOG_AUTH: c_int = 5;
pub const DIALOG_EXTERNAL_PROTOCOL: c_int = 6;
pub const DIALOG_UNRESPONSIVE: c_int = 7;
pub const DIALOG_EXTENSION_INSTALL: c_int = 8;

#[allow(dead_code)]
pub const CLOSED_BY_CHROME: c_int = 0;
pub const CLOSED_BY_QUIT: c_int = 1;
pub const CLOSED_BY_APP: c_int = 2;

pub const DOCK_RIGHT: c_int = 0;
pub const DOCK_BOTTOM: c_int = 1;
pub const DOCK_LEFT: c_int = 2;
pub const DOCK_UNDOCKED: c_int = 3;

pub const DOWNLOAD_IN_PROGRESS: c_int = 0;
pub const DOWNLOAD_COMPLETE: c_int = 1;
#[allow(dead_code)]
pub const DOWNLOAD_CANCELED: c_int = 2;

#[repr(C)]
pub struct Download {
    pub id: u64,
    pub url: *const c_char,
    pub path: *const c_char,
    pub received_bytes: i64,
    pub total_bytes: i64,
    pub state: c_int,
}

/// `BaaDialog`: a question the page or Chrome waits on.
#[repr(C)]
pub struct Dialog {
    pub kind: c_int,
    pub message: *const c_char,
    pub default_text: *const c_char,
    pub title: *const c_char,
    pub ok_label: *const c_char,
    pub cancel_label: *const c_char,
    pub check_label: *const c_char,
}

/// `BaaAnswer`.
#[repr(C)]
pub struct Answer {
    pub ok: bool,
    pub text: *const c_char,
    pub password: *const c_char,
    pub checked: bool,
}

/// `BaaMatch`: one omnibox match.
#[repr(C)]
pub struct Match {
    pub url: *const c_char,
    pub contents: *const c_char,
    pub description: *const c_char,
    pub search: bool,
}

#[repr(C)]
pub struct Extension {
    pub id: *const c_char,
    pub name: *const c_char,
    pub description: *const c_char,
    pub version: *const c_char,
    pub icon_path: *const c_char,
    pub popup_url: *const c_char,
    pub options_url: *const c_char,
    pub permissions: *const c_char,
    pub site_access: c_int,
    pub enabled: bool,
    pub unpacked: bool,
}

pub type ExtensionVisitor = extern "C" fn(ctx: *mut c_void, extension: *const Extension);

pub type SiteRuleVisitor = extern "C" fn(ctx: *mut c_void, pattern: *const c_char, state: c_int);

pub type HistoryVisitor = extern "C" fn(ctx: *mut c_void, title: *const c_char, url: *const c_char);

#[repr(C)]
pub struct Callbacks {
    pub key: extern "C" fn(ctx: *mut c_void, ns_event: *mut c_void, after_page: bool) -> bool,
    pub tab_created: extern "C" fn(
        ctx: *mut c_void,
        tab: TabId,
        opener: TabId,
        profile: *mut Profile,
        url: *const c_char,
        background: bool,
        user_gesture: bool,
    ),
    pub tab_state: extern "C" fn(ctx: *mut c_void, tab: TabId, state: *const TabState),
    pub tab_favicon:
        extern "C" fn(ctx: *mut c_void, tab: TabId, icon_url: *const c_char, png: *const u8, png_len: usize),
    pub dialog: extern "C" fn(ctx: *mut c_void, tab: TabId, id: u64, dialog: *const Dialog),
    pub tab_closed: extern "C" fn(ctx: *mut c_void, tab: TabId, reason: c_int),
    pub permission: extern "C" fn(ctx: *mut c_void, tab: TabId, id: u64, origin: *const c_char, kinds: u32),
    pub request_closed: extern "C" fn(ctx: *mut c_void, tab: TabId, id: u64),
    pub context_menu: extern "C" fn(
        ctx: *mut c_void,
        tab: TabId,
        id: u64,
        x: f32,
        y: f32,
        items: *const MenuItem,
        count: usize,
    ),
    pub download: extern "C" fn(ctx: *mut c_void, tab: TabId, download: *const Download),
    pub resized: extern "C" fn(ctx: *mut c_void, tab: TabId, width: c_int, height: c_int),
    pub devtools_dock: extern "C" fn(ctx: *mut c_void, front: TabId, side: c_int),
    pub fullscreen: extern "C" fn(ctx: *mut c_void, tab: TabId, on: bool),
    pub find_result: extern "C" fn(ctx: *mut c_void, tab: TabId, active: c_int, count: c_int),
    pub tab_group: extern "C" fn(ctx: *mut c_void, tab: TabId, group: u64, title: *const c_char),
    pub autocomplete: extern "C" fn(ctx: *mut c_void, id: u64, matches: *const Match, count: usize, done: bool),
    pub clear_done: extern "C" fn(ctx: *mut c_void, id: u64),
    pub load_unpacked_done: extern "C" fn(ctx: *mut c_void, id: u64, ok: bool, message: *const c_char),
}

/// Every function of `baa.h`. Plain data, so tests build one from fakes.
pub struct Core {
    pub install_app: extern "C" fn(c_int, *const *const c_char, *const Callbacks, *mut c_void),
    pub exit_code: extern "C" fn() -> c_int,
    /// ⌘Q is closing every tab: a leave-site question would wait on nobody.
    pub is_quitting: extern "C" fn() -> bool,
    pub profile_open: extern "C" fn(*const c_char) -> *mut Profile,
    pub profile_open_ephemeral: extern "C" fn() -> *mut Profile,
    pub profile_close: extern "C" fn(*mut Profile),
    /// Null profile: Local State.
    pub pref_set_bool: extern "C" fn(*mut Profile, *const c_char, bool) -> bool,
    pub pref_set_int: extern "C" fn(*mut Profile, *const c_char, c_int) -> bool,
    pub pref_set_string: extern "C" fn(*mut Profile, *const c_char, *const c_char) -> bool,
    /// Back to Chrome's value.
    pub pref_clear: extern "C" fn(*mut Profile, *const c_char) -> bool,
    pub is_default_browser: extern "C" fn() -> bool,
    pub set_default_browser: extern "C" fn(),
    /// prefers-color-scheme (0 system, 1 light, 2 dark) of the profile's websites and of its Chrome pages.
    pub profile_set_color_schemes: extern "C" fn(*mut Profile, c_int, c_int),
    pub profile_set_default_zoom: extern "C" fn(*mut Profile, f64),
    pub profile_set_search_engine: extern "C" fn(*mut Profile, *const c_char, *const c_char, *const c_char) -> bool,
    pub profile_site_permission: extern "C" fn(*mut Profile, *const c_char, u32) -> c_int,
    pub profile_set_site_permission: extern "C" fn(*mut Profile, *const c_char, u32, c_int),
    pub profile_site_rules: extern "C" fn(*mut Profile, u32, SiteRuleVisitor, *mut c_void) -> c_int,
    pub profile_set_site_rule: extern "C" fn(*mut Profile, u32, *const c_char, c_int) -> bool,
    pub profile_clear_data: extern "C" fn(*mut Profile, u64, i64, u32),
    #[allow(dead_code)] // WP-R2
    pub profile_autocomplete: extern "C" fn(*mut Profile, u64, *const c_char),
    pub profile_extensions: extern "C" fn(*mut Profile, ExtensionVisitor, *mut c_void) -> c_int,
    pub profile_extension_change: extern "C" fn(*mut Profile, *const c_char, c_int, c_int) -> bool,
    pub profile_load_unpacked: extern "C" fn(*mut Profile, u64, *const c_char),
    pub extension_popup: extern "C" fn(*mut Profile, *const c_char) -> TabId,
    pub tab_create: extern "C" fn(*mut Profile, *const c_char) -> TabId,
    pub tab_close: extern "C" fn(TabId),
    pub tab_get_state: extern "C" fn(TabId, *mut TabState) -> bool,
    pub tab_load_url: extern "C" fn(TabId, *const c_char),
    pub tab_reload: extern "C" fn(TabId, bool),
    pub tab_stop: extern "C" fn(TabId),
    pub tab_go_to_offset: extern "C" fn(TabId, c_int),
    pub tab_history: extern "C" fn(TabId, HistoryVisitor, *mut c_void) -> c_int,
    pub tab_eval: extern "C" fn(TabId, *const c_char, bool),
    pub tab_set_zoom: extern "C" fn(TabId, f64),
    pub tab_find: extern "C" fn(TabId, *const c_char, bool),
    pub tab_print: extern "C" fn(TabId),
    pub tab_set_muted: extern "C" fn(TabId, bool),
    pub tab_activate: extern "C" fn(TabId),
    pub tab_focus: extern "C" fn(TabId),
    pub tab_set_init_script: extern "C" fn(TabId, *const c_char),
    pub tab_open_devtools: extern "C" fn(TabId, bool, c_int, c_int, c_int) -> TabId,
    pub tab_show_devtools_window: extern "C" fn(TabId, bool, c_int, c_int),
    pub tab_exit_fullscreen: extern "C" fn(TabId),
    pub tab_set_group: extern "C" fn(TabId, u64, *const c_char),
    /// Answers a dialog or permission ask.
    pub answer: extern "C" fn(u64, *const Answer),
    pub context_menu_done: extern "C" fn(u64, c_int),
    pub download_cancel: extern "C" fn(u64),
}

impl Core {
    /// An answer that is only yes or no.
    pub fn answer_ok(&self, id: u64, ok: bool) {
        (self.answer)(id, &Answer { ok, text: c"".as_ptr(), password: c"".as_ptr(), checked: false });
    }
}

/// # Safety
/// `F` is the `extern "C" fn` type of the symbol `ptr`.
unsafe fn fn_ptr<F: Copy>(ptr: *mut c_void) -> F {
    debug_assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<*mut c_void>());
    unsafe { std::mem::transmute_copy(&ptr) }
}

/// Loads `../Frameworks/BaaCore.framework` next to the executable. Call it
/// first in `main`: PartitionAlloc installs its malloc zone when the framework
/// loads, which is only safe while the process has one thread.
pub fn load() -> &'static Core {
    let exe = std::env::current_exe().expect("executable path");
    let path = exe.parent().expect("executable directory").join("../Frameworks/BaaCore.framework/BaaCore");
    let c_path = CString::new(path.to_str().expect("UTF-8 path")).unwrap();
    unsafe {
        let handle = libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if handle.is_null() {
            let error = CStr::from_ptr(libc::dlerror()).to_string_lossy();
            panic!("dlopen {}: {error}", path.display());
        }
        let sym = |name: &CStr| -> *mut c_void {
            let f = libc::dlsym(handle, name.as_ptr());
            assert!(!f.is_null(), "BaaCore lacks {name:?}");
            f
        };
        let version: extern "C" fn() -> c_int = fn_ptr(sym(c"baa_abi_version"));
        assert_eq!(version(), ABI_VERSION, "BaaCore ABI mismatch");
        macro_rules! core {
            ($($field:ident),* $(,)?) => {
                Core { $($field: fn_ptr(sym(
                    CStr::from_bytes_with_nul(concat!("baa_", stringify!($field), "\0").as_bytes()).unwrap()
                ))),* }
            };
        }
        Box::leak(Box::new(core!(
            install_app,
            exit_code,
            is_quitting,
            profile_open,
            profile_open_ephemeral,
            profile_close,
            pref_set_bool,
            pref_set_int,
            pref_set_string,
            pref_clear,
            is_default_browser,
            set_default_browser,
            profile_set_color_schemes,
            profile_set_default_zoom,
            profile_set_search_engine,
            profile_site_permission,
            profile_set_site_permission,
            profile_site_rules,
            profile_set_site_rule,
            profile_clear_data,
            profile_autocomplete,
            profile_extensions,
            profile_extension_change,
            profile_load_unpacked,
            extension_popup,
            tab_create,
            tab_close,
            tab_get_state,
            tab_load_url,
            tab_reload,
            tab_stop,
            tab_go_to_offset,
            tab_history,
            tab_eval,
            tab_set_zoom,
            tab_find,
            tab_print,
            tab_set_muted,
            tab_activate,
            tab_focus,
            tab_set_init_script,
            tab_open_devtools,
            tab_show_devtools_window,
            tab_exit_fullscreen,
            tab_set_group,
            answer,
            context_menu_done,
            download_cancel,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_comments(header: &str) -> String {
        let mut out = String::new();
        let mut rest = header;
        while let Some(at) = rest.find("/*") {
            out.push_str(&rest[..at]);
            let end = rest[at..].find("*/").expect("unterminated /* in baa.h");
            rest = &rest[at + end + 2..];
        }
        out.push_str(rest);
        out.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n")
    }

    /// One number: decimal, hex or `a << b`, optionally negative.
    fn term(text: &str) -> Option<i64> {
        let text = text.trim();
        if let Some((a, b)) = text.split_once("<<") {
            return Some(term(a)? << term(b)?);
        }
        let (negative, digits) = text.strip_prefix('-').map_or((false, text), |d| (true, d.trim()));
        let value = match digits.strip_prefix("0x") {
            Some(hex) => i64::from_str_radix(hex, 16).ok()?,
            None => digits.parse().ok()?,
        };
        Some(if negative { -value } else { value })
    }

    /// Every constant of every `enum { ... }` of the header with its value.
    /// One whose value is implicit or not a plain expression fails the test: a
    /// value this parser cannot read is a value nobody checks.
    fn header_constants(header: &str) -> Vec<(String, i64)> {
        let code = strip_comments(header);
        let mut out = Vec::new();
        let mut rest = code.as_str();
        while let Some(at) = rest.find("enum") {
            let after = &rest[at + 4..];
            let Some(open) = after.find('{') else { break };
            let close = after[open..].find('}').expect("unterminated enum in baa.h") + open;
            for entry in after[open + 1..close].split(',').map(str::trim).filter(|e| !e.is_empty()) {
                let (name, value) = entry.split_once('=').unwrap_or_else(|| panic!("`{entry}` has an implicit value"));
                let name = name.trim();
                assert!(name.starts_with("BAA_"), "`{name}` is not a BAA_ constant");
                // `A | B` ORs terms.
                let value = value
                    .split('|')
                    .map(|t| term(t).unwrap_or_else(|| panic!("{name}: cannot read `{}`", value.trim())))
                    .fold(0, |a, b| a | b);
                out.push((name.to_string(), value));
            }
            rest = &after[close + 1..];
        }
        out
    }

    #[test]
    fn header_matches() {
        let header = include_str!("../../../../chromium/baa/baa.h");
        let version: c_int = header
            .lines()
            .find_map(|l| l.strip_prefix("#define BAA_ABI_VERSION "))
            .and_then(|v| v.trim().parse().ok())
            .expect("BAA_ABI_VERSION in baa.h");
        assert_eq!(version, ABI_VERSION);

        let ours: &[(&str, i64)] = &[
            ("BAA_MENU_COMMAND", MENU_COMMAND as i64),
            ("BAA_MENU_SEPARATOR", MENU_SEPARATOR as i64),
            ("BAA_MENU_SUBMENU", MENU_SUBMENU as i64),
            ("BAA_MENU_INSPECT", MENU_INSPECT as i64),
            ("BAA_PERMISSION_CAMERA", PERMISSION_CAMERA as i64),
            ("BAA_PERMISSION_MICROPHONE", PERMISSION_MICROPHONE as i64),
            ("BAA_PERMISSION_LOCATION", PERMISSION_LOCATION as i64),
            ("BAA_PERMISSION_NOTIFICATIONS", PERMISSION_NOTIFICATIONS as i64),
            ("BAA_PERMISSION_CLIPBOARD", PERMISSION_CLIPBOARD as i64),
            ("BAA_PERMISSION_MIDI", PERMISSION_MIDI as i64),
            ("BAA_PERMISSION_OTHER", PERMISSION_OTHER as i64),
            ("BAA_PERMISSION_POPUPS", PERMISSION_POPUPS as i64),
            ("BAA_PERMISSION_SOUND", PERMISSION_SOUND as i64),
            ("BAA_PERMISSION_JAVASCRIPT", PERMISSION_JAVASCRIPT as i64),
            ("BAA_PERMISSION_COOKIES", PERMISSION_COOKIES as i64),
            ("BAA_PERMISSION_AUTOMATIC_DOWNLOADS", PERMISSION_AUTOMATIC_DOWNLOADS as i64),
            ("BAA_PERMISSION_INSECURE_CONTENT", PERMISSION_INSECURE_CONTENT as i64),
            ("BAA_PERMISSION_SENSORS", PERMISSION_SENSORS as i64),
            ("BAA_PERMISSION_WINDOW_MANAGEMENT", PERMISSION_WINDOW_MANAGEMENT as i64),
            ("BAA_SITE_UNSET", SITE_UNSET as i64),
            ("BAA_SITE_SESSION_ONLY", SITE_SESSION_ONLY as i64),
            ("BAA_CLEAR_HISTORY", CLEAR_HISTORY as i64),
            ("BAA_CLEAR_DOWNLOADS", CLEAR_DOWNLOADS as i64),
            ("BAA_CLEAR_SITE_DATA", CLEAR_SITE_DATA as i64),
            ("BAA_CLEAR_CACHE", CLEAR_CACHE as i64),
            ("BAA_EXTENSION_RELOAD", EXTENSION_RELOAD as i64),
            ("BAA_SITE_ASK", SITE_ASK as i64),
            ("BAA_SITE_ALLOW", SITE_ALLOW as i64),
            ("BAA_SITE_BLOCK", SITE_BLOCK as i64),
            ("BAA_SITE_ACCESS_NONE", SITE_ACCESS_NONE as i64),
            ("BAA_SITE_ACCESS_ON_CLICK", SITE_ACCESS_ON_CLICK as i64),
            ("BAA_SITE_ACCESS_ON_SPECIFIC_SITES", SITE_ACCESS_ON_SPECIFIC_SITES as i64),
            ("BAA_SITE_ACCESS_ON_ALL_SITES", SITE_ACCESS_ON_ALL_SITES as i64),
            ("BAA_EXTENSION_ENABLE", EXTENSION_ENABLE as i64),
            ("BAA_EXTENSION_REMOVE", EXTENSION_REMOVE as i64),
            ("BAA_EXTENSION_SITE_ACCESS", EXTENSION_SITE_ACCESS as i64),
            ("BAA_DIALOG_ALERT", DIALOG_ALERT as i64),
            ("BAA_DIALOG_CONFIRM", DIALOG_CONFIRM as i64),
            ("BAA_DIALOG_PROMPT", DIALOG_PROMPT as i64),
            ("BAA_DIALOG_BEFOREUNLOAD", DIALOG_BEFOREUNLOAD as i64),
            ("BAA_DIALOG_BEFOREUNLOAD_RELOAD", DIALOG_BEFOREUNLOAD_RELOAD as i64),
            ("BAA_DIALOG_AUTH", DIALOG_AUTH as i64),
            ("BAA_DIALOG_EXTERNAL_PROTOCOL", DIALOG_EXTERNAL_PROTOCOL as i64),
            ("BAA_DIALOG_UNRESPONSIVE", DIALOG_UNRESPONSIVE as i64),
            ("BAA_DIALOG_EXTENSION_INSTALL", DIALOG_EXTENSION_INSTALL as i64),
            ("BAA_CLOSED_BY_CHROME", CLOSED_BY_CHROME as i64),
            ("BAA_CLOSED_BY_QUIT", CLOSED_BY_QUIT as i64),
            ("BAA_CLOSED_BY_APP", CLOSED_BY_APP as i64),
            ("BAA_DOCK_RIGHT", DOCK_RIGHT as i64),
            ("BAA_DOCK_BOTTOM", DOCK_BOTTOM as i64),
            ("BAA_DOCK_LEFT", DOCK_LEFT as i64),
            ("BAA_DOCK_UNDOCKED", DOCK_UNDOCKED as i64),
            ("BAA_DOWNLOAD_IN_PROGRESS", DOWNLOAD_IN_PROGRESS as i64),
            ("BAA_DOWNLOAD_COMPLETE", DOWNLOAD_COMPLETE as i64),
            ("BAA_DOWNLOAD_CANCELED", DOWNLOAD_CANCELED as i64),
        ];
        let theirs = header_constants(header);
        // A constant added to the header without a mirror here fails the test.
        for (name, value) in &theirs {
            let mine = ours.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("{name} is not in ffi.rs"));
            assert_eq!(mine.1, *value, "{name}");
        }
        assert_eq!(theirs.len(), ours.len(), "a constant in ffi.rs is gone from baa.h");
    }
}
