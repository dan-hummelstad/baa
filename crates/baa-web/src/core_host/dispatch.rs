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
    ffi::{self, Callbacks, Core, Profile, TabId, TabState},
    from_c,
    host::permissions_of,
    menu::{present_menu, Row},
    native,
    page::CorePage,
    PageKind, Shared, Snapshot,
};
use crate::{Callback, ChromeTabs, DialogKind, DownloadState, Downloads, PageEvent, SuggestUpdate, Suggestion};

/// How long a tab Chrome opened waits for the UI to take it before it is closed.
pub(super) const ADOPTION_EXPIRY: Duration = Duration::from_secs(60);

/// A tab Chrome opened that baa has not taken yet.
struct Pending {
    tab: TabId,
    profile: *mut Profile,
    url: String,
}

/// Calls Chrome has not answered yet, by the id it was given.
pub(super) struct Waiting<T> {
    next: Cell<u64>,
    by_id: RefCell<HashMap<u64, T>>,
}

impl<T> Waiting<T> {
    fn new() -> Self {
        Self { next: Cell::new(1), by_id: RefCell::default() }
    }

    pub(super) fn add(&self, waiting: T) -> u64 {
        let id = self.next.replace(self.next.get() + 1);
        self.by_id.borrow_mut().insert(id, waiting);
        id
    }

    /// An id nobody waits for (answered already, or never asked) gives none.
    fn take(&self, id: u64) -> Option<T> {
        self.by_id.borrow_mut().remove(&id)
    }
}

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
    pub(super) tabs: RefCell<HashMap<TabId, Live>>,
    pending: RefCell<Vec<Pending>>,
    pub(super) chrome_tabs: RefCell<Option<WeakEntity<ChromeTabs>>>,
    pub(super) downloads: RefCell<Option<WeakEntity<Downloads>>>,
    pub(super) clears: Waiting<Callback<()>>,
    pub(super) unpacked: Waiting<Callback<Result<String, String>>>,
    /// The one query whose results still count: its id and whom to tell.
    autocomplete: RefCell<Option<(u64, SuggestUpdate)>>,
    next_autocomplete: Cell<u64>,
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
            clears: Waiting::new(),
            unpacked: Waiting::new(),
            autocomplete: RefCell::default(),
            next_autocomplete: Cell::new(1),
            root: OnceCell::new(),
        }
    }

    pub(super) fn queue(&self, f: impl FnOnce(&mut AsyncApp) + 'static) {
        let app = self.app.borrow().clone();
        if let Some(app) = app {
            app.spawn(async move |cx| f(cx)).detach();
        }
    }

    /// Next turn, so the answer never lands inside the call that asked. An id
    /// nobody waits for is ignored.
    pub(super) fn finish_clear(&'static self, id: u64) {
        self.queue(move |cx| {
            let done = self.clears.take(id);
            if let Some(done) = done {
                cx.update(|cx| done((), cx));
            }
        });
    }

    pub(super) fn finish_unpacked(&'static self, id: u64, result: Result<String, String>) {
        self.queue(move |cx| {
            let done = self.unpacked.take(id);
            if let Some(done) = done {
                cx.update(|cx| done(result, cx));
            }
        });
    }

    /// A new query replaces the last; returns the id to give Chrome.
    pub(super) fn autocomplete_started(&self, update: SuggestUpdate) -> u64 {
        let id = self.next_autocomplete.replace(self.next_autocomplete.get() + 1);
        *self.autocomplete.borrow_mut() = Some((id, update));
        id
    }

    /// Matches for query `id`, next turn. Late results of a replaced query are
    /// dropped, and so is the callback once Chrome is done.
    fn autocompleted(&'static self, id: u64, list: Vec<Suggestion>, done: bool) {
        self.queue(move |cx| {
            // Out of the cell while it runs: `update` may start the next query.
            let current = self.autocomplete.borrow_mut().take_if(|(current, _)| *current == id);
            let Some((_, mut update)) = current else { return };
            cx.update(|cx| update(list, cx));
            if !done {
                // A query `update` started wins.
                self.autocomplete.borrow_mut().get_or_insert((id, update));
            }
        });
    }

    /// The gpui entity of `tab`; none for an unknown tab or one without.
    pub(super) fn page_of(&self, tab: TabId) -> Option<WeakEntity<CorePage>> {
        self.tabs.borrow().get(&tab)?.page.clone()
    }

    /// Runs `f` on the page of `tab` next turn; nothing for an unknown tab.
    pub(super) fn on_page(&self, tab: TabId, f: impl FnOnce(&mut CorePage, &mut Context<CorePage>) + 'static) {
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
        tab: TabId,
        opener: TabId,
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
        self.pending.borrow_mut().push(Pending { tab, profile, url: url.clone() });
        let opener = self.page_of(opener);
        self.queue(move |cx| {
            let requested = PageEvent::OpenRequested { url: url.clone(), from_user_gesture, background };
            let delivered = opener.and_then(|page| page.update(cx, |_, cx| cx.emit(requested)).ok());
            if delivered.is_none() {
                self.chrome_tab(url, cx);
            } else if !background {
                // The UI takes it (create_page) while handling the event, or
                // not at all: a tab nobody shows must not linger.
                cx.update(|cx| cx.defer(move |_| self.expire(tab)));
            }
        });
        app.spawn(async move |cx| {
            cx.background_executor().timer(ADOPTION_EXPIRY).await;
            self.expire(tab);
        })
        .detach();
    }

    /// Closes the pending tab unless it was adopted meanwhile; ids are never
    /// reused, so a late timer cannot hit a later tab.
    fn expire(&self, tab: TabId) {
        let mut pending = self.pending.borrow_mut();
        if let Some(at) = pending.iter().position(|p| p.tab == tab) {
            let expired = pending.remove(at);
            drop(pending);
            (self.core.tab_close)(expired.tab);
        }
    }

    /// A tab's snapshot. A changed view is swapped in now: Chrome frees the
    /// old one as soon as this callback returns.
    fn tab_state(&self, tab: TabId, snapshot: Snapshot) {
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
    fn download(&'static self, tab: TabId, download: crate::Download) {
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
    fn context_menu(&'static self, tab: TabId, id: u64, x: f32, y: f32, rows: Vec<Row>) {
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

    pub(super) fn take_pending(&self, profile: *mut Profile, url: &str) -> Option<TabId> {
        let mut pending = self.pending.borrow_mut();
        let at = pending.iter().position(|p| p.profile == profile && p.url == url)?;
        Some(pending.remove(at).tab)
    }

    fn tab_closed(&self, tab: TabId, reason: c_int) {
        self.pending.borrow_mut().retain(|p| p.tab != tab);
        let Some(live) = self.tabs.borrow_mut().remove(&tab) else { return };
        // Now, not next turn: its view is freed once this callback returns.
        live.shared.release();
        // The sheet has no entity to tell; it notices through `Shared::tab`.
        let Some(page) = live.page else { return };
        // The UI closed it itself, or ⌘Q closes every tab and the session must keep them.
        if matches!(reason, ffi::CLOSED_BY_APP | ffi::CLOSED_BY_QUIT) {
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
        tab: TabId,
        opener: TabId,
        profile: *mut Profile,
        url: *const c_char,
        background: bool,
        user_gesture: bool,
    ) {
        unsafe { dispatch(ctx).tab_created(tab, opener, profile, super::skin::to_baa(from_c(url)), background, user_gesture) }
    }
    extern "C" fn tab_state(ctx: *mut c_void, tab: TabId, state: *const TabState) {
        let snapshot = Snapshot::from_c(unsafe { &*state });
        unsafe { dispatch(ctx) }.tab_state(tab, snapshot);
    }
    extern "C" fn tab_favicon(ctx: *mut c_void, tab: TabId, icon_url: *const c_char, png: *const u8, len: usize) {
        let url = unsafe { from_c(icon_url) };
        let png = Arc::new(if png.is_null() { Vec::new() } else { unsafe { std::slice::from_raw_parts(png, len) }.to_vec() });
        unsafe { dispatch(ctx) }.on_page(tab, move |_, cx| cx.emit(PageEvent::FaviconImage { url, png }));
    }
    extern "C" fn dialog(ctx: *mut c_void, tab: TabId, id: u64, dialog: *const ffi::Dialog) {
        let dialog = unsafe { &*dialog };
        let d = unsafe { dispatch(ctx) };
        let kind = dialog_kind(dialog.kind);
        let quitting = matches!(kind, DialogKind::BeforeUnload { .. }) && (d.core.is_quitting)();
        if quitting || d.page_of(tab).is_none() {
            // ⌘Q waits on a leave-site question nobody would answer for a hidden
            // tab; a page with no UI (the sheet, a tab not adopted yet) cannot
            // ask. Answered next turn: Chrome is inside its dialog code now,
            // and may not have recorded the id yet.
            d.queue(move |_| d.core.answer_ok(id, quitting));
            return;
        }
        let dialog = unsafe {
            crate::Dialog {
                id,
                kind,
                message: from_c(dialog.message),
                default_text: from_c(dialog.default_text),
                title: from_c(dialog.title),
                ok_label: from_c(dialog.ok_label),
                cancel_label: from_c(dialog.cancel_label),
                check_label: from_c(dialog.check_label),
            }
        };
        d.on_page(tab, move |_, cx| cx.emit(PageEvent::Dialog(dialog)));
    }
    extern "C" fn tab_closed(ctx: *mut c_void, tab: TabId, reason: c_int) {
        unsafe { dispatch(ctx) }.tab_closed(tab, reason);
    }
    extern "C" fn permission(ctx: *mut c_void, tab: TabId, id: u64, origin: *const c_char, kinds: u32) {
        let origin = unsafe { from_c(origin) };
        let kinds = permissions_of(kinds);
        let d = unsafe { dispatch(ctx) };
        if d.page_of(tab).is_none() {
            d.queue(move |_| d.core.answer_ok(id, false));
            return;
        }
        d.on_page(tab, move |_, cx| cx.emit(PageEvent::PermissionRequested { id, origin, kinds }));
    }
    extern "C" fn request_closed(ctx: *mut c_void, tab: TabId, id: u64) {
        unsafe { dispatch(ctx) }.on_page(tab, move |_, cx| cx.emit(PageEvent::AskClosed(id)));
    }
    extern "C" fn context_menu(
        ctx: *mut c_void,
        tab: TabId,
        id: u64,
        x: f32,
        y: f32,
        items: *const ffi::MenuItem,
        count: usize,
    ) {
        unsafe { dispatch(ctx) }.context_menu(tab, id, x, y, Row::from_c(items, count));
    }
    extern "C" fn download(ctx: *mut c_void, tab: TabId, download: *const ffi::Download) {
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
    extern "C" fn resized(ctx: *mut c_void, tab: TabId, width: c_int, height: c_int) {
        let size = gpui::size(gpui::px(width as f32), gpui::px(height as f32));
        unsafe { dispatch(ctx) }.on_page(tab, move |_, cx| cx.emit(PageEvent::PreferredSize(size)));
    }
    extern "C" fn devtools_dock(ctx: *mut c_void, front: TabId, side: c_int) {
        let side = match side {
            ffi::DOCK_BOTTOM => "bottom",
            ffi::DOCK_LEFT => "left",
            ffi::DOCK_UNDOCKED => "undocked",
            _ => "right",
        };
        unsafe { dispatch(ctx) }.on_page(front, move |_, cx| cx.emit(PageEvent::DevToolsDock(side.into())));
    }
    extern "C" fn fullscreen(ctx: *mut c_void, tab: TabId, on: bool) {
        unsafe { dispatch(ctx) }.on_page(tab, move |_, cx| cx.emit(PageEvent::Fullscreen(on)));
    }
    extern "C" fn find_result(ctx: *mut c_void, tab: TabId, active: c_int, count: c_int) {
        let (active, total) = (active.max(0) as u32, count.max(0) as u32);
        unsafe { dispatch(ctx) }.on_page(tab, move |_, cx| cx.emit(PageEvent::FindResult { active, total }));
    }
    extern "C" fn tab_group(ctx: *mut c_void, tab: TabId, group: u64, title: *const c_char) {
        let title = unsafe { from_c(title) };
        let group = (group != 0).then_some(group);
        unsafe { dispatch(ctx) }.on_page(tab, move |_, cx| cx.emit(PageEvent::GroupChanged { group, title }));
    }
    extern "C" fn autocomplete(ctx: *mut c_void, id: u64, matches: *const ffi::Match, count: usize, done: bool) {
        // Copied now: Chrome frees the array when this returns.
        let matches = if matches.is_null() { &[] } else { unsafe { std::slice::from_raw_parts(matches, count) } };
        let list = matches
            .iter()
            .map(|m| unsafe {
                Suggestion { url: from_c(m.url), text: from_c(m.contents), description: from_c(m.description), search: m.search }
            })
            .collect();
        unsafe { dispatch(ctx) }.autocompleted(id, list, done);
    }
    extern "C" fn clear_done(ctx: *mut c_void, id: u64) {
        unsafe { dispatch(ctx) }.finish_clear(id);
    }
    extern "C" fn load_unpacked_done(ctx: *mut c_void, id: u64, ok: bool, message: *const c_char) {
        let message = unsafe { from_c(message) };
        unsafe { dispatch(ctx) }.finish_unpacked(id, if ok { Ok(message) } else { Err(message) });
    }
    Callbacks {
        key,
        tab_created,
        tab_state,
        tab_favicon,
        dialog,
        tab_closed,
        permission,
        request_closed,
        context_menu,
        download,
        resized,
        devtools_dock,
        fullscreen,
        find_result,
        tab_group,
        autocomplete,
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
        ffi::DIALOG_AUTH => DialogKind::Auth,
        ffi::DIALOG_EXTERNAL_PROTOCOL => DialogKind::ExternalProtocol,
        ffi::DIALOG_UNRESPONSIVE => DialogKind::Unresponsive,
        ffi::DIALOG_EXTENSION_INSTALL => DialogKind::ExtensionInstall,
        _ => DialogKind::Alert,
    }
}
