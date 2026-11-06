//! BaaCore's C ABI (`chromium/baa/baa.h`), loaded with dlopen like Chrome's
//! own launcher does, so cargo never links against the Chromium build.

use std::{
    ffi::{c_char, c_int, c_void, CStr, CString},
    sync::OnceLock,
};

pub type WebContents = *mut c_void;
pub type KeyFn = extern "C" fn(*mut c_void, WebContents, *mut c_void, bool) -> bool;

pub const ABI_VERSION: c_int = 2;

pub const EDIT_COPY: c_int = 0;
pub const EDIT_CUT: c_int = 1;
pub const EDIT_PASTE: c_int = 2;
pub const EDIT_SELECT_ALL: c_int = 3;
pub const EDIT_UNDO: c_int = 4;
pub const EDIT_REDO: c_int = 5;

pub struct Core {
    pub install_app: extern "C" fn(c_int, *const *const c_char, KeyFn, *mut c_void),
    pub exit_code: extern "C" fn() -> c_int,
    pub web_contents_create: extern "C" fn(*const c_char) -> WebContents,
    pub web_contents_view: extern "C" fn(WebContents) -> *mut c_void,
    pub web_contents_focus: extern "C" fn(WebContents),
    pub web_contents_edit: extern "C" fn(WebContents, c_int),
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
            web_contents_create: std::mem::transmute(sym(c"baa_web_contents_create")),
            web_contents_view: std::mem::transmute(sym(c"baa_web_contents_view")),
            web_contents_focus: std::mem::transmute(sym(c"baa_web_contents_focus")),
            web_contents_edit: std::mem::transmute(sym(c"baa_web_contents_edit")),
        };
        CORE.get_or_init(|| core)
    }
}
