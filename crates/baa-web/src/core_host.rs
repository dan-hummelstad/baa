//! [`WebHost`] on BaaCore, baa's own Chromium, through the C ABI of
//! `chromium/baa/baa.h` (PLAN-chromium-phase3.md). Pages are Chrome tabs whose
//! NSView sits under gpui's view, seen through the hole each page element punches.

mod ffi;
mod menu;
mod native;
#[cfg(test)]
mod tests;

use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::HashMap,
    ffi::{c_char, c_int, c_void, CStr, CString},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Once},
    time::Duration,
};

use gpui::{
    div, relative, AnyElement, App, AppContext as _, AsyncApp, Bounds, Context, DispatchPhase, Element, ElementId,
    Entity, EventEmitter, FocusHandle, Focusable, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId,
    InteractiveElement, IntoElement, LayoutId, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    Render, ScrollWheelEvent, Style, Styled, Subscription, WeakEntity, Window,
};
use objc2::runtime::AnyObject;

use self::ffi::{Callbacks, Core, Profile, Tab, TabState};
use crate::{
    cef_host::{installed_extensions, is_ephemeral, profile_dir, queue_delete},
    Callback, ChromeSheet, ChromeTabs, DialogKind, Downloads, Extension, ExtensionChange, ExtensionInfo, MenuDone,
    MenuItem, PageEvent, PageHandle, Permission, PermissionState, ProfileId, WebHost,
};

/// How long a tab Chrome opened waits for the UI to take it before it is closed.
const ADOPTION_EXPIRY: Duration = Duration::from_secs(60);

/// Chrome's command line for baa: the user's `-` arguments from `args`
/// (`args[0]` is the executable), no window of its own, baa's privacy
/// switches. URLs in `args` are not passed on.
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
    for switch in ["no-first-run"].iter().chain(baa_privacy::SWITCHES) {
        let switch = format!("--{switch}");
        if !out.contains(&switch) {
            out.push(switch);
        }
    }
    out.push(format!("--disable-features={}", baa_privacy::merge_disabled_features(&disabled)));
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

/// A tab Chrome opened that baa has not taken yet.
struct Pending {
    tab: *mut Tab,
    profile: *mut Profile,
    url: String,
    /// Tells this entry from a later tab at the same address, for its timer.
    seq: u64,
}

/// What callbacks and pages share about one tab. Main thread only.
struct Shared {
    core: &'static Core,
    profile: *mut Profile,
    container: Rc<native::Container>,
    /// Null once the tab is gone (Chrome closed it, or baa did).
    tab: Cell<*mut Tab>,
    /// The tab's NSView while it is in the container; null once released.
    view: Cell<*mut AnyObject>,
    dialog: Cell<Option<u64>>,
    /// The view a mouse drag started on, and whether the pointer is over the
    /// page; across frames, so a repaint mid-click loses nothing.
    drag: Cell<Option<*mut AnyObject>>,
    inside: Cell<bool>,
}

impl Shared {
    /// The tab is gone: forget its handle and take its view out of the
    /// container before Chromium frees it. Only AppKit calls, so it is safe
    /// inside a callback.
    fn release(&self) {
        self.tab.set(std::ptr::null_mut());
        self.container.swap_page(self.view.replace(std::ptr::null_mut()), std::ptr::null_mut());
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

struct Live {
    shared: Rc<Shared>,
    page: WeakEntity<CorePage>,
}

/// The callback side. `ctx` of every callback points at it; it lives for the
/// process. Callbacks may run inside any `baa_*` call, so they only record
/// plain data here and queue the rest for the next turn of gpui's executor.
struct Dispatch {
    core: &'static Core,
    /// Before [`CoreRuntime::start`] there is none and callbacks do nothing;
    /// Chrome makes no tabs of its own then (`--no-startup-window`).
    app: RefCell<Option<AsyncApp>>,
    pages: RefCell<HashMap<*mut Tab, Live>>,
    pending: RefCell<Vec<Pending>>,
    chrome_tabs: RefCell<Option<WeakEntity<ChromeTabs>>>,
    next_seq: Cell<u64>,
    /// `--user-data-dir`.
    root: OnceCell<PathBuf>,
}

impl Dispatch {
    fn new(core: &'static Core) -> Self {
        Self {
            core,
            app: RefCell::default(),
            pages: RefCell::default(),
            pending: RefCell::default(),
            chrome_tabs: RefCell::default(),
            next_seq: Cell::new(0),
            root: OnceCell::new(),
        }
    }

    fn queue(&self, f: impl FnOnce(&mut AsyncApp) + 'static) {
        let app = self.app.borrow().clone();
        if let Some(app) = app {
            app.spawn(async move |cx| f(cx)).detach();
        }
    }

    /// Runs `f` on the page of `tab` next turn; nothing for an unknown tab.
    fn on_page(&self, tab: *mut Tab, f: impl FnOnce(&mut CorePage, &mut Context<CorePage>) + 'static) {
        let page = self.pages.borrow().get(&tab).map(|live| live.page.clone());
        if let Some(page) = page {
            self.queue(move |cx| {
                page.update(cx, f).ok();
            });
        }
    }

    fn chrome_tab(&self, url: String, cx: &mut AsyncApp) {
        let tabs = self.chrome_tabs.borrow().clone();
        if let Some(tabs) = tabs {
            tabs.update(cx, |tabs, cx| {
                tabs.0.push(url);
                cx.notify();
            })
            .ok();
        }
    }

    fn tab_created(
        &'static self,
        tab: *mut Tab,
        opener: *mut Tab,
        profile: *mut Profile,
        url: String,
        background: bool,
        from_user_gesture: bool,
    ) {
        let Some(app) = self.app.borrow().clone() else { return };
        if profile.is_null() {
            // A profile baa never opened: no page of ours can show it.
            self.queue(move |cx| {
                (self.core.tab_close)(tab);
                self.chrome_tab(url, cx);
            });
            return;
        }
        let seq = self.next_seq.get();
        self.next_seq.set(seq + 1);
        self.pending.borrow_mut().push(Pending { tab, profile, url: url.clone(), seq });
        let opener = self.pages.borrow().get(&opener).map(|live| live.page.clone());
        self.queue(move |cx| {
            let requested = PageEvent::OpenRequested { url: url.clone(), from_user_gesture, background };
            let delivered = opener.and_then(|page| page.update(cx, |_, cx| cx.emit(requested)).ok());
            if delivered.is_none() {
                self.chrome_tab(url, cx);
            } else if !background {
                // The UI takes it (create_page) while handling the event, or
                // not at all: a tab nobody shows must not linger.
                cx.update(|cx| cx.defer(move |_| self.expire(seq)));
            }
        });
        app.spawn(async move |cx| {
            cx.background_executor().timer(ADOPTION_EXPIRY).await;
            self.expire(seq);
        })
        .detach();
    }

    /// Closes the pending tab `seq` unless it was adopted meanwhile.
    fn expire(&self, seq: u64) {
        let mut pending = self.pending.borrow_mut();
        if let Some(at) = pending.iter().position(|p| p.seq == seq) {
            let expired = pending.remove(at);
            drop(pending);
            (self.core.tab_close)(expired.tab);
        }
    }

    /// A tab's snapshot. A changed view is swapped in now: Chrome frees the
    /// old one as soon as this callback returns.
    fn tab_state(&self, tab: *mut Tab, snapshot: Snapshot) {
        let Some((shared, page)) = self.pages.borrow().get(&tab).map(|l| (l.shared.clone(), l.page.clone())) else {
            return;
        };
        let view_changed = snapshot.view != shared.view.get();
        if view_changed {
            shared.container.swap_page(shared.view.replace(snapshot.view), snapshot.view);
            shared.container.page_created(snapshot.view);
        }
        self.queue(move |cx| {
            page.update(cx, |page, cx| page.apply(snapshot, view_changed, cx)).ok();
        });
    }

    /// The profile is going away with its tabs, which Chrome may not report.
    fn forget_profile(&self, profile: *mut Profile) {
        self.pending.borrow_mut().retain(|p| p.profile != profile);
        let gone: Vec<_> =
            self.pages.borrow().iter().filter(|(_, l)| l.shared.profile == profile).map(|(tab, _)| *tab).collect();
        for tab in gone {
            let live = self.pages.borrow_mut().remove(&tab);
            if let Some(live) = live {
                live.shared.release();
            }
        }
    }

    fn take_pending(&self, profile: *mut Profile, url: &str) -> Option<*mut Tab> {
        let mut pending = self.pending.borrow_mut();
        let at = pending.iter().position(|p| p.profile == profile && p.url == url)?;
        Some(pending.remove(at).tab)
    }

    fn tab_closed(&self, tab: *mut Tab) {
        self.pending.borrow_mut().retain(|p| p.tab != tab);
        let Some(live) = self.pages.borrow_mut().remove(&tab) else { return };
        // Now, not next turn: the handle is dead, its address may be reused, and
        // its view is freed once this callback returns.
        live.shared.release();
        if (self.core.is_quitting)() {
            // ⌘Q closes every tab; the session must keep them.
            return;
        }
        self.queue(move |cx| {
            live.page.update(cx, |page, cx| page.chrome_closed(cx)).ok();
        });
    }
}

fn callbacks() -> Callbacks {
    unsafe fn dispatch(ctx: *mut c_void) -> &'static Dispatch {
        unsafe { &*(ctx as *const Dispatch) }
    }
    extern "C" fn key(_: *mut c_void, event: *mut c_void, after_page: bool) -> bool {
        let event = event.cast::<AnyObject>();
        if !after_page {
            return native::offer_key_equivalent(event);
        }
        // Editing shortcuts the page left alone go to the page's view as edit
        // actions: no Chrome window or menu behind it would run them.
        native::edit_action(event).is_some_and(native::send_edit_action)
    }
    extern "C" fn tab_created(
        ctx: *mut c_void,
        tab: *mut Tab,
        opener: *mut Tab,
        profile: *mut Profile,
        url: *const c_char,
        background: bool,
        user_gesture: bool,
    ) {
        unsafe { dispatch(ctx).tab_created(tab, opener, profile, from_c(url), background, user_gesture) }
    }
    extern "C" fn tab_state(ctx: *mut c_void, tab: *mut Tab, state: *const TabState) {
        let snapshot = Snapshot::from_c(unsafe { &*state });
        unsafe { dispatch(ctx) }.tab_state(tab, snapshot);
    }
    extern "C" fn tab_favicon(ctx: *mut c_void, tab: *mut Tab, icon_url: *const c_char, png: *const u8, len: usize) {
        let url = unsafe { from_c(icon_url) };
        let png = Arc::new(if png.is_null() { Vec::new() } else { unsafe { std::slice::from_raw_parts(png, len) }.to_vec() });
        unsafe { dispatch(ctx) }.on_page(tab, move |_, cx| cx.emit(PageEvent::FaviconImage { url, png }));
    }
    extern "C" fn dialog(
        ctx: *mut c_void,
        tab: *mut Tab,
        id: u64,
        kind: c_int,
        message: *const c_char,
        default_text: *const c_char,
    ) {
        let (message, default_text) = unsafe { (from_c(message), from_c(default_text)) };
        let kind = match kind {
            1 => DialogKind::Confirm,
            2 => DialogKind::Prompt,
            _ => DialogKind::Alert,
        };
        unsafe { dispatch(ctx) }.on_page(tab, move |page, cx| {
            page.shared.dialog.set(Some(id));
            cx.emit(PageEvent::Dialog { kind, message, default_text });
        });
    }
    extern "C" fn dialog_closed(ctx: *mut c_void, tab: *mut Tab, id: u64) {
        unsafe { dispatch(ctx) }.on_page(tab, move |page, cx| {
            if page.shared.dialog.get() == Some(id) {
                page.shared.dialog.set(None);
            }
            cx.emit(PageEvent::DialogClosed);
        });
    }
    extern "C" fn tab_closed(ctx: *mut c_void, tab: *mut Tab) {
        unsafe { dispatch(ctx) }.tab_closed(tab);
    }
    Callbacks { key, tab_created, tab_state, tab_favicon, dialog, dialog_closed, tab_closed }
}

pub struct CoreWebHost {
    core: &'static Core,
    dispatch: &'static Dispatch,
    /// `--user-data-dir`: profiles live in `root/space-<id>`.
    root: PathBuf,
    profiles: RefCell<HashMap<ProfileId, *mut Profile>>,
    next_ephemeral: Cell<u64>,
    downloads: Entity<Downloads>,
    chrome_tabs: Entity<ChromeTabs>,
}

impl CoreWebHost {
    fn new(core: &'static Core, dispatch: &'static Dispatch, root: PathBuf, cx: &mut App) -> Self {
        let chrome_tabs = cx.new(|_| ChromeTabs::default());
        *dispatch.chrome_tabs.borrow_mut() = Some(chrome_tabs.downgrade());
        *dispatch.app.borrow_mut() = Some(cx.to_async());
        Self {
            core,
            dispatch,
            root,
            profiles: RefCell::default(),
            next_ephemeral: Cell::new(u64::MAX),
            downloads: cx.new(|_| Downloads::default()),
            chrome_tabs,
        }
    }

    fn profile_handle(&self, id: ProfileId) -> *mut Profile {
        if let Some(handle) = self.profiles.borrow().get(&id) {
            return *handle;
        }
        let handle = if is_ephemeral(id) {
            (self.core.profile_open_ephemeral)()
        } else {
            (self.core.profile_open)(c_string(&format!("space-{}", id.0)).as_ptr())
        };
        if handle.is_null() {
            log::error!("profile {id:?}: Chrome refused it");
        } else {
            self.profiles.borrow_mut().insert(id, handle);
        }
        handle
    }
}

impl WebHost for CoreWebHost {
    type Page = CorePage;

    fn create_devtools(
        &self,
        of: &Entity<CorePage>,
        _inspect_at: Option<(i32, i32)>,
        _side: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<CorePage> {
        // ponytail: M1 has no DevTools; an empty page keeps the UI's dock working.
        // Made directly: a pending about:blank popup is not ours to take.
        let profile = of.read(cx).profile;
        let handle = self.profile_handle(profile);
        let tab = if handle.is_null() {
            std::ptr::null_mut()
        } else {
            (self.core.tab_create)(handle, c"about:blank".as_ptr())
        };
        let dispatch = self.dispatch;
        cx.new(|cx| CorePage::new(dispatch, profile, handle, tab, window, cx))
    }

    fn show_menu(&self, at: gpui::Point<Pixels>, items: Vec<MenuItem>, window: &mut Window, cx: &mut App, done: MenuDone) {
        menu::show_menu(window, cx, at, items, done);
    }

    fn create_page(&self, profile: ProfileId, url: &str, window: &mut Window, cx: &mut App) -> Entity<CorePage> {
        let handle = self.profile_handle(profile);
        let tab = if handle.is_null() {
            std::ptr::null_mut()
        } else {
            // A tab Chrome already made for this (window.open) is taken, not duplicated.
            self.dispatch
                .take_pending(handle, url)
                .unwrap_or_else(|| (self.core.tab_create)(handle, c_string(url).as_ptr()))
        };
        let dispatch = self.dispatch;
        cx.new(|cx| CorePage::new(dispatch, profile, handle, tab, window, cx))
    }

    fn create_popup(
        &self,
        profile: ProfileId,
        url: &str,
        _tab_url: &str,
        _tab_title: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<CorePage> {
        // ponytail: a background tab; chrome.tabs.query({active}) still answers
        // the shown tab because baa activates it. Ceiling: popups sized to content (M2).
        self.create_page(profile, url, window, cx)
    }

    fn ephemeral_profile(&self) -> ProfileId {
        let id = self.next_ephemeral.get();
        self.next_ephemeral.set(id - 1);
        ProfileId(id)
    }

    fn delete_profile(&self, profile: ProfileId) {
        if let Some(handle) = self.profiles.borrow_mut().remove(&profile) {
            self.dispatch.forget_profile(handle);
            (self.core.profile_close)(handle);
        }
        if !is_ephemeral(profile) {
            // Next launch removes the directory, before Chrome opens anything in it.
            queue_delete(&self.root, profile);
        }
    }

    fn extension_details(&self, _profile: ProfileId, cx: &mut App, done: Callback<Option<Vec<ExtensionInfo>>>) {
        done(Some(Vec::new()), cx);
    }

    fn show_chrome_page(&self, _profile: ProfileId, _sheet: Option<ChromeSheet>, _window: &mut Window) -> bool {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| log::info!("Chrome's own pages are not shown yet (M2)"));
        true
    }

    fn change_extension(&self, _profile: ProfileId, _id: &str, _change: ExtensionChange, cx: &mut App, done: Callback<bool>) {
        done(false, cx);
    }

    fn extensions(&self, profile: ProfileId) -> Vec<Extension> {
        if is_ephemeral(profile) {
            return Vec::new();
        }
        installed_extensions(&profile_dir(&self.root, profile))
    }

    fn fetch(&self, _profile: ProfileId, _url: &str, done: Box<dyn FnOnce(Option<String>)>) {
        done(None);
    }

    fn downloads(&self) -> Entity<Downloads> {
        self.downloads.clone()
    }

    fn chrome_tabs(&self) -> Entity<ChromeTabs> {
        self.chrome_tabs.clone()
    }

    fn cancel_download(&self, _id: u32) {}

    fn site_permission(&self, _profile: ProfileId, _url: &str, _kind: Permission) -> PermissionState {
        PermissionState::Ask
    }

    fn set_site_permission(&self, _profile: ProfileId, _url: &str, _kind: Permission, _state: PermissionState) {}
}

/// One Chrome tab as a gpui entity.
pub struct CorePage {
    dispatch: &'static Dispatch,
    shared: Rc<Shared>,
    profile: ProfileId,
    state: Snapshot,
    container: Rc<native::Container>,
    focus_handle: FocusHandle,
    /// gpui's own view, which gets keyboard focus back when the page loses it.
    gpui_view: *mut AnyObject,
    corner_radius: Cell<f32>,
    square_corners: Cell<u8>,
    this: WeakEntity<CorePage>,
    _focus: [Subscription; 2],
}

impl CorePage {
    /// `tab` may be null (the app is terminating): the page then renders empty.
    fn new(
        dispatch: &'static Dispatch,
        profile: ProfileId,
        handle: *mut Profile,
        tab: *mut Tab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut state = Snapshot::default();
        if !tab.is_null() {
            let mut raw: TabState = unsafe { std::mem::zeroed() };
            if (dispatch.core.tab_get_state)(tab, &mut raw) {
                state = Snapshot::from_c(&raw);
            }
        }
        let gpui_view = native::gpui_view(window);
        native::install_mouse_forwarding(gpui_view);
        let container = Rc::new(native::Container::new(gpui_view, state.view, 0.));
        container.page_created(state.view);
        let shared = Rc::new(Shared {
            core: dispatch.core,
            profile: handle,
            container: container.clone(),
            tab: Cell::new(tab),
            view: Cell::new(state.view),
            dialog: Cell::new(None),
            drag: Cell::new(None),
            inside: Cell::new(false),
        });
        if !tab.is_null() {
            dispatch.pages.borrow_mut().insert(tab, Live { shared: shared.clone(), page: cx.weak_entity() });
        }
        let focus_handle = cx.focus_handle();
        let focus = subscribe(&focus_handle, window, cx);
        if !tab.is_null() {
            // An adopted tab already has a url and title: the UI hears them
            // once it has subscribed, which is after this constructor.
            let first = state.clone();
            cx.spawn(async move |page, cx| {
                page.update(cx, |page, cx| page.emit_changes(&Snapshot::default(), &first, cx)).ok();
            })
            .detach();
        }
        Self {
            dispatch,
            shared,
            profile,
            state,
            container,
            focus_handle,
            gpui_view,
            corner_radius: Cell::new(0.),
            square_corners: Cell::new(0),
            this: cx.weak_entity(),
            _focus: focus,
        }
    }

    /// Diffs a tab snapshot against the last one and reports what changed.
    /// The view was already swapped by the callback.
    fn apply(&mut self, new: Snapshot, view_changed: bool, cx: &mut Context<Self>) {
        if self.shared.tab.get().is_null() {
            return;
        }
        let old = std::mem::replace(&mut self.state, new.clone());
        self.emit_changes(&old, &new, cx);
        if view_changed {
            cx.notify();
        }
    }

    fn emit_changes(&self, old: &Snapshot, new: &Snapshot, cx: &mut Context<Self>) {
        if new.url != old.url {
            cx.emit(PageEvent::UrlChanged(new.url.clone()));
        }
        if new.title != old.title {
            cx.emit(PageEvent::TitleChanged(new.title.clone()));
        }
        if new.favicon != old.favicon {
            cx.emit(PageEvent::FaviconChanged((!new.favicon.is_empty()).then(|| new.favicon.clone())));
        }
        if new.loading != old.loading {
            cx.emit(PageEvent::LoadingChanged(new.loading));
        }
        if (new.can_go_back, new.can_go_forward) != (old.can_go_back, old.can_go_forward) {
            cx.emit(PageEvent::HistoryChanged { can_go_back: new.can_go_back, can_go_forward: new.can_go_forward });
        }
        if new.audible != old.audible {
            cx.emit(PageEvent::AudibleChanged(new.audible));
        }
    }

    fn chrome_closed(&mut self, cx: &mut Context<Self>) {
        cx.emit(PageEvent::Closed);
    }

    fn set_browser_focus(&self, focused: bool) {
        let view = self.shared.view.get();
        if focused {
            self.shared.run(|core, tab| {
                (core.tab_focus)(tab);
                (core.tab_activate)(tab);
            });
        } else if native::has_first_responder(view) {
            native::make_first_responder(self.gpui_view);
        }
    }
}

/// Keyboard focus follows gpui's: the page's view is first responder while
/// the page is focused, so typing and IME go to Chromium natively.
fn subscribe(focus_handle: &FocusHandle, window: &mut Window, cx: &mut Context<CorePage>) -> [Subscription; 2] {
    [
        cx.on_focus_in(focus_handle, window, |page, _, _| page.set_browser_focus(true)),
        cx.on_focus_out(focus_handle, window, |page, _, _, _| page.set_browser_focus(false)),
    ]
}

impl Drop for CorePage {
    fn drop(&mut self) {
        // The view leaves the window before Chrome destroys it.
        self.container.set_visible(false, self.shared.view.get());
        let tab = self.shared.tab.get();
        self.shared.release();
        if !tab.is_null() {
            self.dispatch.pages.borrow_mut().remove(&tab);
            (self.dispatch.core.tab_close)(tab);
        }
    }
}

impl EventEmitter<PageEvent> for CorePage {}

impl Focusable for CorePage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CorePage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus_handle).key_context("Webview").size_full().child(PageSlot {
            shared: self.shared.clone(),
            container: self.container.clone(),
            focus_handle: self.focus_handle.clone(),
            corner_radius: self.corner_radius.get(),
            square_corners: self.square_corners.get(),
        })
    }
}

impl PageHandle for CorePage {
    fn load_url(&self, url: &str) {
        self.shared.run(|core, tab| (core.tab_load_url)(tab, c_string(url).as_ptr()));
    }
    fn reload(&self) {
        self.shared.run(|core, tab| (core.tab_reload)(tab));
    }
    fn stop(&self) {
        self.shared.run(|core, tab| (core.tab_stop)(tab));
    }
    fn go_back(&self) {
        self.go_to_offset(-1);
    }
    fn go_forward(&self) {
        self.go_to_offset(1);
    }
    fn url(&self) -> String {
        self.state.url.clone()
    }
    fn title(&self) -> String {
        self.state.title.clone()
    }
    fn is_loading(&self) -> bool {
        self.state.loading
    }
    fn can_go_back(&self) -> bool {
        self.state.can_go_back
    }
    fn can_go_forward(&self) -> bool {
        self.state.can_go_forward
    }
    fn history(&self) -> (Vec<(String, String)>, usize) {
        extern "C" fn visit(entries: *mut c_void, title: *const c_char, url: *const c_char) {
            let entries = unsafe { &mut *entries.cast::<Vec<(String, String)>>() };
            entries.push(unsafe { (from_c(title), from_c(url)) });
        }
        let mut entries = Vec::new();
        let mut current = 0;
        self.shared.run(|core, tab| {
            current = (core.tab_history)(tab, visit, (&mut entries as *mut Vec<_>).cast());
        });
        (entries, current.max(0) as usize)
    }
    fn go_to_offset(&self, offset: i32) {
        self.shared.run(|core, tab| (core.tab_go_to_offset)(tab, offset));
    }
    fn eval(&self, script: &str) {
        self.shared.run(|core, tab| (core.tab_eval)(tab, c_string(script).as_ptr(), false));
    }
    fn set_zoom(&self, level: f64) {
        self.shared.run(|core, tab| (core.tab_set_zoom)(tab, level));
    }
    fn find(&self, text: &str, forward: bool) {
        self.shared.run(|core, tab| (core.tab_find)(tab, c_string(text).as_ptr(), forward));
    }
    fn open_devtools(&self, _inspect_at: Option<(i32, i32)>) {
        log::info!("DevTools: not in M1");
    }
    fn answer_permission(&self, _id: u64, _allow: bool) {}
    fn answer_dialog(&self, ok: bool, text: &str) {
        if let Some(id) = self.shared.dialog.take() {
            (self.dispatch.core.dialog_answer)(id, ok, c_string(text).as_ptr());
        }
    }
    fn print(&self) {
        self.shared.run(|core, tab| (core.tab_print)(tab));
    }
    fn set_muted(&self, muted: bool) {
        self.shared.run(|core, tab| (core.tab_set_muted)(tab, muted));
    }
    fn is_secure(&self) -> bool {
        self.state.secure
    }
    fn eval_with_gesture(&self, script: &str) {
        self.shared.run(|core, tab| (core.tab_eval)(tab, c_string(script).as_ptr(), true));
    }
    fn set_visible(&self, visible: bool) {
        self.container.set_visible(visible, self.shared.view.get());
        // The tab on show is the one the user is on, focused or not.
        if visible {
            self.shared.run(|core, tab| (core.tab_activate)(tab));
        }
    }
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        Focusable::focus_handle(self, cx)
    }
    fn render(&self, _window: &mut Window, _cx: &mut App) -> AnyElement {
        match self.this.upgrade() {
            Some(page) => page.into_any_element(),
            None => div().into_any_element(),
        }
    }
    fn set_corner_radius(&self, radius: f32) {
        self.container.set_corner_radius(radius);
        self.corner_radius.set(radius);
    }
    fn set_square_corners(&self, square: u8) {
        self.container.set_square_corners(square);
        self.square_corners.set(square);
    }
    fn set_floating(&self, floating: bool) {
        self.container.set_floating(floating);
    }
    fn move_to_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let gpui_view = native::gpui_view(window);
        self.gpui_view = gpui_view;
        self.container.move_below(gpui_view);
        native::install_mouse_forwarding(gpui_view);
        self._focus = subscribe(&self.focus_handle, window, cx);
        cx.notify();
    }
}

/// Where the page goes: moves the native view to the laid-out rect and makes
/// gpui's layer transparent over it.
struct PageSlot {
    shared: Rc<Shared>,
    container: Rc<native::Container>,
    focus_handle: FocusHandle,
    corner_radius: f32,
    square_corners: u8,
}

impl IntoElement for PageSlot {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for PageSlot {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> Hitbox {
        let visible = bounds.intersect(&window.content_mask().bounds);
        if visible.size.width > Pixels::ZERO && visible.size.height > Pixels::ZERO {
            if self.container.place(self.shared.view.get(), bounds, visible) {
                window.request_animation_frame();
            }
        } else {
            self.container.set_hidden(true);
        }
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        _: &mut App,
    ) {
        let radius = (self.corner_radius * window.scale_factor()).round() as usize;
        if let Some(hole) = native::punch_buffer(radius, self.square_corners) {
            window.paint_surface(bounds, hole);
        }

        // gpui hit-tests (overlays drawn over the page keep the mouse); the
        // page gets the original NSEvent, so Chromium sees real clicks,
        // trackpad phases and momentum. A drag stays with the view it started on.
        let (shared, hit, focus) = (self.shared.clone(), hitbox.clone(), self.focus_handle.clone());
        window.on_mouse_event(move |_: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hit.is_hovered(window) {
                shared.drag.set(native::forward_current_event(shared.view.get(), None));
                window.focus(&focus, cx);
                shared.run(|core, tab| (core.tab_focus)(tab));
            }
        });
        let (shared, hit) = (self.shared.clone(), hitbox.clone());
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            if let Some(view) = shared.drag.get().filter(|_| event.pressed_button.is_some()) {
                native::forward_current_event(shared.view.get(), Some(view));
                return;
            }
            // Moves over the page, and the first one off it so hover ends.
            let over = hit.is_hovered(window);
            if over || shared.inside.get() {
                shared.inside.set(over);
                native::forward_current_event(shared.view.get(), None);
            }
        });
        let shared = self.shared.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, _| {
            if phase == DispatchPhase::Bubble {
                if let Some(view) = shared.drag.take() {
                    native::forward_current_event(shared.view.get(), Some(view));
                }
            }
        });
        let (shared, hit) = (self.shared.clone(), hitbox.clone());
        window.on_mouse_event(move |_: &ScrollWheelEvent, phase, window, _| {
            if phase == DispatchPhase::Bubble && hit.should_handle_scroll(window) {
                native::forward_current_event(shared.view.get(), None);
            }
        });
    }
}
