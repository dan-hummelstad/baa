//! The callback side: what BaaCore reports, queued onto gpui's executor and
//! routed to the page of the tab it is about.

use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::HashMap,
    ffi::{c_char, c_int, c_void},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use gpui::{AsyncApp, Context, WeakEntity};
use objc2::runtime::AnyObject;

use super::{
    ffi::{self, Callbacks, Core, Profile, Tab, TabState},
    from_c,
    host::permissions_of,
    menu::{present_menu, Row},
    native,
    page::CorePage,
    PageKind, Shared, Snapshot,
};
use crate::{ChromeTabs, DialogKind, DownloadState, Downloads, PageEvent};

/// How long a tab Chrome opened waits for the UI to take it before it is closed.
pub(super) const ADOPTION_EXPIRY: Duration = Duration::from_secs(60);

/// A tab Chrome opened that baa has not taken yet.
struct Pending {
    tab: *mut Tab,
    profile: *mut Profile,
    url: String,
    /// Tells this entry from a later tab at the same address, for its timer.
    seq: u64,
}

/// What `WebHost::fetch` waits with.
type FetchDone = Box<dyn FnOnce(Option<String>)>;

/// A tab baa shows: its shared half, and its gpui entity unless it has none
/// (the settings sheet), when only its view is followed.
pub(super) struct Live {
    pub(super) shared: Rc<Shared>,
    pub(super) page: Option<WeakEntity<CorePage>>,
}

/// The callback side. `ctx` of every callback points at it; it lives for the
/// process. Callbacks may run inside any `baa_*` call, so they only record
/// plain data here and queue the rest for the next turn of gpui's executor.
pub(super) struct Dispatch {
    pub(super) core: &'static Core,
    /// Before [`CoreRuntime::start`] there is none and callbacks do nothing;
    /// Chrome makes no tabs of its own then (`--no-startup-window`).
    pub(super) app: RefCell<Option<AsyncApp>>,
    pub(super) tabs: RefCell<HashMap<*mut Tab, Live>>,
    pending: RefCell<Vec<Pending>>,
    pub(super) chrome_tabs: RefCell<Option<WeakEntity<ChromeTabs>>>,
    pub(super) downloads: RefCell<Option<WeakEntity<Downloads>>>,
    pub(super) fetches: RefCell<HashMap<u64, FetchDone>>,
    pub(super) next_fetch: Cell<u64>,
    next_seq: Cell<u64>,
    /// `--user-data-dir`.
    pub(super) root: OnceCell<PathBuf>,
}

impl Dispatch {
    pub(super) fn new(core: &'static Core) -> Self {
        Self {
            core,
            app: RefCell::default(),
            tabs: RefCell::default(),
            pending: RefCell::default(),
            chrome_tabs: RefCell::default(),
            downloads: RefCell::default(),
            fetches: RefCell::default(),
            next_fetch: Cell::new(1),
            next_seq: Cell::new(0),
            root: OnceCell::new(),
        }
    }

    pub(super) fn queue(&self, f: impl FnOnce(&mut AsyncApp) + 'static) {
        let app = self.app.borrow().clone();
        if let Some(app) = app {
            app.spawn(async move |cx| f(cx)).detach();
        }
    }

    /// The gpui entity of `tab`; none for an unknown tab or one without.
    pub(super) fn page_of(&self, tab: *mut Tab) -> Option<WeakEntity<CorePage>> {
        self.tabs.borrow().get(&tab)?.page.clone()
    }

    /// Runs `f` on the page of `tab` next turn; nothing for an unknown tab.
    pub(super) fn on_page(&self, tab: *mut Tab, f: impl FnOnce(&mut CorePage, &mut Context<CorePage>) + 'static) {
        if let Some(page) = self.page_of(tab) {
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
        let opener = self.page_of(opener);
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
        let live = self.tabs.borrow().get(&tab).map(|l| (l.shared.clone(), l.page.clone()));
        let Some((shared, page)) = live else { return };
        let view_changed = shared.container.swap_page(snapshot.view);
        if let Some(page) = page {
            self.queue(move |cx| {
                page.update(cx, |page, cx| page.apply(snapshot, view_changed, cx)).ok();
            });
        }
    }

    /// A download started or changed. The first sight of one also tells the
    /// page it came from.
    fn download(&'static self, tab: *mut Tab, download: crate::Download) {
        let page = self.page_of(tab);
        self.queue(move |cx| {
            let list = self.downloads.borrow().clone();
            let new = list.and_then(|list| {
                list.update(cx, |list, cx| {
                    let new = list.items.iter().all(|d| d.id != download.id);
                    list.upsert(download);
                    cx.notify();
                    new
                })
                .ok()
            });
            if let (Some(true), Some(page)) = (new, page) {
                page.update(cx, |_, cx| cx.emit(PageEvent::DownloadStarted)).ok();
            }
        });
    }

    /// A menu is open in the page; a page that is gone gets it closed.
    fn context_menu(&'static self, tab: *mut Tab, id: u64, x: f32, y: f32, rows: Vec<Row>) {
        let live = self.tabs.borrow().get(&tab).map(|l| (l.shared.clone(), l.page.clone()));
        self.queue(move |cx| match live {
            Some((_, Some(page))) => {
                if page.update(cx, |page, cx| page.show_context_menu(id, x, y, rows, cx)).is_err() {
                    (self.core.context_menu_done)(id, -1);
                }
            }
            // The sheet has no entity: its menu opens over its view, and Inspect
            // goes to Chrome's own window like a popup's.
            Some((shared, None)) => present_menu(shared, id, x, y, rows, PageKind::Popup, || {}),
            None => (self.core.context_menu_done)(id, -1),
        });
    }

    /// The profile is going away with its tabs, which Chrome may not report.
    pub(super) fn forget_profile(&self, profile: *mut Profile) {
        self.pending.borrow_mut().retain(|p| p.profile != profile);
        let gone: Vec<_> =
            self.tabs.borrow().iter().filter(|(_, l)| l.shared.profile == profile).map(|(tab, _)| *tab).collect();
        for tab in gone {
            let live = self.tabs.borrow_mut().remove(&tab);
            if let Some(live) = live {
                live.shared.release();
            }
        }
    }

    pub(super) fn take_pending(&self, profile: *mut Profile, url: &str) -> Option<*mut Tab> {
        let mut pending = self.pending.borrow_mut();
        let at = pending.iter().position(|p| p.profile == profile && p.url == url)?;
        Some(pending.remove(at).tab)
    }

    fn tab_closed(&self, tab: *mut Tab) {
        self.pending.borrow_mut().retain(|p| p.tab != tab);
        let Some(live) = self.tabs.borrow_mut().remove(&tab) else { return };
        // Now, not next turn: the handle is dead, its address may be reused, and
        // its view is freed once this callback returns.
        live.shared.release();
        // The sheet has no entity to tell; it notices through `Shared::tab`.
        let Some(page) = live.page else { return };
        if (self.core.is_quitting)() {
            // ⌘Q closes every tab; the session must keep them.
            return;
        }
        self.queue(move |cx| {
            page.update(cx, |_, cx| cx.emit(PageEvent::Closed)).ok();
        });
    }
}

pub(super) fn callbacks() -> Callbacks {
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
        let d = unsafe { dispatch(ctx) };
        let kind = dialog_kind(kind);
        let quitting = matches!(kind, DialogKind::BeforeUnload { .. }) && (d.core.is_quitting)();
        if quitting || d.page_of(tab).is_none() {
            // ⌘Q waits on a leave-site question nobody would answer for a hidden
            // tab; a page with no UI (the sheet, a tab not adopted yet) cannot
            // ask. Answered next turn: Chrome is inside its dialog code now,
            // and may not have recorded the id yet.
            d.queue(move |_| (d.core.dialog_answer)(id, quitting, c"".as_ptr()));
            return;
        }
        d.on_page(tab, move |page, cx| {
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
    extern "C" fn permission(ctx: *mut c_void, tab: *mut Tab, id: u64, origin: *const c_char, kinds: u32) {
        let origin = unsafe { from_c(origin) };
        let kinds = permissions_of(kinds);
        let d = unsafe { dispatch(ctx) };
        if d.page_of(tab).is_none() {
            d.queue(move |_| (d.core.permission_answer)(id, false));
            return;
        }
        d.on_page(tab, move |_, cx| cx.emit(PageEvent::PermissionRequested { id, origin, kinds }));
    }
    extern "C" fn permission_closed(ctx: *mut c_void, tab: *mut Tab, id: u64) {
        unsafe { dispatch(ctx) }.on_page(tab, move |_, cx| cx.emit(PageEvent::PermissionDismissed(id)));
    }
    extern "C" fn context_menu(
        ctx: *mut c_void,
        tab: *mut Tab,
        id: u64,
        x: f32,
        y: f32,
        items: *const ffi::MenuItem,
        count: usize,
    ) {
        unsafe { dispatch(ctx) }.context_menu(tab, id, x, y, Row::from_c(items, count));
    }
    extern "C" fn download(ctx: *mut c_void, tab: *mut Tab, download: *const ffi::Download) {
        let download = unsafe { &*download };
        let download = crate::Download {
            id: download.id,
            url: unsafe { from_c(download.url) },
            path: unsafe { from_c(download.path) },
            received_bytes: download.received_bytes,
            total_bytes: download.total_bytes,
            state: match download.state {
                ffi::DOWNLOAD_IN_PROGRESS => DownloadState::InProgress,
                ffi::DOWNLOAD_COMPLETE => DownloadState::Complete,
                _ => DownloadState::Canceled,
            },
        };
        unsafe { dispatch(ctx) }.download(tab, download);
    }
    extern "C" fn resized(ctx: *mut c_void, tab: *mut Tab, width: c_int, height: c_int) {
        let size = gpui::size(gpui::px(width as f32), gpui::px(height as f32));
        unsafe { dispatch(ctx) }.on_page(tab, move |_, cx| cx.emit(PageEvent::PreferredSize(size)));
    }
    extern "C" fn page_message(ctx: *mut c_void, tab: *mut Tab, message: *const c_char) {
        let message = unsafe { from_c(message) };
        unsafe { dispatch(ctx) }.on_page(tab, move |page, cx| {
            // Only the DevTools front-end's dock picker speaks this.
            if let Some(side) = message.strip_prefix("devtools-dock:").filter(|_| page.shared.kind == PageKind::DevTools) {
                cx.emit(PageEvent::DevToolsDock(side.into()));
            }
        });
    }
    extern "C" fn fetch_done(ctx: *mut c_void, id: u64, ok: bool, body: *const c_char, len: usize) {
        let body = (ok && !body.is_null())
            .then(|| String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(body.cast::<u8>(), len) }).into_owned());
        let d = unsafe { dispatch(ctx) };
        d.queue(move |_| {
            let done = d.fetches.borrow_mut().remove(&id);
            if let Some(done) = done {
                done(body);
            }
        });
    }
    // ponytail: WP-C routes these to the pending-call maps (as `fetch_done`).
    extern "C" fn clear_done(_: *mut c_void, _: u64) {}
    extern "C" fn load_unpacked_done(_: *mut c_void, _: u64, _: bool, _: *const c_char) {}
    Callbacks {
        key,
        tab_created,
        tab_state,
        tab_favicon,
        dialog,
        dialog_closed,
        tab_closed,
        permission,
        permission_closed,
        context_menu,
        download,
        resized,
        page_message,
        fetch_done,
        clear_done,
        load_unpacked_done,
    }
}

fn dialog_kind(raw: c_int) -> DialogKind {
    match raw {
        ffi::DIALOG_CONFIRM => DialogKind::Confirm,
        ffi::DIALOG_PROMPT => DialogKind::Prompt,
        ffi::DIALOG_BEFOREUNLOAD => DialogKind::BeforeUnload { is_reload: false },
        ffi::DIALOG_BEFOREUNLOAD_RELOAD => DialogKind::BeforeUnload { is_reload: true },
        _ => DialogKind::Alert,
    }
}
