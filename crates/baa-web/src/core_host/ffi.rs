//! BaaCore's C ABI (`chromium/baa/baa.h`), loaded with dlopen like Chrome's
//! own launcher does, so cargo never links against the Chromium build.

use std::ffi::{c_char, c_int, c_void, CStr, CString};

pub const ABI_VERSION: c_int = 4;

#[repr(C)]
pub struct Profile {
    _private: [u8; 0],
}

#[repr(C)]
pub struct Tab {
    _private: [u8; 0],
}

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

pub const MENU_SEPARATOR: u8 = 1;
pub const MENU_SUBMENU: u8 = 2;
pub const MENU_INSPECT: u8 = 3;

#[repr(C)]
pub struct Download {
    pub id: u32,
    pub url: *const c_char,
    pub path: *const c_char,
    pub received_bytes: i64,
    pub total_bytes: i64,
    pub state: c_int,
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
}

pub type ExtensionVisitor = extern "C" fn(ctx: *mut c_void, extension: *const Extension);

pub type HistoryVisitor = extern "C" fn(ctx: *mut c_void, title: *const c_char, url: *const c_char);

#[repr(C)]
pub struct Callbacks {
    pub key: extern "C" fn(ctx: *mut c_void, ns_event: *mut c_void, after_page: bool) -> bool,
    pub tab_created: extern "C" fn(
        ctx: *mut c_void,
        tab: *mut Tab,
        opener: *mut Tab,
        profile: *mut Profile,
        url: *const c_char,
        background: bool,
        user_gesture: bool,
    ),
    pub tab_state: extern "C" fn(ctx: *mut c_void, tab: *mut Tab, state: *const TabState),
    pub tab_favicon:
        extern "C" fn(ctx: *mut c_void, tab: *mut Tab, icon_url: *const c_char, png: *const u8, png_len: usize),
    pub dialog: extern "C" fn(
        ctx: *mut c_void,
        tab: *mut Tab,
        id: u64,
        kind: c_int,
        message: *const c_char,
        default_text: *const c_char,
    ),
    pub dialog_closed: extern "C" fn(ctx: *mut c_void, tab: *mut Tab, id: u64),
    pub tab_closed: extern "C" fn(ctx: *mut c_void, tab: *mut Tab),
    pub permission:
        extern "C" fn(ctx: *mut c_void, tab: *mut Tab, id: u64, origin: *const c_char, kinds: u32),
    pub permission_closed: extern "C" fn(ctx: *mut c_void, tab: *mut Tab, id: u64),
    pub context_menu: extern "C" fn(
        ctx: *mut c_void,
        tab: *mut Tab,
        id: u64,
        x: f32,
        y: f32,
        items: *const MenuItem,
        count: usize,
    ),
    pub download: extern "C" fn(ctx: *mut c_void, tab: *mut Tab, download: *const Download),
    pub resized: extern "C" fn(ctx: *mut c_void, tab: *mut Tab, width: c_int, height: c_int),
    pub page_message: extern "C" fn(ctx: *mut c_void, tab: *mut Tab, message: *const c_char),
    pub fetch_done: extern "C" fn(ctx: *mut c_void, id: u64, ok: bool, body: *const c_char, len: usize),
}

/// Every function of `baa.h`. Plain data, so tests build one from fakes.
pub struct Core {
    pub install_app: extern "C" fn(c_int, *const *const c_char, *const Callbacks, *mut c_void),
    pub exit_code: extern "C" fn() -> c_int,
    /// Chrome is closing every tab for ⌘Q: their `tab_closed` is not the user's doing.
    pub is_quitting: extern "C" fn() -> bool,
    pub profile_open: extern "C" fn(*const c_char) -> *mut Profile,
    pub profile_open_ephemeral: extern "C" fn() -> *mut Profile,
    pub profile_close: extern "C" fn(*mut Profile),
    pub profile_set_pref_bool: extern "C" fn(*mut Profile, *const c_char, bool),
    pub profile_site_permission: extern "C" fn(*mut Profile, *const c_char, u32) -> c_int,
    pub profile_set_site_permission: extern "C" fn(*mut Profile, *const c_char, u32, c_int),
    pub profile_fetch: extern "C" fn(*mut Profile, *const c_char, u64),
    pub profile_extensions: extern "C" fn(*mut Profile, ExtensionVisitor, *mut c_void) -> c_int,
    pub profile_extension_change: extern "C" fn(*mut Profile, *const c_char, c_int, c_int) -> bool,
    pub extension_popup: extern "C" fn(*mut Profile, *const c_char) -> *mut Tab,
    pub tab_create: extern "C" fn(*mut Profile, *const c_char) -> *mut Tab,
    pub tab_close: extern "C" fn(*mut Tab),
    pub tab_get_state: extern "C" fn(*mut Tab, *mut TabState) -> bool,
    pub tab_load_url: extern "C" fn(*mut Tab, *const c_char),
    pub tab_reload: extern "C" fn(*mut Tab),
    pub tab_stop: extern "C" fn(*mut Tab),
    pub tab_go_to_offset: extern "C" fn(*mut Tab, c_int),
    pub tab_history: extern "C" fn(*mut Tab, HistoryVisitor, *mut c_void) -> c_int,
    pub tab_eval: extern "C" fn(*mut Tab, *const c_char, bool),
    pub tab_set_zoom: extern "C" fn(*mut Tab, f64),
    pub tab_find: extern "C" fn(*mut Tab, *const c_char, bool),
    pub tab_print: extern "C" fn(*mut Tab),
    pub tab_set_muted: extern "C" fn(*mut Tab, bool),
    pub tab_activate: extern "C" fn(*mut Tab),
    pub tab_focus: extern "C" fn(*mut Tab),
    pub tab_set_init_script: extern "C" fn(*mut Tab, *const c_char),
    pub tab_set_color_scheme: extern "C" fn(*mut Tab, c_int),
    pub tab_open_devtools: extern "C" fn(*mut Tab, bool, c_int, c_int) -> *mut Tab,
    pub tab_show_devtools_window: extern "C" fn(*mut Tab, bool, c_int, c_int),
    pub permission_answer: extern "C" fn(u64, bool),
    pub context_menu_done: extern "C" fn(u64, c_int),
    pub download_cancel: extern "C" fn(u32),
    pub dialog_answer: extern "C" fn(u64, bool, *const c_char),
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
            profile_set_pref_bool,
            profile_site_permission,
            profile_set_site_permission,
            profile_fetch,
            profile_extensions,
            profile_extension_change,
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
            tab_set_color_scheme,
            tab_open_devtools,
            tab_show_devtools_window,
            permission_answer,
            context_menu_done,
            download_cancel,
            dialog_answer,
        )))
    }
}
