//! [`WebHost`] on BaaCore, baa's own Chromium, through the C ABI of
//! `chromium/baa/baa.h` (PLAN-chromium-phase3.md). Pages are Chrome tabs whose
//! NSView sits under gpui's view, seen through the hole each page element punches.

mod devtools;
mod dispatch;
mod extensions;
mod ffi;
mod host;
mod menu;
mod native;
mod page;
pub(crate) mod profiles;
mod sheet;
mod slot;
#[cfg(test)]
mod tests;

use std::{
    cell::Cell,
    ffi::{c_char, c_int, c_void, CStr, CString},
    path::{Path, PathBuf},
    rc::Rc,
};

use gpui::App;
use objc2::runtime::AnyObject;

use self::{
    dispatch::{callbacks, Dispatch},
    ffi::{Core, Profile, Tab, TabState},
};
pub use self::{host::CoreWebHost, page::CorePage};

// objc2 looks classes up by name at run time; without this a test binary that
// does not pull in gpui's backend has no NSView.
#[link(name = "AppKit", kind = "framework")]
extern "C" {}

/// Chrome's command line for baa: the user's `-` arguments from `args`
/// (`args[0]` is the executable), no window of its own, baa's privacy
/// switches (the short core list). URLs in `args` are not passed on.
pub fn chrome_argv(args: &[String], user_data_dir: &Path) -> Vec<String> {
    let mut out = vec![args[0].clone()];
    let mut disabled = String::new();
    for arg in args.iter().skip(1).filter(|a| a.starts_with('-')) {
        match arg.strip_prefix("--disable-features=") {
            // Chrome keeps only the last --disable-features, so ours is merged into it.
            Some(list) => {
                disabled.push(',');
                disabled.push_str(list);
            }
            None => out.push(arg.clone()),
        }
    }
    out.push("--no-startup-window".into());
    if !out.iter().any(|a| a.starts_with("--user-data-dir=")) {
        out.push(format!("--user-data-dir={}", user_data_dir.display()));
    }
    for switch in ["no-first-run"].iter().chain(baa_privacy::CORE_SWITCHES) {
        let switch = format!("--{switch}");
        if !out.contains(&switch) {
            out.push(switch);
        }
    }
    let mut features: Vec<&str> = disabled.split(',').filter(|f| !f.is_empty()).collect();
    for feature in baa_privacy::CORE_DISABLED_FEATURES {
        if !features.contains(feature) {
            features.push(feature);
        }
    }
    out.push(format!("--disable-features={}", features.join(",")));
    out
}

/// BaaCore in this process. [`CoreRuntime::load`] first thing in `main`,
/// [`CoreRuntime::install`] before anything touches NSApp,
/// [`CoreRuntime::start`] inside `Application::run`.
#[derive(Clone, Copy)]
pub struct CoreRuntime {
    core: &'static Core,
    dispatch: &'static Dispatch,
}

impl CoreRuntime {
    /// dlopens the framework; call it while the process has one thread.
    pub fn load() -> Self {
        let core = ffi::load();
        Self { core, dispatch: Box::leak(Box::new(Dispatch::new(core))) }
    }

    /// Hands Chrome its command line and the callbacks. The first
    /// `-[NSApp run]` then runs Chromium's main.
    pub fn install(&self, argv: Vec<String>) {
        let root = argv.iter().find_map(|a| a.strip_prefix("--user-data-dir=")).map(PathBuf::from);
        self.dispatch.root.set(root.expect("--user-data-dir in argv")).ok();
        // ChromeMain reads argv for the life of the process.
        let argv: &'static [*const c_char] =
            Vec::leak(argv.into_iter().map(|a| c_string(&a).into_raw() as *const c_char).collect());
        (self.core.install_app)(
            argv.len() as c_int,
            argv.as_ptr(),
            &callbacks(),
            self.dispatch as *const Dispatch as *mut c_void,
        );
        assert!(native::nsapp_has_gpui_ivar(), "NSApp lacks gpui's `platform` ivar");
    }

    /// Inside the `Application::run` closure, once Chromium's browser process is up.
    pub fn start(&self, cx: &mut App) -> CoreWebHost {
        let root = self.dispatch.root.get().expect("install before start").clone();
        CoreWebHost::new(self.core, self.dispatch, root, cx)
    }

    pub fn exit_code(&self) -> i32 {
        (self.core.exit_code)()
    }
}

fn c_string(text: &str) -> CString {
    CString::new(text.replace('\0', "")).unwrap_or_default()
}

/// # Safety
/// `ptr` is null or a NUL-terminated string.
unsafe fn from_c(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
}

/// What a tab looks like: [`TabState`] with owned strings.
#[derive(Clone, PartialEq)]
struct Snapshot {
    url: String,
    title: String,
    favicon: String,
    view: *mut AnyObject,
    loading: bool,
    can_go_back: bool,
    can_go_forward: bool,
    audible: bool,
    secure: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            url: String::new(),
            title: String::new(),
            favicon: String::new(),
            view: std::ptr::null_mut(),
            loading: false,
            can_go_back: false,
            can_go_forward: false,
            audible: false,
            secure: false,
        }
    }
}

impl Snapshot {
    fn from_c(state: &TabState) -> Self {
        unsafe {
            Self {
                url: from_c(state.url),
                title: from_c(state.title),
                favicon: from_c(state.favicon_url),
                view: state.view.cast(),
                loading: state.loading,
                can_go_back: state.can_go_back,
                can_go_forward: state.can_go_forward,
                audible: state.audible,
                secure: state.secure,
            }
        }
    }
}

/// What a `BaaTab` is: any WebContents baa shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PageKind {
    Tab,
    /// An extension's toolbar popup.
    Popup,
    /// A DevTools front-end.
    DevTools,
}

/// What callbacks and pages share about one tab. Main thread only.
struct Shared {
    core: &'static Core,
    kind: PageKind,
    profile: *mut Profile,
    /// Holds the tab's NSView.
    container: native::Container,
    /// Null once the tab is gone (Chrome closed it, or baa did).
    tab: Cell<*mut Tab>,
    dialog: Cell<Option<u64>>,
    /// The view a mouse drag started on, and whether the pointer is over the
    /// page; across frames, so a repaint mid-click loses nothing.
    drag: Cell<Option<*mut AnyObject>>,
    inside: Cell<bool>,
}

impl Shared {
    /// Reads `tab`'s first state (empty for a null tab, as when the app is
    /// terminating) and puts its view in a new container below `gpui_view`.
    fn open(
        core: &'static Core,
        kind: PageKind,
        profile: *mut Profile,
        tab: *mut Tab,
        gpui_view: *mut AnyObject,
        corner_radius: f32,
    ) -> (Rc<Self>, Snapshot) {
        let mut state = Snapshot::default();
        if !tab.is_null() {
            let mut raw: TabState = unsafe { std::mem::zeroed() };
            if (core.tab_get_state)(tab, &mut raw) {
                state = Snapshot::from_c(&raw);
            }
        }
        let shared = Rc::new(Self {
            core,
            kind,
            profile,
            container: native::Container::new(gpui_view, state.view, corner_radius),
            tab: Cell::new(tab),
            dialog: Cell::new(None),
            drag: Cell::new(None),
            inside: Cell::new(false),
        });
        (shared, state)
    }

    /// The tab is gone: forget its handle and take its view out of the
    /// container before Chromium frees it. Only AppKit calls, so it is safe
    /// inside a callback.
    fn release(&self) {
        self.tab.set(std::ptr::null_mut());
        self.container.swap_page(std::ptr::null_mut());
        self.drag.set(None);
        self.inside.set(false);
    }

    fn run(&self, f: impl FnOnce(&Core, *mut Tab)) {
        let tab = self.tab.get();
        if !tab.is_null() {
            f(self.core, tab);
        }
    }
}
