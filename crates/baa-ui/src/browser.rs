//! Root window view. Owns the `Session` (baa-model is the source of truth for
//! spaces/tabs), the live pages, and every §8 action handler.

use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use baa_model::{now, DownloadRecord, FolderId, Session, SidebarSide, Space, SpaceId, Store, Tab, TabId, Zone};
use baa_web::{PageEvent, PageHandle, WebHost};
use gpui::{
    anchored, deferred, div, linear_color_stop, linear_gradient, point, prelude::*, px,
    relative, rgb, AnyElement, AnyWindowHandle, BoxShadow, ClipboardItem, Context, Entity,
    Animation, AnimationExt, Div, ElementId, Transformation, FocusHandle, Hsla, KeyDownEvent, MouseButton, Pixels, Point, Rgba,
    SharedString, Stateful, Subscription, Window,
};

use crate::{
    actions::*,
    command_bar::{self, Bar, Mode},
    icons::icon,
    input::{Key, TextField},
    lil_baa::LilBaa,
    motion::{self, Feel, Motion, Spring},
    theme::{fade, lerp_oklch, mix, space_gradient, surface, Theme},
};

pub(crate) const SIDEBAR_WIDTH: f32 = 258.0;
/// The window-wide row with the traffic lights, navigation, URL and extensions.
pub(crate) const HEADER_H: f32 = 38.0;
/// Gap between the window edge and the page card (and the floating sidebar).
pub(crate) const INSET: f32 = 9.0;
/// The page card's corner radius; the native page view is rounded to match.
pub(crate) const PAGE_RADIUS: f32 = 8.0;
const PEEK_RADIUS: f32 = 12.0;
/// Collapsed sidebar (Arc): hot zone at the window edge, and how long the
/// revealed card lingers after the pointer leaves: about the moment it does,
/// with a frame or two of grace for crossing from the edge onto the card.
const REVEAL_EDGE: f32 = 8.0;
/// How far past the revealed card the pointer may stray before it closes.
const REVEAL_SLACK: f32 = 24.0;
const REVEAL_LINGER: std::time::Duration = std::time::Duration::from_millis(40);
/// Chrome's zoom ladder, as factors (`PageHandle::set_zoom` takes 1.0 = 100%).
const ZOOM_LEVELS: [f64; 15] = [
    0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
];
/// Space gradients (top, bottom): Personal, Work, Studio, Night from the mocks,
/// then a few more in the same key.
pub(crate) const PALETTE: [(u32, u32); 7] = [
    (0xDCCFFB, 0xA592EC),
    (0xF3D07F, 0xE8808E),
    (0xC9F0E0, 0x8FD8A7),
    (0x34304A, 0x16151E),
    (0xCFE3FB, 0x8FB4EC),
    (0xFBE0CF, 0xECA88F),
    (0xF6D1E8, 0xD98FC2),
];
const ICONS: [&str; 7] = ["🐑", "🌿", "🔥", "🌊", "⭐", "🍋", "🪐"];
const INCOGNITO: (u32, u32) = (0x4A4856, 0x2A2932);

type MenuAction<H> = Box<dyn Fn(&mut Browser<H>, &mut Window, &mut Context<Browser<H>>)>;

pub(crate) struct ContextMenu {
    pub at: Point<Pixels>,
    pub kind: MenuKind,
}

pub(crate) enum MenuKind {
    Page {
        tab: TabId,
        link: Option<String>,
        selection: Option<String>,
    },
    /// Right-clicks in the sidebar.
    Tab(TabId),
    /// A finished download in the Library.
    Download { path: String, url: String },
    /// The Library's Downloads filter.
    LibraryFilter,
    Folder(FolderId),
    Sidebar,
}

pub struct Browser<H: WebHost> {
    pub(crate) host: H,
    /// `None` after an unreadable session: run on a fresh one without overwriting the file.
    store: Option<Store>,
    pub(crate) session: Session,
    pub(crate) settings: baa_model::Settings,
    pub(crate) pages: HashMap<TabId, (Entity<H::Page>, Subscription)>,
    zoom: HashMap<TabId, f64>,
    pub(crate) bar: Option<Bar>,
    pub(crate) find: Option<TextField>,
    pub(crate) menu: Option<ContextMenu>,
    /// A link previewed over the page (Peek), or an extension's popup.
    pub(crate) peek: Option<PeekPage<H::Page>>,
    /// The Space name being edited in place.
    pub(crate) renaming: Option<TextField>,
    /// Set when `renaming` edits this folder's name instead of the Space's.
    pub(crate) renaming_folder: Option<FolderId>,
    /// Folders opening or closing: 0 closed, 1 open. Absent = at rest.
    pub(crate) folder_motion: HashMap<FolderId, Spring>,
    /// Hover, press and selection springs of everything the pointer touches.
    pub(crate) motion: Motion,
    /// Where each tab's row was at the last sync, to animate rows in and out.
    placements: Option<HashMap<TabId, Placement>>,
    /// Rows that just left a list, collapsing where they were.
    pub(crate) ghosts: Vec<Ghost>,
    shown_tab: Option<TabId>,
    /// Copies of the URL so far, so only the latest one's check goes away.
    copies: u64,
    /// The command bar on its way out.
    pub(crate) closing_bar: Option<Bar>,
    /// Favicons by URL, fetched by the pages themselves.
    pub(crate) favicons: HashMap<String, std::sync::Arc<gpui::Image>>,
    /// The active Space's extensions, for the toolbar.
    pub(crate) extensions: Vec<baa_web::Extension>,
    /// What each tab's page is waiting on the user for.
    pub(crate) asks: HashMap<TabId, crate::prompts::Asks>,
    /// Tabs playing sound, and tabs the user muted.
    pub(crate) audible: HashSet<TabId>,
    pub(crate) muted: HashSet<TabId>,
    /// Header popovers: the download list, the site's settings.
    pub(crate) popover: Option<crate::prompts::Popover>,
    /// Downloads the user cleared from the list.
    pub(crate) cleared_downloads: HashSet<u32>,
    pub(crate) downloads_seen: usize,
    pub(crate) settings_open: bool,
    /// The Library, shown in the sidebar instead of the Space.
    pub(crate) library: Option<crate::library::Library>,
    pub(crate) library_focus: FocusHandle,
    /// Finished downloads, newest first, kept across launches.
    pub(crate) saved_downloads: Vec<DownloadRecord>,
    /// This session's downloads already saved.
    pub(crate) recorded_downloads: HashSet<u32>,
    /// A short confirmation at the bottom of the window, and how many so far.
    pub(crate) toast: Option<SharedString>,
    pub(crate) toasts: u64,
    _downloads: Subscription,
    /// Back/forward pressed and held: which way, and when.
    pub(crate) nav_press: Option<(bool, Instant)>,
    pub(crate) history_menu: Option<HistoryMenu>,
    /// Two-finger swipe across the chrome, in sidebar widths, while one is on.
    swipe: Option<f32>,
    window: AnyWindowHandle,
    focus: FocusHandle,
    /// Shared by the command bar and find bar; only one is open at a time.
    pub(crate) input_focus: FocusHandle,
    pub(crate) sidebar_open: bool,
    pub(crate) sidebar_w: Spring,
    /// Collapsed sidebar shown over the page: 0 hidden, 1 shown.
    reveal: Spring,
    /// Pointer over the edge zone / over the card.
    /// The pointer is where the collapsed sidebar should stay out.
    reveal_hover: bool,
    /// Space-switch slide, in sidebar widths (−1…1).
    pub(crate) slide: Spring,
    fade: Spring,
    gradient_from: (Rgba, Rgba),
    gradient_to: (Rgba, Rgba),
    shown_space: Option<SpaceId>,
    last_frame: Instant,
}

impl<H: WebHost> Browser<H> {
    pub fn new(host: H, store: Store, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = store.load_settings().unwrap_or_else(|e| {
            log::error!("settings unreadable, using defaults: {e}");
            Default::default()
        });
        let (mut session, store) = match store.load_session() {
            Ok(session) => (session, Some(store)),
            Err(e) => {
                log::error!("session unreadable; this run will not be saved: {e}");
                (Session::first_run(), None)
            }
        };
        // ponytail: auto-archive runs at launch and on Space switch only; a timer comes with milestone 4.
        session.archive_expired(now(), settings.archive_after_hours);
        // Spaces from before gradient themes get the palette, in order.
        let ids: Vec<SpaceId> = session.spaces().iter().map(|s| s.id).collect();
        for (i, id) in ids.into_iter().enumerate() {
            if let Some(space) = session.space_mut(id).filter(|s| s.accent_to.is_none()) {
                let (top, bottom) = PALETTE[i % PALETTE.len()];
                (space.accent, space.accent_to) = (top, Some(bottom));
            }
        }
        // gpui's flag also drives `with_animation`; the app shell sets it from the OS.
        if settings.reduce_motion {
            cx.set_reduce_motion(true);
        }
        let favicons = store
            .as_ref()
            .and_then(|s| s.load_favicons().ok())
            .unwrap_or_default()
            .into_iter()
            .map(|(url, png)| (url, std::sync::Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png))))
            .collect();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let downloads = cx.observe(&host.downloads(), |this: &mut Self, list, cx| {
            let items = list.read(cx).items.clone();
            this.remember_downloads(&items);
            this.downloads_changed(items.len(), cx)
        });
        let saved_downloads = store.as_ref().and_then(|s| s.downloads().ok()).unwrap_or_default();
        let mut this = Self {
            host,
            store,
            shown_space: session.active_space(),
            session,
            settings,
            pages: HashMap::new(),
            zoom: HashMap::new(),
            bar: None,
            find: None,
            menu: None,
            peek: None,
            renaming: None,
            renaming_folder: None,
            folder_motion: HashMap::new(),
            motion: Motion::default(),
            placements: None,
            ghosts: Vec::new(),
            shown_tab: None,
            closing_bar: None,
            copies: 0,
            favicons,
            extensions: Vec::new(),
            asks: HashMap::new(),
            audible: HashSet::new(),
            muted: HashSet::new(),
            popover: None,
            cleared_downloads: HashSet::new(),
            downloads_seen: 0,
            settings_open: false,
            library: None,
            library_focus: cx.focus_handle(),
            saved_downloads,
            recorded_downloads: HashSet::new(),
            toast: None,
            toasts: 0,
            _downloads: downloads,
            nav_press: None,
            history_menu: None,
            swipe: None,
            window: window.window_handle(),
            focus,
            input_focus: cx.focus_handle(),
            sidebar_open: true,
            sidebar_w: Spring::new(SIDEBAR_WIDTH),
            reveal: Spring::quick(0.0),
            reveal_hover: false,
            slide: Spring::new(0.0),
            fade: Spring::new(1.0),
            gradient_from: (rgb(PALETTE[0].0), rgb(PALETTE[0].1)),
            gradient_to: (rgb(PALETTE[0].0), rgb(PALETTE[0].1)),
            last_frame: Instant::now(),
        };
        this.sync(window, cx);
        // Launch shows the Space's colour, not a crossfade from the placeholder.
        this.fade.snap(1.0);
        this
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub(crate) fn active_space(&self) -> Option<&Space> {
        self.session.space(self.session.active_space()?)
    }

    pub(crate) fn active_tab(&self) -> Option<TabId> {
        self.active_space()?.active_tab
    }

    pub(crate) fn active_page(&self) -> Option<&Entity<H::Page>> {
        self.pages.get(&self.active_tab()?).map(|(p, _)| p)
    }

    pub(crate) fn with_page(&self, cx: &mut Context<Self>, f: impl FnOnce(&H::Page)) {
        if let Some(page) = self.active_page() {
            f(page.read(cx));
        }
    }

    pub(crate) fn side(&self) -> SidebarSide {
        self.active_space()
            .and_then(|s| s.sidebar_side)
            .unwrap_or(self.settings.sidebar_side)
    }

    pub(crate) fn store(&self) -> Option<&Store> {
        self.store.as_ref()
    }

    pub(crate) fn save_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(Err(e)) = self.store.as_ref().map(|s| s.save_settings(&self.settings)) {
            log::error!("saving settings: {e}");
        }
        cx.set_reduce_motion(self.settings.reduce_motion);
        cx.notify();
    }

    pub(crate) fn save(&mut self) {
        if let Some(Err(e)) = self.store.as_ref().map(|s| s.save_session(&self.session)) {
            log::error!("saving session: {e}");
        }
    }

    /// Single place that reconciles live pages and motion with the session after any change.
    pub(crate) fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let space = self.session.active_space();
        if space != self.shown_space {
            let index = |id| self.session.spaces().iter().position(|s| Some(s.id) == id);
            let forward = index(space) > index(self.shown_space);
            let mirrored = self.side() == SidebarSide::Right;
            self.slide
                .snap(if forward != mirrored { 1.0 } else { -1.0 });
            self.slide.set_target(0.0);
            if let Some(i) = index(space) {
                let x = i as f32 * SPACE_DOT;
                match self.shown_space {
                    Some(_) => self.motion.to(("space-pill", 0), Feel::Glide, x),
                    None => self.motion.from(("space-pill", 0), Feel::Glide, x, x),
                }
            }
            for s in [self.shown_space, space].into_iter().flatten() {
                self.motion.to(("space-sel", s.0), Feel::Glide, (Some(s) == space) as u8 as f32);
            }
            self.shown_space = space;
            self.session
                .archive_expired(now(), self.settings.archive_after_hours);
        }
        let gradient = self
            .active_space()
            .map_or(self.gradient_to, |s| space_gradient(s.accent, s.accent_to));
        if gradient != self.gradient_to {
            self.gradient_from = self.gradient(); // mid-flight colours, so rapid switches never jump
            self.gradient_to = gradient;
            self.fade.snap(0.0);
            self.fade.set_target(1.0);
        }

        let session = &self.session;
        self.pages.retain(|id, _| session.tab(*id).is_some());
        let pages = &self.pages;
        self.asks.retain(|id, _| pages.contains_key(id));
        self.audible.retain(|id| pages.contains_key(id));
        self.muted.retain(|id| pages.contains_key(id));
        let active = self.active_tab();
        if active != self.shown_tab {
            for (id, on) in [(self.shown_tab, 0.0), (active, 1.0)] {
                if let Some(id) = id {
                    self.motion.to(("sel", id.0), Feel::Glide, on);
                }
            }
            self.shown_tab = active;
        }
        self.track_rows();
        if let Some(id) = active.filter(|id| !self.pages.contains_key(id)) {
            self.create_page(id, window, cx);
        }
        for (id, (page, _)) in &self.pages {
            page.read(cx).set_visible(Some(*id) == active);
        }
        if self.bar.is_none() && self.find.is_none() && self.peek.is_none() && self.renaming.is_none() {
            match self.active_dialog() {
                Some(_) => window.focus(&self.input_focus, cx),
                None => self.focus_page(window, cx),
            }
        }
        // ponytail: re-read on every session change; an install shows up at
        // the next tab or Space switch.
        self.extensions = self
            .active_space()
            .map(|s| self.host.extensions(s.profile))
            .unwrap_or_default();
        self.save();
        cx.notify();
    }

    /// Rows that appear in a list grow in; rows that leave one (closed, pinned,
    /// filed, archived) collapse where they were.
    fn track_rows(&mut self) {
        let s = &self.session;
        let mut now = HashMap::new();
        let mut add = |list: List, tabs: Vec<&Tab>| {
            for (index, t) in tabs.into_iter().enumerate() {
                now.insert(
                    t.id,
                    Placement {
                        list,
                        index,
                        title: tab_title(t).to_string(),
                        favicon: t.favicon.clone(),
                    },
                );
            }
        };
        for space in s.spaces() {
            for f in s.folders(space.id) {
                add((space.id, Zone::Pinned, Some(f.id)), s.folder_tabs(f.id).collect());
            }
            add((space.id, Zone::Pinned, None), s.tabs(space.id, Zone::Pinned).collect());
            add((space.id, Zone::Today, None), s.tabs(space.id, Zone::Today).collect());
        }
        if let Some(first) = s.spaces().first() {
            add(FAVORITES, s.tabs(first.id, Zone::Favorite).collect());
        }
        let Some(before) = self.placements.replace(now) else {
            return;
        };
        let now = self.placements.as_ref().expect("just set");
        for (id, was) in &before {
            if now.get(id).is_some_and(|p| p.list == was.list) || was.list == FAVORITES {
                continue;
            }
            self.ghosts.retain(|g| g.id != *id);
            self.ghosts.push(Ghost {
                id: *id,
                list: was.list,
                index: was.index,
                title: was.title.clone().into(),
                favicon: was.favicon.clone(),
            });
            self.motion.from(("ghost", id.0), Feel::Glide, 1.0, 0.0);
        }
        for (id, p) in now {
            if before.get(id).is_none_or(|was| was.list != p.list) {
                self.motion.from(("tab-in", id.0), Feel::Glide, 1.0, 0.0);
            }
        }
    }

    /// Keyboard goes to the active page; a hidden page's element is not in the
    /// tree, so focus left there would swallow every action.
    fn focus_page(&self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self
            .active_page()
            .map(|p| p.read(cx).focus_handle(cx))
            .unwrap_or_else(|| self.focus.clone());
        window.focus(&target, cx);
    }

    fn create_page(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.session.tab(id) else {
            return;
        };
        let Some(space) = self.session.space(tab.space) else {
            return;
        };
        let page = self.host.create_page(space.profile, &tab.url, window, cx);
        let sub = cx.subscribe_in(&page, window, move |this, _, event, window, cx| {
            this.on_page_event(id, event, window, cx)
        });
        self.pages.insert(id, (page, sub));
    }

    fn on_page_event(
        &mut self,
        id: TabId,
        event: &PageEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.session.tab_mut(id) else {
            return;
        };
        let (space, url) = (tab.space, tab.url.clone());
        let private = self.session.space(space).is_some_and(|s| s.ephemeral);
        let tab = self.session.tab_mut(id).expect("looked up above");
        match event {
            // Chromium reports a source view by the page's own address.
            PageEvent::UrlChanged(new) if url.strip_prefix("view-source:") == Some(new.as_str()) => return,
            PageEvent::UrlChanged(new) => {
                tab.url = new.clone();
                if !private {
                    // The title arrives after the address; this one is the old page's.
                    if let Some(Err(e)) = self.store.as_ref().map(|s| s.record_visit(new, "", now() as i64)) {
                        log::error!("recording history: {e}");
                    }
                }
            }
            PageEvent::TitleChanged(new) => {
                tab.title = new.clone();
                if !private && !url.starts_with("view-source:") {
                    if let Some(Err(e)) = self.store.as_ref().map(|s| s.set_history_title(&url, new)) {
                        log::error!("recording history: {e}");
                    }
                }
            }
            PageEvent::FaviconChanged(favicon) => tab.favicon = favicon.clone(),
            PageEvent::FaviconImage { url, png } => {
                if let Some(Err(e)) = self.store.as_ref().map(|s| s.save_favicon(url, png)) {
                    log::error!("saving favicon: {e}");
                }
                let image = gpui::Image::from_bytes(gpui::ImageFormat::Png, png.to_vec());
                self.favicons.insert(url.clone(), std::sync::Arc::new(image));
                return cx.notify();
            }
            PageEvent::LoadingChanged(_) | PageEvent::HistoryChanged { .. } => return cx.notify(),
            PageEvent::OpenRequested {
                url, background, ..
            } => {
                let space = tab.space;
                if *background {
                    self.open_background_tab(space, url);
                } else {
                    // Arc's Peek: a link that would open a tab (or a shift-click)
                    // previews over the page instead.
                    let url = url.clone();
                    return self.open_peek(space, &url, window, cx);
                }
            }
            PageEvent::ContextMenu {
                x,
                y,
                link_url,
                selection,
            } => {
                self.menu = Some(ContextMenu {
                    at: point(px(*x), px(*y)),
                    kind: MenuKind::Page {
                        tab: id,
                        link: link_url.clone(),
                        selection: selection.clone(),
                    },
                });
                return cx.notify();
            }
            PageEvent::Closed => self.session.close_tab(id, now()),
            PageEvent::PermissionRequested { .. }
            | PageEvent::PermissionDismissed(_)
            | PageEvent::Dialog { .. }
            | PageEvent::DialogClosed
            | PageEvent::AudibleChanged(_) => return self.on_ask(id, event, window, cx),
            // A tab opened only to fetch a file has nothing to show: it goes.
            PageEvent::DownloadStarted => {
                let blank = tab.title.is_empty()
                    && self.pages.get(&id).is_some_and(|(p, _)| !p.read(cx).can_go_back());
                if !blank {
                    return;
                }
                self.session.close_tab(id, now());
            }
        }
        self.sync(window, cx);
    }

    /// A cmd- or middle-clicked link: a tab for later, the current one stays.
    fn open_background_tab(&mut self, space: SpaceId, url: &str) {
        let current = self.session.space(space).and_then(|s| s.active_tab);
        self.session.open_tab(space, url, now());
        if let Some(current) = current {
            self.session.activate_tab(current, now());
        }
    }

    pub(crate) fn open_peek(
        &mut self,
        space: SpaceId,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_preview(space, url, false, window, cx)
    }

    /// Peek, or with `popup` an extension's toolbar popup under the address bar.
    fn open_preview(
        &mut self,
        space: SpaceId,
        url: &str,
        popup: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = self.session.space(space).map(|s| s.profile) else {
            return;
        };
        self.close_peek(window, cx);
        let page = self.host.create_page(profile, url, window, cx);
        page.read(cx).set_corner_radius(PEEK_RADIUS);
        page.read(cx).set_floating(true);
        let sub = cx.subscribe_in(&page, window, |this, _, event, window, cx| {
            this.on_peek_event(event, window, cx)
        });
        let mut open = Spring::new(0.0);
        open.set_target(1.0);
        window.focus(&page.read(cx).focus_handle(cx), cx);
        self.peek = Some(PeekPage {
            page,
            _sub: sub,
            space,
            url: url.into(),
            open,
            popup,
        });
        cx.notify();
    }

    fn on_peek_event(&mut self, event: &PageEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(peek) = &mut self.peek else { return };
        match event {
            PageEvent::UrlChanged(url) => peek.url = url.clone(),
            PageEvent::OpenRequested {
                url, background, ..
            } => {
                if *background {
                    let space = peek.space;
                    self.open_background_tab(space, url);
                    return self.sync(window, cx);
                }
                peek.page.read(cx).load_url(url);
            }
            PageEvent::Closed => return self.close_peek(window, cx),
            // ponytail: Peek has no room for prompts and an unanswered one
            // freezes the page; opening it as a tab gets them.
            PageEvent::Dialog { .. } => peek.page.read(cx).answer_dialog(false, ""),
            PageEvent::PermissionRequested { id, .. } => peek.page.read(cx).answer_permission(*id, false),
            _ => {}
        }
        cx.notify();
    }

    pub(crate) fn close_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(peek) = self.peek.take() {
            peek.page.read(cx).set_visible(false);
            self.focus_page(window, cx);
            cx.notify();
        }
    }

    /// Peek → a real tab, keeping the live page.
    pub(crate) fn expand_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(peek) = self.peek.take() else { return };
        self.adopt_page(peek.page, peek.space, &peek.url, window, cx);
    }

    /// Makes `page` (from Peek or lil-baa) a tab in `space` of this window.
    pub(crate) fn adopt_page(
        &mut self,
        page: Entity<H::Page>,
        space: SpaceId,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        page.update(cx, |page, cx| page.move_to_window(window, cx));
        page.read(cx).set_corner_radius(PAGE_RADIUS);
        page.read(cx).set_floating(false);
        self.session.set_active_space(space);
        let id = self.session.open_tab(space, url, now());
        let title = page.read(cx).title();
        if let Some(tab) = self.session.tab_mut(id) {
            tab.title = title;
        }
        let sub = cx.subscribe_in(&page, window, move |this, _, event, window, cx| {
            this.on_page_event(id, event, window, cx)
        });
        self.pages.insert(id, (page, sub));
        self.sync(window, cx);
    }

    pub(crate) fn open_extension(
        &mut self,
        extension: &baa_web::Extension,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(space) = self.session.active_space() else {
            return;
        };
        match &extension.popup {
            Some(popup) => self.open_preview(space, popup, true, window, cx),
            None => self.open_extensions(&format!("chrome://extensions/?id={}", extension.id), window, cx),
        }
    }

    /// Back (`back`) or forward pressed: a short press navigates on release, a
    /// long one shows that direction's history.
    pub(crate) fn nav_down(&mut self, back: bool, at: Point<Pixels>, cx: &mut Context<Self>) {
        let pressed = Instant::now();
        self.nav_press = Some((back, pressed));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOLD).await;
            this.update(cx, |this, cx| {
                if this.nav_press.is_some_and(|(_, t)| t == pressed) {
                    this.nav_press = None;
                    this.show_history(back, at, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn nav_up(&mut self, cx: &mut Context<Self>) {
        if let Some((back, _)) = self.nav_press.take() {
            self.with_page(cx, |p| if back { p.go_back() } else { p.go_forward() });
        }
    }

    pub(crate) fn show_history(&mut self, back: bool, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(page) = self.active_page() else { return };
        let (entries, current) = page.read(cx).history();
        let label = |(title, url): &(String, String)| {
            if title.is_empty() { url.clone() } else { title.clone() }
        };
        let entries: Vec<(i32, String)> = if back {
            entries[..current.min(entries.len())]
                .iter()
                .enumerate()
                .rev()
                .map(|(i, e)| (i as i32 - current as i32, label(e)))
                .collect()
        } else {
            entries
                .iter()
                .enumerate()
                .skip(current + 1)
                .map(|(i, e)| (i as i32 - current as i32, label(e)))
                .collect()
        };
        if !entries.is_empty() {
            self.history_menu = Some(HistoryMenu { at, entries });
            cx.notify();
        }
    }

    /// Two-finger horizontal swipe across the chrome switches Spaces; the list
    /// follows the fingers and a quarter width commits.
    pub(crate) fn on_chrome_scroll(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let gpui::ScrollDelta::Pixels(delta) = event.delta else {
            return;
        };
        match event.touch_phase {
            gpui::TouchPhase::Started => self.swipe = Some(0.0),
            gpui::TouchPhase::Moved => {
                let Some(swipe) = &mut self.swipe else { return };
                let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
                if dx.abs() <= dy.abs() {
                    return;
                }
                *swipe = (*swipe + dx / SIDEBAR_WIDTH).clamp(-1.0, 1.0);
                self.slide.snap(*swipe);
                cx.notify();
            }
            gpui::TouchPhase::Ended | gpui::TouchPhase::Cancelled => {
                let Some(swipe) = self.swipe.take() else { return };
                let first = self.session.spaces().first().map(|s| s.id) == self.session.active_space();
                if self.library.is_some() {
                    // Fingers left: back to the first Space, which lies to the right.
                    if swipe < -0.25 {
                        self.close_library(window, cx);
                    }
                    self.slide.set_target(0.0);
                } else if swipe > 0.25 && first {
                    // Past the first Space lies the Library (Arc).
                    self.slide.set_target(0.0);
                    self.open_library(crate::library::Section::Downloads, cx);
                } else if swipe.abs() > 0.25 {
                    // Fingers left: the next Space comes in from the right.
                    self.step_space(if swipe < 0.0 { 1 } else { -1 }, window, cx);
                } else {
                    self.slide.set_target(0.0);
                    cx.notify();
                }
            }
        }
    }

    /// A link from another app: a lil-baa window, which can be expanded into
    /// the browser (Arc's Little Arc).
    pub fn open_external(&mut self, url: &str, cx: &mut Context<Self>) {
        let Some(space) = self.session.active_space() else {
            return;
        };
        let Some(profile) = self.session.space(space).map(|s| s.profile) else {
            return;
        };
        let browser = cx.entity().downgrade();
        let host = &self.host;
        let url = url.to_string();
        let spaces: Vec<crate::lil_baa::SpaceChoice> = self
            .session
            .spaces()
            .iter()
            .filter(|s| !s.ephemeral)
            .map(|s| crate::lil_baa::SpaceChoice {
                id: s.id,
                label: format!("{} {}", s.icon, s.name),
                name: s.name.clone(),
                gradient: (s.accent, s.accent_to),
            })
            .collect();
        let options = crate::lil_baa::window_options(cx);
        let opened = cx.open_window(options, |window, cx| {
            let page = host.create_page(profile, &url, window, cx);
            cx.new(|cx| LilBaa::new(browser, page, (space, profile), url.clone(), spaces, window, cx))
        });
        if let Err(e) = opened {
            log::error!("opening lil-baa: {e}");
        }
    }

    pub(crate) fn window_handle(&self) -> AnyWindowHandle {
        self.window
    }

    pub(crate) fn activate_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        self.session.activate_tab(id, now());
        self.sync(window, cx);
    }

    pub(crate) fn activate_space(
        &mut self,
        id: SpaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.session.set_active_space(id);
        self.sync(window, cx);
    }

    pub(crate) fn close_tab_id(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        self.session.close_tab(id, now());
        self.sync(window, cx);
    }

    pub(crate) fn reopen_closed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.session.reopen_closed(now());
        self.sync(window, cx);
    }

    pub fn open_url(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self.session.active_space() else {
            return;
        };
        self.open_url_in(space, url, window, cx);
    }

    pub(crate) fn open_url_in(
        &mut self,
        space: SpaceId,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.session.set_active_space(space);
        self.session.open_tab(space, url, now());
        self.sync(window, cx);
    }

    /// New tab or, with `in_place` (address mode), the current one.
    pub(crate) fn navigate(
        &mut self,
        url: &str,
        in_place: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.active_tab().filter(|_| in_place) {
            Some(id) => {
                if let Some(tab) = self.session.tab_mut(id) {
                    tab.url = url.into();
                }
                self.with_page(cx, |p| p.load_url(url));
                self.sync(window, cx);
            }
            None => self.open_url(url, window, cx),
        }
    }

    pub(crate) fn new_space(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.session.spaces().len();
        let id = self
            .session
            .add_space(&format!("Space {}", n + 1), ICONS[n % ICONS.len()], None);
        if let Some(space) = self.session.space_mut(id) {
            let (top, bottom) = PALETTE[n % PALETTE.len()];
            (space.accent, space.accent_to) = (top, Some(bottom));
        }
        self.activate_space(id, window, cx);
        self.start_rename(window, cx);
    }

    pub(crate) fn new_incognito_space(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self
            .session
            .add_ephemeral_space("Incognito", self.host.ephemeral_profile());
        if let Some(space) = self.session.space_mut(id) {
            (space.accent, space.accent_to) = (INCOGNITO.0, Some(INCOGNITO.1));
        }
        self.activate_space(id, window, cx);
        self.open_bar(Mode::NewTab, window, cx);
    }

    pub(crate) fn delete_active_space(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.session.active_space() else {
            return;
        };
        if self.session.spaces().len() < 2 {
            return;
        }
        let orphan = self.session.remove_space(id);
        // Pages must be gone before the profile directory is deleted.
        self.sync(window, cx);
        if let Some(profile) = orphan {
            self.host.delete_profile(profile);
        }
    }

    /// "🐑 Work" sets icon and name; plain "Work" keeps the icon.
    pub(crate) fn rename_active_space(&mut self, text: &str) {
        let Some(space) = self
            .session
            .active_space()
            .and_then(|id| self.session.space_mut(id))
        else {
            return;
        };
        let text = text.trim();
        let (icon, name) = match text.split_once(' ') {
            Some((first, rest)) if !first.chars().any(char::is_alphanumeric) => {
                (Some(first), rest.trim())
            }
            _ => (None, text),
        };
        if let Some(icon) = icon {
            space.icon = icon.into();
        }
        if !name.is_empty() {
            space.name = name.into();
        }
        self.save();
    }

    pub(crate) fn cycle_space_color(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self
            .session
            .active_space()
            .and_then(|id| self.session.space_mut(id))
        else {
            return;
        };
        let next = PALETTE
            .iter()
            .position(|(top, _)| *top == space.accent)
            .map_or(0, |i| i + 1)
            % PALETTE.len();
        let (top, bottom) = PALETTE[next];
        (space.accent, space.accent_to) = (top, Some(bottom));
        self.sync(window, cx);
    }

    pub(crate) fn open_extensions(
        &mut self,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(profile) = self.active_space().map(|s| s.profile) {
            self.host.open_extensions(profile, url, cx);
        }
        self.close_overlays(window, cx);
    }

    pub(crate) fn swap_sidebar_side(&mut self, cx: &mut Context<Self>) {
        self.settings.sidebar_side = match self.settings.sidebar_side {
            SidebarSide::Left => SidebarSide::Right,
            SidebarSide::Right => SidebarSide::Left,
        };
        if let Some(Err(e)) = self.store.as_ref().map(|s| s.save_settings(&self.settings)) {
            log::error!("saving settings: {e}");
        }
        cx.notify();
    }

    pub(crate) fn open_bar(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        let field = match mode {
            Mode::Address => TextField::selected(
                self.active_tab()
                    .and_then(|id| self.session.tab(id))
                    .map(|t| t.url.clone())
                    .unwrap_or_default(),
            ),
            Mode::NewTab | Mode::History => TextField::default(),
        };
        self.bar = Some(Bar::new(mode, field));
        command_bar::refresh_history(self);
        self.closing_bar = None;
        self.find = None;
        window.focus(&self.input_focus, cx);
        cx.notify();
    }

    /// Edits the Space's name where the sidebar shows it, all selected.
    pub(crate) fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.active_space().map(|s| s.name.clone()).unwrap_or_default();
        self.bar = None;
        self.find = None;
        self.renaming = Some(TextField::selected(name));
        self.renaming_folder = None;
        if !self.sidebar_open {
            self.reveal.set_target(1.0);
        }
        window.focus(&self.input_focus, cx);
        cx.notify();
    }

    /// A new folder in the active Space, holding `tab` if given, named in place.
    pub(crate) fn new_folder(&mut self, tab: Option<TabId>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self.session.active_space() else {
            return;
        };
        let id = self.session.add_folder(space, "New Folder");
        if let Some(tab) = tab {
            self.session.move_to_folder(tab, id, usize::MAX);
        }
        self.sync(window, cx);
        self.start_folder_rename(id, window, cx);
    }

    pub(crate) fn start_folder_rename(&mut self, id: FolderId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.session.folder(id).map(|f| f.name.clone()) else {
            return;
        };
        self.start_rename(window, cx);
        self.renaming = Some(TextField::selected(name));
        self.renaming_folder = Some(id);
    }

    /// Copies the tab's URL; the header's link icon turns into a check for a moment.
    pub(crate) fn copy_url(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab().and_then(|id| self.session.tab(id)) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(tab.url.clone()));
        self.copies += 1;
        let copy = self.copies;
        self.motion.from(("copied", 0), Feel::Glide, 0.0, 1.0);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _ = this.update(cx, |this, cx| {
                // A later copy restarts the moment instead.
                if this.copies == copy {
                    this.motion.to(("copied", 0), Feel::Glide, 0.0);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn set_folder_open(&mut self, id: FolderId, open: bool) {
        let Some(folder) = self.session.folder_mut(id) else {
            return;
        };
        let from = if folder.collapsed { 0.0 } else { 1.0 };
        folder.collapsed = !open;
        self.folder_motion
            .entry(id)
            .or_insert_with(|| Spring::new(from))
            .set_target(if open { 1.0 } else { 0.0 });
    }

    pub(crate) fn open_menu(&mut self, kind: MenuKind, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.menu = Some(ContextMenu { at, kind });
        cx.notify();
    }

    pub(crate) fn close_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.renaming = None;
        self.renaming_folder = None;
        self.history_menu = None;
        if self.find.take().is_some() {
            self.with_page(cx, |p| p.find("", true));
        }
        if let Some(mut bar) = self.bar.take() {
            bar.scale.set_target(0.0);
            self.closing_bar = Some(bar);
        }
        self.menu = None;
        self.popover = None;
        self.settings_open = false;
        self.motion.forget("menu");
        self.motion.forget("bar-row");
        self.motion.forget("bar-sel");
        match (&self.peek, self.active_dialog()) {
            (Some(peek), _) => window.focus(&peek.page.read(cx).focus_handle(cx), cx),
            (None, Some(_)) => window.focus(&self.input_focus, cx),
            (None, None) => self.focus_page(window, cx),
        }
        cx.notify();
    }

    pub(crate) fn on_input_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shift = event.keystroke.modifiers.shift;
        if self.bar.is_none() && self.renaming.is_none() && self.find.is_none() && self.active_dialog().is_some() {
            self.on_dialog_key(event, window, cx);
        } else if let Some(field) = &mut self.renaming {
            match field.handle(&event.keystroke, cx) {
                Key::Submit => {
                    let text = field.text.trim().to_string();
                    match self.renaming_folder.and_then(|id| self.session.folder_mut(id)) {
                        Some(folder) if !text.is_empty() => folder.name = text,
                        Some(_) => {}
                        None => self.rename_active_space(&text),
                    }
                    self.save();
                    self.close_overlays(window, cx);
                }
                Key::Cancel => self.close_overlays(window, cx),
                _ => {}
            }
        } else if let Some(bar) = &mut self.bar {
            let key = bar.field.handle(&event.keystroke, cx);
            command_bar::on_key(self, key, window, cx);
        } else if let Some(field) = &mut self.find {
            match field.handle(&event.keystroke, cx) {
                Key::Submit => {
                    let text = field.text.clone();
                    self.with_page(cx, |p| p.find(&text, !shift));
                }
                Key::Cancel => self.close_overlays(window, cx),
                _ => {}
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn space_tab_ids(&self) -> Vec<TabId> {
        self.active_space()
            .map(|s| self.session.space_tabs(s.id).map(|t| t.id).collect())
            .unwrap_or_default()
    }

    fn step_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let tabs = self.space_tab_ids();
        if tabs.is_empty() {
            return;
        }
        let i = self
            .active_tab()
            .and_then(|a| tabs.iter().position(|t| *t == a))
            .unwrap_or(0);
        let next = (i as isize + delta).rem_euclid(tabs.len() as isize) as usize;
        self.activate_tab(tabs[next], window, cx);
    }

    pub(crate) fn step_space(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let spaces: Vec<SpaceId> = self.session.spaces().iter().map(|s| s.id).collect();
        let Some(i) = spaces
            .iter()
            .position(|s| Some(*s) == self.session.active_space())
        else {
            return;
        };
        let next = (i as isize + delta).rem_euclid(spaces.len() as isize) as usize;
        self.activate_space(spaces[next], window, cx);
    }

    /// `step` moves along the ladder; `0` resets to 100%.
    fn zoom_by(&mut self, step: isize, cx: &mut Context<Self>) {
        let Some(id) = self.active_tab() else { return };
        let current = self.zoom.get(&id).copied().unwrap_or(1.0);
        let i = ZOOM_LEVELS
            .iter()
            .position(|z| *z >= current)
            .unwrap_or(ZOOM_LEVELS.len() - 1);
        let level = match step {
            0 => 1.0,
            _ => ZOOM_LEVELS[i.saturating_add_signed(step).min(ZOOM_LEVELS.len() - 1)],
        };
        self.zoom.insert(id, level);
        self.with_page(cx, |p| p.set_zoom(level));
    }

    fn register_actions(&self, root: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        root.on_action(
            cx.listener(|this, _: &CommandBar, window, cx| match this.bar {
                Some(_) => this.close_overlays(window, cx),
                None => this.open_bar(Mode::NewTab, window, cx),
            }),
        )
        .on_action(cx.listener(|this, _: &FocusAddress, window, cx| {
            this.open_bar(Mode::Address, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
            this.sidebar_open = !this.sidebar_open;
            this.sidebar_w.set_target(if this.sidebar_open {
                this.sidebar_width()
            } else {
                0.0
            });
            this.reveal_hover = false;
            this.reveal.snap(0.0);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
            if let Some(id) = this.active_tab() {
                this.close_tab_id(id, window, cx);
            }
        }))
        .on_action(
            cx.listener(|this, _: &ReopenClosedTab, window, cx| this.reopen_closed(window, cx)),
        )
        .on_action(cx.listener(|this, _: &NextTab, window, cx| this.step_tab(1, window, cx)))
        .on_action(cx.listener(|this, _: &PrevTab, window, cx| this.step_tab(-1, window, cx)))
        .on_action(cx.listener(|this, GoToTab(n): &GoToTab, window, cx| {
            if let Some(id) = this.space_tab_ids().get(*n) {
                this.activate_tab(*id, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &LastTab, window, cx| {
            if let Some(id) = this.space_tab_ids().last() {
                this.activate_tab(*id, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &NextSpace, window, cx| this.step_space(1, window, cx)))
        .on_action(cx.listener(|this, _: &PrevSpace, window, cx| this.step_space(-1, window, cx)))
        .on_action(cx.listener(|this, GoToSpace(n): &GoToSpace, window, cx| {
            if let Some(id) = this.session.spaces().get(*n).map(|s| s.id) {
                this.activate_space(id, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &TogglePin, window, cx| {
            if let Some(id) = this.active_tab() {
                this.session.toggle_pin(id);
                this.sync(window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &Back, _, cx| this.with_page(cx, |p| p.go_back())))
        .on_action(cx.listener(|this, _: &Forward, _, cx| this.with_page(cx, |p| p.go_forward())))
        .on_action(cx.listener(|this, _: &Reload, _, cx| {
            this.with_page(cx, |p| if p.is_loading() { p.stop() } else { p.reload() })
        }))
        .on_action(cx.listener(|this, _: &CopyUrl, _, cx| this.copy_url(cx)))
        .on_action(cx.listener(|this, _: &Find, window, cx| {
            if this.active_page().is_some() {
                this.bar = None;
                this.find = Some(TextField::default());
                window.focus(&this.input_focus, cx);
                cx.notify();
            }
        }))
        .on_action(
            cx.listener(|this, _: &DevTools, _, cx| this.with_page(cx, |p| p.open_devtools())),
        )
        .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom_by(1, cx)))
        .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom_by(-1, cx)))
        .on_action(cx.listener(|this, _: &ZoomReset, _, cx| this.zoom_by(0, cx)))
        .on_action(cx.listener(|this, _: &NewSpace, window, cx| this.new_space(window, cx)))
        .on_action(cx.listener(|this, _: &Print, _, cx| this.with_page(cx, |p| p.print())))
        .on_action(cx.listener(|this, _: &ViewSource, window, cx| this.view_source(window, cx)))
        .on_action(cx.listener(|this, _: &ReaderMode, _, cx| {
            this.with_page(cx, |p| p.eval(crate::tools::READER_SCRIPT))
        }))
        .on_action(cx.listener(|this, _: &PictureInPicture, _, cx| {
            this.with_page(cx, |p| p.eval_with_gesture(crate::tools::PIP_SCRIPT))
        }))
        .on_action(cx.listener(|this, _: &MuteTab, _, cx| {
            if let Some(id) = this.active_tab() {
                this.toggle_mute(id, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &ShowDownloads, window, cx| {
            match this.library.as_ref().map(|l| l.section) {
                Some(crate::library::Section::Downloads) => this.close_library(window, cx),
                _ => this.open_library(crate::library::Section::Downloads, cx),
            }
        }))
        .on_action(cx.listener(|this, _: &ShowHistory, window, cx| this.open_bar(Mode::History, window, cx)))
        .on_action(cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)))
        .on_action(cx.listener(|this, _: &NewIncognitoSpace, window, cx| {
            this.new_incognito_space(window, cx)
        }))
    }

    fn gradient(&self) -> (Rgba, Rgba) {
        let t = self.fade.value();
        (
            lerp_oklch(self.gradient_from.0, self.gradient_to.0, t),
            lerp_oklch(self.gradient_from.1, self.gradient_to.1, t),
        )
    }

    pub(crate) fn theme(&self) -> Theme {
        let (top, bottom) = self.gradient();
        Theme::from_gradient(top, bottom)
    }

    pub(crate) fn reveal_hovered(&self) -> bool {
        self.reveal_hover
    }

    /// The collapsed sidebar follows the pointer's position, not hover: the
    /// edge zone opens it, and it stays while the pointer is anywhere between
    /// the window edge and a little past the card (the inset gap included), or
    /// has left the window over that edge, or a menu from it is open.
    fn track_reveal(&mut self, at: Point<Pixels>, window: &Window, cx: &mut Context<Self>) {
        if self.sidebar_open {
            return;
        }
        let x = f32::from(match self.side() {
            SidebarSide::Left => at.x,
            SidebarSide::Right => window.viewport_size().width - at.x,
        });
        let reach = if self.reveal_hover {
            INSET + self.sidebar_width() + REVEAL_SLACK
        } else {
            REVEAL_EDGE
        };
        let inside = x < reach || (self.reveal_hover && self.menu.is_some());
        if inside != self.reveal_hover {
            self.set_reveal(inside, cx);
        }
    }

    fn set_reveal(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.reveal_hover = hovered;
        if self.reveal_hovered() {
            self.reveal.set_target(1.0);
            cx.notify();
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REVEAL_LINGER).await;
            this.update(cx, |this, cx| {
                if !this.reveal_hovered() {
                    this.reveal.set_target(0.0);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The collapsed sidebar: an edge hot zone, and the sidebar as a floating card
    /// sliding in over the page (which keeps its size).
    fn render_reveal(
        &mut self,
        sidebar: Option<AnyElement>,
        theme: &Theme,
    ) -> [AnyElement; 2] {
        let right = self.side() == SidebarSide::Right;
        let edge = |d: gpui::Div, offset: f32| {
            if right {
                d.right(px(offset))
            } else {
                d.left(px(offset))
            }
        };
        // Only keeps the edge's pointer events from the page; `track_reveal` decides.
        let zone = edge(div().absolute().top_0().bottom_0().w(px(REVEAL_EDGE)), 0.)
            .id("reveal-edge")
            .occlude();
        let shown = self.reveal.value();
        // Off-screen at 0, inset at 1; the spring's overshoot reads as Arc's bounce.
        let width = self.sidebar_width();
        let offset = INSET + (shown - 1.0) * (width + INSET * 2.0);
        let card = sidebar.map(|sidebar| {
            edge(div().absolute().top(px(HEADER_H)).bottom(px(INSET)), offset)
                .id("reveal-card")
                .occlude()
                .w(px(width))
                // The page's radius: a larger one shows the page past its corners.
                .rounded(px(PAGE_RADIUS))
                .overflow_hidden()
                .bg(gradient_fill(theme))
                .border_1()
                .border_color(Hsla { a: 0.10, ..theme.ink })
                .shadow(vec![
                    shadow(Hsla { a: 0.20, ..theme.ink }, 12., 32.),
                    shadow(Hsla { a: 0.10, ..theme.ink }, 2., 6.),
                ])
                .child(sidebar)
                .into_any_element()
        });
        [
            zone.into_any_element(),
            card.unwrap_or_else(|| div().into_any_element()),
        ]
    }

    /// Peek: a scrim over the browser and the link's page in a card, with
    /// close / open-as-tab controls above it.
    fn render_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let peek = self.peek.as_ref()?;
        let t = peek.open.value();
        let host = SharedString::from(host_of(&peek.url));
        let content = peek.page.update(cx, |p, cx| p.render(window, cx));
        let control = |id: &'static str, name: &'static str| {
            div()
                .id(id)
                .size(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(9.))
                .bg(gpui::rgba(0xFFFFFFEB))
                .shadow(vec![shadow(gpui::rgba(0x0000002E).into(), 2., 8.)])
                .hover(|s| s.bg(gpui::white()))
                .child(icon(name, 15., rgb(0x3A3644).into()))
        };
        let controls = div()
            .absolute()
            .top(px(-40.))
            .left_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .occlude()
            .child(control("peek-close", "x").on_click(
                cx.listener(|this, _, window, cx| this.close_peek(window, cx)),
            ))
            .child(control("peek-expand", "maximize-2").on_click(
                cx.listener(|this, _, window, cx| this.expand_peek(window, cx)),
            ))
            .child(
                div()
                    .px(px(10.))
                    .py(px(5.))
                    .rounded(px(8.))
                    .bg(gpui::rgba(0x14101E66))
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(gpui::white())
                    .child(host),
            );
        if peek.popup {
            // An extension's popup: a small card under the address bar's right end.
            return Some(
                div()
                    .absolute()
                    .inset_0()
                    .child(
                        div()
                            .id("popup-backdrop")
                            .absolute()
                            .inset_0()
                            .occlude()
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, window, cx| this.close_peek(window, cx))),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(HEADER_H + 2.))
                            .right(px(INSET))
                            .mt(px((1.0 - t) * -8.0))
                            .w(px(380.))
                            .h(px(520.))
                            .opacity(t.clamp(0.0, 1.0))
                            .rounded(px(PEEK_RADIUS))
                            .overflow_hidden()
                            .bg(gpui::white())
                            .border_1()
                            .border_color(gpui::rgba(0x0000001F))
                            .shadow(vec![shadow(gpui::rgba(0x0A08143D).into(), 12., 32.)])
                            .child(content),
                    )
                    .into_any_element(),
            );
        }
        // Rises a little as it fades in; gpui has no transforms, so offset and opacity carry it.
        let rise = (1.0 - t) * 24.0;
        let card = div()
            .absolute()
            .left(relative(0.139))
            .right(relative(0.139))
            .top(relative(0.093))
            .bottom(relative(0.084))
            .mt(px(rise))
            .mb(px(-rise))
            .opacity(t.clamp(0.0, 1.0))
            .child(controls)
            .child(
                div()
                    .size_full()
                    .rounded(px(PEEK_RADIUS))
                    .overflow_hidden()
                    .bg(rgb(0xFBFAF7))
                    .border_1()
                    .border_color(gpui::rgba(0x0000001F))
                    .shadow(vec![
                        shadow(gpui::rgba(0x0A081461).into(), 40., 100.),
                        shadow(gpui::rgba(0x0A08142E).into(), 8., 24.),
                    ])
                    .child(content),
            );
        Some(
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .id("peek-scrim")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .bg(surface::scrim())
                        .opacity(t.clamp(0.0, 1.0))
                        .on_click(cx.listener(|this, _, window, cx| this.close_peek(window, cx))),
                )
                .child(card)
                .into_any_element(),
        )
    }

    fn tick_motion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        let reduce = cx.reduce_motion();
        let mut moving = false;
        for spring in [
            &mut self.sidebar_w,
            &mut self.reveal,
            &mut self.slide,
            &mut self.fade,
        ] {
            moving |= spring.tick(dt, reduce);
        }
        if let Some(peek) = &mut self.peek {
            moving |= peek.open.tick(dt, reduce);
        }
        if let Some(bar) = &mut self.bar {
            moving |= bar.scale.tick(dt, reduce);
        }
        if let Some(bar) = &mut self.closing_bar {
            let more = bar.scale.tick(dt, reduce);
            moving |= more;
            if !more {
                self.closing_bar = None;
            }
        }
        moving |= self.motion.tick(dt, reduce);
        let motion = &self.motion;
        self.ghosts.retain(|g| motion.get(("ghost", g.id.0), Feel::Glide) > 0.0);
        self.folder_motion.retain(|_, spring| {
            let more = spring.tick(dt, reduce);
            moving |= more;
            more
        });
        if moving {
            window.request_animation_frame();
        }
    }

    fn render_page(
        &mut self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let page = self.active_page().cloned();
        let content = match page {
            Some(page) => page.update(cx, |p, cx| p.render(window, cx)),
            None => crate::start::render(theme).into_any_element(),
        };
        let asks = self.render_asks(theme, cx);
        let find = self.find.as_ref().map(|field| {
            drop_in(div()
                .id("find")
                // Pages are native views: only occluding gpui keeps the click.
                .occlude()
                .absolute()
                .top(px(12.))
                .right(px(12.))
                .w(px(280.))
                .h(px(40.))
                .px(px(14.))
                .flex()
                .items_center()
                .rounded(px(10.))
                .bg(surface::bg())
                .border_1()
                .border_color(surface::border())
                .shadow(vec![shadow(gpui::rgba(0x18122C29).into(), 8., 24.)])
                .text_size(px(14.))
                .track_focus(&self.input_focus)
                .on_key_down(cx.listener(Self::on_input_key))
                .child(field.render(
                    "Find in page",
                    surface::ink().into(),
                    surface::muted().into(),
                    Hsla { a: 0.25, ..theme.accent },
                )), "find")
        });
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .rounded(px(PAGE_RADIUS))
                    .overflow_hidden()
                    .bg(gpui::white())
                    .shadow(vec![shadow(Hsla { a: 0.12, ..theme.ink }, 1., 4.)])
                    .child(content),
            )
            .children(asks)
            .children(find)
    }

    /// The window's top row (Arc's top-bar layout): traffic lights, sidebar
    /// toggle and navigation at the left, the short URL centred on the window,
    /// extensions at the right. Empty space drags the window.
    fn render_header(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let url = self
            .active_tab()
            .and_then(|id| self.session.tab(id))
            .map(|t| t.url.clone())
            .unwrap_or_default();
        let host = host_of(&url);
        let loading = self.active_page().is_some_and(|p| p.read(cx).is_loading());
        let extensions: Vec<_> = self.extensions.iter().enumerate().map(|(i, ext)| {
            let icon_path = ext.icon.clone();
            let ext = ext.clone();
            let key = ("extension", i as u64);
            let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
            let size = 16. * (1. + 0.08 * h - 0.18 * p);
            let el = div()
                .id(("extension", i))
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .bg(fade(theme.hover, h))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| this.open_extension(&ext, window, cx)))
                .child(match icon_path {
                    Some(path) => gpui::img(path).size(px(size)).into_any_element(),
                    None => icon("sliders-horizontal", size, theme.ink_muted).into_any_element(),
                });
            feel(el, key, cx)
        }).collect();
        let key = ("address", 0);
        let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
        // The link icon copies the URL; on a copy it flips to a check that pops in.
        let copied = self.motion.get(("copied", 0), Feel::Glide);
        let c = copied.clamp(0.0, 1.0);
        let link_key = ("link", 0);
        let (lh, lp) = (self.motion.get(link_key, Feel::Hover), self.motion.get(link_key, Feel::Press));
        let ink = mix(theme.ink_muted, theme.ink, h.max(lh));
        let squash = 1. - 0.2 * lp;
        let link_icon = feel(
            div()
                .id("copy-link")
                .relative()
                .size(px(18.))
                .ml(px(-3.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .bg(fade(theme.hover, lh))
                .on_click(cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.copy_url(cx);
                }))
                .child(icon("link", 13., ink).with_transformation(
                    Transformation::scale(gpui::size(squash * (1. - 0.5 * c), squash * (1. - 0.5 * c)))
                        .with_rotation(gpui::radians(-0.6 * c)),
                ).opacity(1. - c))
                .child(
                    div().absolute().inset_0().flex().items_center().justify_center().child(
                        // Overshoot in the spring is the pop.
                        icon("check", 13., theme.accent)
                            .with_transformation(Transformation::scale(gpui::size(
                                squash * copied.max(0.),
                                squash * copied.max(0.),
                            )))
                            .opacity(c),
                    ),
                ),
            link_key,
            cx,
        );
        let label: SharedString = if host.is_empty() {
            "Search or enter address".into()
        } else {
            host.clone().into()
        };
        // A new host fades in; the id changes with it, so the animation restarts.
        let label = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .min_w_0()
            .when(!host.is_empty(), |d| d.child(link_icon))
            .child(div().truncate().child(label.clone()))
            .children(self.render_site_button(theme, ink, cx))
            .with_animation(
                ElementId::Name(label),
                Animation::new(std::time::Duration::from_millis(220)).with_easing(gpui::ease_out_quint()),
                |d, t| d.opacity(t).ml(px(4. * (1. - t))),
            );
        let accent = theme.accent;
        let progress = loading.then(|| {
            div()
                .absolute()
                .bottom_0()
                .h(px(2.))
                .w(relative(0.3))
                .rounded(px(1.))
                .bg(accent)
                .with_animation(
                    "loading",
                    Animation::new(std::time::Duration::from_millis(1100))
                        .repeat()
                        .with_easing(gpui::ease_in_out),
                    |d, t| d.left(relative(-0.3 + 1.3 * t)),
                )
        });
        let address = div()
            .id("address")
            .relative()
            .overflow_hidden()
            .flex_none()
            .max_w(px(480.))
            .h(px(26.))
            .px(px(10.))
            .mx(px(3. * p))
            .flex()
            .items_center()
            .rounded(px(7.))
            .text_size(px(13.))
            .text_color(mix(theme.ink_muted, theme.ink, h))
            .bg(fade(theme.hover, h))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.open_bar(Mode::Address, window, cx)))
            .child(label)
            .children(progress);
        let address = feel(address, key, cx);
        div()
            .id("header")
            .flex_none()
            .h(px(HEADER_H))
            .pl(px(16.))
            .pr(px(INSET + 2.))
            .flex()
            .items_center()
            .on_mouse_down(gpui::MouseButton::Left, crate::sidebar::drag_or_zoom)
            .on_scroll_wheel(cx.listener(|this, e, window, cx| this.on_chrome_scroll(e, window, cx)))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .child(self.render_window_controls(theme, cx)),
            )
            .child(address)
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(4.))
                    .children(self.render_download_button(theme, cx))
                    .children(extensions),
            )
    }

    fn render_history_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let menu = self.history_menu.as_ref()?;
        let accent = self.theme().accent;
        let rows: Vec<_> = menu.entries.iter().enumerate().map(|(i, (offset, title))| {
            let offset = *offset;
            let key = ("menu", i as u64);
            let h = self.motion.get(key, Feel::Hover);
            let row = div()
                .id(("history", offset as i64 as u64))
                .h(px(30.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(px(7.))
                .max_w(px(360.))
                .bg(fade(accent, h))
                .text_color(mix(surface::ink().into(), gpui::white(), h))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.with_page(cx, |p| p.go_to_offset(offset));
                    this.close_overlays(window, cx);
                }))
                .child(div().truncate().child(SharedString::from(title.clone())));
            feel(row, key, cx)
        }).collect();
        Some(deferred(div().absolute().inset_0().occlude().child(
            anchored().position(menu.at).snap_to_window().child(drop_in(
                div()
                    .id("history-menu")
                    .occlude()
                    .min_w(px(220.))
                    .p(px(5.))
                    .rounded(px(10.))
                    .bg(surface::bg())
                    .border_1()
                    .border_color(surface::border())
                    .shadow(vec![shadow(gpui::rgba(0x18122C29).into(), 8., 24.)])
                    .text_size(px(13.))
                    .text_color(surface::ink())
                    .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_overlays(window, cx)))
                    .children(rows),
                "history-menu",
            )),
        )))
    }

    fn render_context_menu(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let menu = self.menu.as_ref()?;
        let items = match &menu.kind {
            MenuKind::Page {
                tab,
                link,
                selection,
            } => self.page_menu(*tab, link.clone(), selection.clone()),
            kind => self.sidebar_menu(kind),
        };
        let rows: Vec<_> = items.into_iter().enumerate().map(|(i, (label, run))| {
            let key = ("menu", i as u64);
            let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
            let row = div()
                .id(("menu", i))
                .h(px(32.))
                .px(px(10.))
                .mx(px(2. * p))
                .flex()
                .items_center()
                .rounded(px(7.))
                .bg(fade(theme.accent, h))
                .text_color(mix(surface::ink().into(), gpui::white(), h))
                .on_click(cx.listener(move |this, _, window, cx| {
                    // Closed first: a rename started by the item must stay open.
                    this.close_overlays(window, cx);
                    run(this, window, cx);
                }))
                .child(label);
            feel(row, key, cx)
        }).collect();
        // The backdrop keeps a click on the (native) page from bypassing the
        // menu's mouse_down_out.
        Some(deferred(div().absolute().inset_0().occlude().child(
            anchored().position(menu.at).snap_to_window().child(drop_in(
                div()
                    .id("context-menu")
                    .occlude()
                    .min_w(px(200.))
                    .p(px(5.))
                    .rounded(px(10.))
                    .bg(surface::bg())
                    .border_1()
                    .border_color(surface::border())
                    .shadow(vec![shadow(gpui::rgba(0x18122C29).into(), 8., 24.)])
                    .text_size(px(13.))
                    .text_color(surface::ink())
                    .on_mouse_down_out(
                        cx.listener(|this, _, window, cx| this.close_overlays(window, cx)),
                    )
                    .children(rows),
                "context-menu",
            )),
        )))
    }

    fn sidebar_menu(&self, kind: &MenuKind) -> Vec<(&'static str, MenuAction<H>)> {
        let mut items: Vec<(&'static str, MenuAction<H>)> = Vec::new();
        let space = self.session.active_space();
        match *kind {
            MenuKind::Tab(id) => {
                let Some(tab) = self.session.tab(id) else {
                    return items;
                };
                let zone = tab.zone;
                if zone == Zone::Favorite {
                    items.push((
                        "Remove from Favorites",
                        Box::new(move |this, window, cx| {
                            if let Some(space) = space {
                                this.session.move_tab(id, space, Zone::Today, 0);
                                this.sync(window, cx);
                            }
                        }),
                    ));
                } else {
                    items.push((
                        "Add to Favorites",
                        // The Space argument is ignored: a favourite keeps its own.
                        Box::new(move |this, window, cx| {
                            let space = this.session.tab(id).map(|t| t.space);
                            if let Some(space) = space {
                                this.session.move_tab(id, space, Zone::Favorite, usize::MAX);
                                this.sync(window, cx);
                            }
                        }),
                    ));
                    items.push((
                        if zone == Zone::Pinned { "Unpin" } else { "Pin" },
                        Box::new(move |this, window, cx| {
                            this.session.toggle_pin(id);
                            this.sync(window, cx);
                        }),
                    ));
                    items.push((
                        "Move to New Folder",
                        Box::new(move |this, window, cx| this.new_folder(Some(id), window, cx)),
                    ));
                }
                items.push((
                    "Close Tab",
                    Box::new(move |this, window, cx| this.close_tab_id(id, window, cx)),
                ));
            }
            MenuKind::Download { ref path, ref url } => {
                let (open, reveal, link, forget) = (path.clone(), path.clone(), url.clone(), path.clone());
                items.push(("Open", Box::new(move |_, _, cx| cx.open_with_system(std::path::Path::new(&open)))));
                items.push((
                    "Show in Finder",
                    Box::new(move |_, _, cx| cx.reveal_path(std::path::Path::new(&reveal))),
                ));
                items.push((
                    "Copy Download Link",
                    Box::new(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(link.clone()))),
                ));
                items.push(("Remove from List", Box::new(move |this, _, cx| this.forget_download(&forget, cx))));
            }
            MenuKind::LibraryFilter => {
                for (kind, label) in crate::library::KINDS {
                    items.push((
                        label,
                        Box::new(move |this, _, cx| {
                            if let Some(library) = &mut this.library {
                                library.filter = kind;
                            }
                            cx.notify();
                        }),
                    ));
                }
            }
            MenuKind::Folder(id) => {
                items.push((
                    "Rename Folder",
                    Box::new(move |this, window, cx| this.start_folder_rename(id, window, cx)),
                ));
                items.push((
                    "Delete Folder",
                    Box::new(move |this, window, cx| {
                        this.session.remove_folder(id);
                        this.sync(window, cx);
                    }),
                ));
            }
            MenuKind::Sidebar | MenuKind::Page { .. } => {
                items.push((
                    "New Folder",
                    Box::new(|this, window, cx| this.new_folder(None, window, cx)),
                ));
                items.push((
                    "New Tab",
                    Box::new(|this, window, cx| this.open_bar(Mode::NewTab, window, cx)),
                ));
            }
        }
        items
    }

    fn page_menu(
        &self,
        tab: TabId,
        link: Option<String>,
        selection: Option<String>,
    ) -> Vec<(&'static str, MenuAction<H>)> {
        let mut items: Vec<(&'static str, MenuAction<H>)> = Vec::new();
        if let Some(link) = link {
            let space = self.session.tab(tab).map(|t| t.space);
            let open = link.clone();
            items.push((
                "Open Link in New Tab",
                Box::new(move |this, window, cx| {
                    if let Some(space) = space {
                        this.session.open_tab(space, &open, now());
                        this.sync(window, cx);
                    }
                }),
            ));
            items.push((
                "Copy Link",
                Box::new(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(link.clone()))
                }),
            ));
        }
        if let Some(text) = selection.filter(|s| !s.is_empty()) {
            items.push((
                "Copy",
                Box::new(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                }),
            ));
        }
        items.push((
            "Back",
            Box::new(|this, _, cx| this.with_page(cx, |p| p.go_back())),
        ));
        items.push((
            "Forward",
            Box::new(|this, _, cx| this.with_page(cx, |p| p.go_forward())),
        ));
        items.push((
            "Reload",
            Box::new(|this, _, cx| this.with_page(cx, |p| p.reload())),
        ));
        items.push((
            "Inspect",
            Box::new(|this, _, cx| this.with_page(cx, |p| p.open_devtools())),
        ));
        items
    }
}

/// A sidebar list: (Space, zone, folder). Favourites are one list for all.
pub(crate) type List = (SpaceId, Zone, Option<FolderId>);
pub(crate) const FAVORITES: List = (SpaceId(0), Zone::Favorite, None);
/// Width of a Space's emoji button in the bottom bar; the pill slides by it.
pub(crate) const SPACE_DOT: f32 = 24.0;

struct Placement {
    list: List,
    index: usize,
    title: String,
    favicon: Option<String>,
}

/// A row that left its list, drawn collapsing at its old place.
pub(crate) struct Ghost {
    pub id: TabId,
    pub list: List,
    pub index: usize,
    pub title: SharedString,
    pub favicon: Option<String>,
}

/// Hover and press springs for `key` (see `Motion`). Press listens in the
/// capture phase so a handler that stops propagation cannot swallow it.
pub(crate) fn feel<H: WebHost>(el: Stateful<Div>, key: motion::Key, cx: &mut Context<Browser<H>>) -> Stateful<Div> {
    let press = move |on: bool| {
        move |this: &mut Browser<H>, _: &dyn std::any::Any, _: &mut Window, cx: &mut Context<Browser<H>>| {
            if on || this.motion.get(key, Feel::Press) != 0.0 {
                this.motion.to(key, Feel::Press, on as u8 as f32);
                cx.notify();
            }
        }
    };
    let (down, up, out) = (press(true), press(false), press(false));
    el.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
        this.motion.to(key, Feel::Hover, *hovered as u8 as f32);
        cx.notify();
    }))
    .capture_any_mouse_down(cx.listener(move |this, e: &gpui::MouseDownEvent, w, cx| down(this, e, w, cx)))
    .capture_any_mouse_up(cx.listener(move |this, e: &gpui::MouseUpEvent, w, cx| up(this, e, w, cx)))
    .on_mouse_up_out(MouseButton::Left, cx.listener(move |this, e: &gpui::MouseUpEvent, w, cx| out(this, e, w, cx)))
}

/// Overlays (menus, find) drop in: a short fade and settle.
pub(crate) fn drop_in<E: IntoElement + gpui::Styled + 'static>(el: E, id: &'static str) -> impl IntoElement {
    el.with_animation(
        id,
        Animation::new(std::time::Duration::from_millis(160)).with_easing(gpui::ease_out_quint()),
        |el, t| el.opacity(t).mt(px(-6. * (1. - t))),
    )
}

/// A link previewed over the page.
pub(crate) struct PeekPage<P> {
    pub(crate) page: Entity<P>,
    _sub: Subscription,
    pub(crate) space: SpaceId,
    pub(crate) url: String,
    open: Spring,
    /// An extension popup rather than a link preview.
    pub(crate) popup: bool,
}

/// Back/forward held down: the history in that direction.
pub(crate) struct HistoryMenu {
    pub at: Point<Pixels>,
    /// (offset from the current entry, title).
    pub entries: Vec<(i32, String)>,
}

/// How long the header shows the check after copying the URL.
const COPIED_FOR: std::time::Duration = std::time::Duration::from_millis(1200);

/// Back/forward pressed this long shows history instead.
const HOLD: std::time::Duration = std::time::Duration::from_millis(400);

/// The window's Space gradient, top to bottom.
pub(crate) fn gradient_fill(theme: &Theme) -> gpui::Background {
    linear_gradient(
        180.,
        linear_color_stop(theme.bg_top, 0.),
        linear_color_stop(theme.bg_bottom, 1.),
    )
}

pub(crate) fn shadow(color: Hsla, y: f32, blur: f32) -> BoxShadow {
    BoxShadow {
        color,
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(0.),
        inset: false,
    }
}

/// "https://www.example.com/a" → "example.com".
pub(crate) fn host_of(url: &str) -> String {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

pub(crate) fn tab_title(tab: &Tab) -> &str {
    if tab.title.is_empty() {
        &tab.url
    } else {
        &tab.title
    }
}

impl<H: WebHost> Render for Browser<H> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.tick_motion(window, cx);
        let theme = self.theme();
        let sidebar = self.render_sidebar(&theme, cx);
        let page = self.render_page(&theme, window, cx);
        // One sidebar element: docked, or in the reveal card while that is out.
        let (docked, reveal) = if self.sidebar_open {
            (Some(sidebar), None)
        } else if self.reveal.value() > 0.001 || self.reveal_hovered() {
            (None, Some(self.render_reveal(Some(sidebar), &theme)))
        } else {
            (Some(sidebar), Some(self.render_reveal(None, &theme)))
        };
        let peek = self.render_peek(window, cx);
        if let Some(bar) = &self.bar {
            self.motion.to(("bar-sel", 0), Feel::Glide, bar.selected as f32);
        }
        let bar = match (&self.bar, &self.closing_bar) {
            (Some(bar), _) => Some(self.render_command_bar(bar, true, &theme, window, cx)),
            (None, Some(bar)) => Some(self.render_command_bar(bar, false, &theme, window, cx)),
            _ => None,
        };
        let menu = self.render_context_menu(&theme, cx);
        let popover = self.render_popover(&theme, cx);
        let history = self.render_history_menu(cx);
        // The sidebar brings its own padding; collapsed, the page gets the inset.
        let docked_w = self.sidebar_w.value().max(0.0);
        let edge = INSET * (1.0 - docked_w / SIDEBAR_WIDTH).clamp(0.0, 1.0);
        let right = self.side() == SidebarSide::Right;

        let header = self.render_header(&theme, cx);
        let root = div()
            .key_context("Browser")
            .track_focus(&self.focus)
            .child({
                // Every move in the window, even over overlays that occlude the root.
                let track = cx.listener(|this, e: &gpui::MouseMoveEvent, window, cx| {
                    this.track_reveal(e.position, window, cx)
                });
                gpui::canvas(|_, _, _| {}, move |_, _, window, _| {
                    window.on_mouse_event(move |e: &gpui::MouseMoveEvent, phase, window, cx| {
                        if phase == gpui::DispatchPhase::Bubble {
                            track(e, window, cx);
                        }
                    })
                })
                .absolute()
                .size_0()
            })
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(gradient_fill(&theme))
            .text_color(theme.ink)
            .text_size(px(13.))
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .when(right, |d| d.flex_row_reverse())
                    .pb(px(INSET))
                    .when(right, |d| d.pl(px(INSET)).pr(px(edge)))
                    .when(!right, |d| d.pr(px(INSET)).pl(px(edge)))
                    .child(
                        div()
                            .flex_shrink_0()
                            .h_full()
                            .w(px(docked_w))
                            .overflow_hidden()
                            .children(docked),
                    )
                    .child(page),
            )
            .children(reveal.into_iter().flatten())
            .children(peek)
            .children(bar)
            .children(self.render_toast())
            .children(self.render_settings(&theme, cx))
            .children(popover)
            .children(menu)
            .children(history);
        let root = self.register_actions(root, cx);
        if self.peek.is_some() {
            root.on_action(cx.listener(|this, _: &ClosePeek, window, cx| {
                if this.bar.is_none() {
                    this.close_peek(window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &ExpandPeek, window, cx| this.expand_peek(window, cx)))
        } else if self.popover.is_some() || self.settings_open {
            // Escape; bound only while there is something to close, so pages keep it otherwise.
            root.on_action(cx.listener(|this, _: &ClosePeek, window, cx| this.close_overlays(window, cx)))
        } else {
            root
        }
    }
}

impl<H: WebHost> gpui::Focusable for Browser<H> {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}
