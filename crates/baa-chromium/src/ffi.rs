//! BaaCore's C ABI (`chromium/baa/baa.h`, version 4), loaded with dlopen like
//! Chrome's own launcher does, so cargo never links against the Chromium build.

use std::{
    ffi::{c_char, c_int, c_void, CStr, CString},
    sync::OnceLock,
};

pub type Tab = *mut c_void;
pub type Profile = *mut c_void;

pub const ABI_VERSION: c_int = 4;

pub const EDIT_COPY: c_int = 0;
pub const EDIT_CUT: c_int = 1;
pub const EDIT_PASTE: c_int = 2;
pub const EDIT_SELECT_ALL: c_int = 3;
pub const EDIT_UNDO: c_int = 4;
pub const EDIT_REDO: c_int = 5;

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

impl TabState {
    pub fn empty() -> Self {
        Self {
            url: std::ptr::null(),
            title: std::ptr::null(),
            favicon_url: std::ptr::null(),
            view: std::ptr::null_mut(),
            loading: false,
            can_go_back: false,
            can_go_forward: false,
            audible: false,
            secure: false,
        }
    }
}

#[repr(C)]
pub struct MenuItem {
    pub label: *const c_char,
    pub command: c_int,
    pub depth: u8,
    pub kind: u8,
    pub enabled: bool,
    pub checked: bool,
}

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

pub type ExtensionVisitor = extern "C" fn(*mut c_void, *const Extension);

#[repr(C)]
pub struct Callbacks {
    pub key: extern "C" fn(*mut c_void, *mut c_void, bool) -> bool,
    pub tab_created: extern "C" fn(*mut c_void, Tab, Tab, Profile, *const c_char, bool, bool),
    pub tab_state: extern "C" fn(*mut c_void, Tab, *const TabState),
    pub tab_favicon: extern "C" fn(*mut c_void, Tab, *const c_char, *const u8, usize),
    pub dialog: extern "C" fn(*mut c_void, Tab, u64, c_int, *const c_char, *const c_char),
    pub dialog_closed: extern "C" fn(*mut c_void, Tab, u64),
    pub tab_closed: extern "C" fn(*mut c_void, Tab),
    pub permission: extern "C" fn(*mut c_void, Tab, u64, *const c_char, u32),
    pub permission_closed: extern "C" fn(*mut c_void, Tab, u64),
    pub context_menu: extern "C" fn(*mut c_void, Tab, u64, f32, f32, *const MenuItem, usize),
    pub download: extern "C" fn(*mut c_void, Tab, *const Download),
    pub resized: extern "C" fn(*mut c_void, Tab, c_int, c_int),
    pub page_message: extern "C" fn(*mut c_void, Tab, *const c_char),
    pub fetch_done: extern "C" fn(*mut c_void, u64, bool, *const c_char, usize),
}

pub type HistoryVisitor = extern "C" fn(*mut c_void, *const c_char, *const c_char);

pub struct Core {
    pub install_app: extern "C" fn(c_int, *const *const c_char, *const Callbacks, *mut c_void),
    pub exit_code: extern "C" fn() -> c_int,
    pub is_quitting: extern "C" fn() -> bool,
    pub profile_open: extern "C" fn(*const c_char) -> Profile,
    pub profile_open_ephemeral: extern "C" fn() -> Profile,
    pub profile_close: extern "C" fn(Profile),
    pub profile_set_pref_bool: extern "C" fn(Profile, *const c_char, bool),
    pub profile_site_permission: extern "C" fn(Profile, *const c_char, u32) -> c_int,
    pub profile_set_site_permission: extern "C" fn(Profile, *const c_char, u32, c_int),
    pub profile_fetch: extern "C" fn(Profile, *const c_char, u64),
    pub profile_extensions: extern "C" fn(Profile, ExtensionVisitor, *mut c_void) -> c_int,
    pub profile_extension_change: extern "C" fn(Profile, *const c_char, c_int, c_int) -> bool,
    pub extension_popup: extern "C" fn(Profile, *const c_char) -> Tab,
    pub tab_create: extern "C" fn(Profile, *const c_char) -> Tab,
    pub tab_close: extern "C" fn(Tab),
    pub tab_get_state: extern "C" fn(Tab, *mut TabState) -> bool,
    pub tab_load_url: extern "C" fn(Tab, *const c_char),
    pub tab_reload: extern "C" fn(Tab),
    pub tab_stop: extern "C" fn(Tab),
    pub tab_go_to_offset: extern "C" fn(Tab, c_int),
    pub tab_history: extern "C" fn(Tab, HistoryVisitor, *mut c_void) -> c_int,
    pub tab_eval: extern "C" fn(Tab, *const c_char, bool),
    pub tab_set_zoom: extern "C" fn(Tab, f64),
    pub tab_find: extern "C" fn(Tab, *const c_char, bool),
    pub tab_print: extern "C" fn(Tab),
    pub tab_set_muted: extern "C" fn(Tab, bool),
    pub tab_activate: extern "C" fn(Tab),
    pub tab_focus: extern "C" fn(Tab),
    pub tab_set_init_script: extern "C" fn(Tab, *const c_char),
    pub tab_set_color_scheme: extern "C" fn(Tab, c_int),
    pub tab_open_devtools: extern "C" fn(Tab, bool, c_int, c_int) -> Tab,
    pub tab_show_devtools_window: extern "C" fn(Tab, bool, c_int, c_int),
    pub dialog_answer: extern "C" fn(u64, bool, *const c_char),
    pub permission_answer: extern "C" fn(u64, bool),
    pub context_menu_done: extern "C" fn(u64, c_int),
    pub download_cancel: extern "C" fn(u32),
}

static CORE: OnceLock<Core> = OnceLock::new();

pub fn core() -> &'static Core {
    CORE.get().expect("BaaCore not loaded")
}

/// Loads `../Frameworks/BaaCore.framework` next to the executable. Call it
/// first in `main`: PartitionAlloc installs its malloc zone when the framework
/// loads, which is only safe while the process has one thread.
pub fn load() -> &'static Core {
    let exe = std::env::current_exe().expect("executable path");
    let path = exe
        .parent()
        .expect("executable directory")
        .join("../Frameworks/BaaCore.framework/BaaCore");
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
        let version: extern "C" fn() -> c_int = std::mem::transmute(sym(c"baa_abi_version"));
        assert_eq!(version(), ABI_VERSION, "BaaCore ABI mismatch");
        let core = Core {
            install_app: std::mem::transmute(sym(c"baa_install_app")),
            exit_code: std::mem::transmute(sym(c"baa_exit_code")),
            is_quitting: std::mem::transmute(sym(c"baa_is_quitting")),
            profile_open: std::mem::transmute(sym(c"baa_profile_open")),
            profile_open_ephemeral: std::mem::transmute(sym(c"baa_profile_open_ephemeral")),
            profile_close: std::mem::transmute(sym(c"baa_profile_close")),
            profile_set_pref_bool: std::mem::transmute(sym(c"baa_profile_set_pref_bool")),
            profile_site_permission: std::mem::transmute(sym(c"baa_profile_site_permission")),
            profile_set_site_permission: std::mem::transmute(sym(c"baa_profile_set_site_permission")),
            profile_fetch: std::mem::transmute(sym(c"baa_profile_fetch")),
            profile_extensions: std::mem::transmute(sym(c"baa_profile_extensions")),
            profile_extension_change: std::mem::transmute(sym(c"baa_profile_extension_change")),
            extension_popup: std::mem::transmute(sym(c"baa_extension_popup")),
            tab_create: std::mem::transmute(sym(c"baa_tab_create")),
            tab_close: std::mem::transmute(sym(c"baa_tab_close")),
            tab_get_state: std::mem::transmute(sym(c"baa_tab_get_state")),
            tab_load_url: std::mem::transmute(sym(c"baa_tab_load_url")),
            tab_reload: std::mem::transmute(sym(c"baa_tab_reload")),
            tab_stop: std::mem::transmute(sym(c"baa_tab_stop")),
            tab_go_to_offset: std::mem::transmute(sym(c"baa_tab_go_to_offset")),
            tab_history: std::mem::transmute(sym(c"baa_tab_history")),
            tab_eval: std::mem::transmute(sym(c"baa_tab_eval")),
            tab_set_zoom: std::mem::transmute(sym(c"baa_tab_set_zoom")),
            tab_find: std::mem::transmute(sym(c"baa_tab_find")),
            tab_print: std::mem::transmute(sym(c"baa_tab_print")),
            tab_set_muted: std::mem::transmute(sym(c"baa_tab_set_muted")),
            tab_activate: std::mem::transmute(sym(c"baa_tab_activate")),
            tab_focus: std::mem::transmute(sym(c"baa_tab_focus")),
            tab_set_init_script: std::mem::transmute(sym(c"baa_tab_set_init_script")),
            tab_set_color_scheme: std::mem::transmute(sym(c"baa_tab_set_color_scheme")),
            tab_open_devtools: std::mem::transmute(sym(c"baa_tab_open_devtools")),
            tab_show_devtools_window: std::mem::transmute(sym(c"baa_tab_show_devtools_window")),
            dialog_answer: std::mem::transmute(sym(c"baa_dialog_answer")),
            permission_answer: std::mem::transmute(sym(c"baa_permission_answer")),
            context_menu_done: std::mem::transmute(sym(c"baa_context_menu_done")),
            download_cancel: std::mem::transmute(sym(c"baa_download_cancel")),
        };
        CORE.get_or_init(|| core)
    }
}
