//! Root window view. Owns the `Session` (baa-model is the source of truth for
//! spaces/tabs), the live pages, and every §8 action handler.

use crate::theme::r;
use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use baa_model::{now, DevToolsDock, DownloadRecord, FolderId, Session, SidebarSide, Space, SpaceId, Store, Tab, TabId, Zone};
use baa_web::{PageEvent, PageHandle, WebHost};
use gpui::{
    anchored, deferred, div, linear_color_stop, linear_gradient, point, prelude::*, px,
    relative, rgb, AnyElement, AnyWindowHandle, WindowHandle, BoxShadow, ClipboardItem, Context, Entity,
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

mod peek;
mod nav;
mod menus;
mod spaces;
mod actions;
mod page_area;
mod header;
mod groups;
mod live;
mod page_state;
mod shared_extensions;
pub(crate) use shared_extensions::InstallTarget;
#[cfg(test)]
mod tests_shared_extensions;
#[cfg(test)]
mod tests_live;
#[cfg(test)]
mod tests_m3_page;
pub(crate) use nav::HistoryMenu;
pub(crate) use peek::PeekPage;

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
/// Space gradients (top, bottom): the swatch row of Arc's theme editor, left to right;
/// bottom is the swatch, top is it 40% toward white.
pub(crate) const PALETTE: [(u32, u32); 9] = [
    (0xF6F0ED, 0xF0E6E1), // cream
    (0xF1C4D7, 0xE79CBC), // pink
    (0xC4A9C0, 0x9C7096), // purple
    (0xECA1A6, 0xE0636B), // red
    (0xF5B99E, 0xEF8B5E), // orange
    (0xF7E5A7, 0xF1D46C), // yellow
    (0xABEEC4, 0x73E39D), // green
    (0xAFD3E8, 0x7AB6D8), // blue
    (0x5F5F7F, 0x34344A), // slate, the dark one
];
pub(crate) const PALETTE_NAMES: [&str; 9] = ["Cream", "Pink", "Purple", "Red", "Orange", "Yellow", "Green", "Blue", "Slate"];
const ICONS: [&str; 7] = ["🐑", "🌿", "🔥", "🌊", "⭐", "🍋", "🪐"];
const INCOGNITO: (u32, u32) = (0x4A4856, 0x2A2932);

type MenuAction<H> = Box<dyn Fn(&mut Browser<H>, &mut Window, &mut Context<Browser<H>>)>;

pub(crate) enum MenuKind {
    /// Right-clicks in the sidebar.
    Tab(TabId),
    /// A finished download in the Library.
    Download { path: String, url: String },
    /// The Library's Downloads filter.
    LibraryFilter,
    Folder(FolderId),
    Sidebar,
    /// The Space's name at the top of the sidebar.
    Space(SpaceId),
    /// An extension in the extensions panel.
    Extension(String),
}

pub struct Browser<H: WebHost> {
    pub(crate) host: H,
    /// `None` after an unreadable session: run on a fresh one without overwriting the file.
    store: Option<Store>,
    pub(crate) session: Session,
    pub(crate) settings: baa_model::Settings,
    pub(crate) pages: HashMap<TabId, (Entity<H::Page>, Subscription)>,
    /// DevTools docked beside a tab's page (see `Settings::devtools_dock`).
    pub(crate) devtools: HashMap<TabId, (Entity<H::Page>, Subscription)>,
    /// How much of the page area docked DevTools takes, and whether its seam is being dragged.
    devtools_share: f32,
    devtools_drag: bool,
    /// The page area's bounds, last frame: the seam drag measures against it.
    page_area: std::rc::Rc<std::cell::Cell<gpui::Bounds<Pixels>>>,
    zoom: HashMap<TabId, f64>,
    pub(crate) bar: Option<Bar>,
    pub(crate) find: Option<TextField>,
    /// The tab whose page is fullscreen: it fills the window.
    pub(crate) fullscreen: Option<TabId>,
    pub(crate) page_state: page_state::PageState,
    /// A native context menu is open (it runs modally).
    pub(crate) menu_open: bool,
    /// A link previewed over the page (Peek), or an extension's popup.
    pub(crate) peek: Option<PeekPage<H::Page>>,
    /// The Space name being edited in place.
    pub(crate) renaming: Option<TextField>,
    /// Set when `renaming` edits this folder's name instead of the Space's.
    pub(crate) renaming_folder: Option<FolderId>,
    /// Folders opening or closing: 0 closed, 1 open. Absent = at rest.
    pub(crate) folder_motion: HashMap<FolderId, Spring>,
    /// Why a live folder's last refresh failed.
    pub(crate) live_errors: HashMap<FolderId, String>,
    /// When each live folder last fetched, and everything it got (hidden repos included,
    /// so un-hiding one needs no fetch).
    pub(crate) live_fetched: HashMap<FolderId, u64>,
    live_items: HashMap<FolderId, Vec<baa_model::LiveItem>>,
    /// (profile, extension) copies under way, so none starts twice.
    copying: HashSet<(baa_model::ProfileId, String)>,
    /// The profiles shared extensions were last spread over.
    shared_to: Vec<baa_model::ProfileId>,
    pending_install: Option<shared_extensions::PendingInstall>,
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
    pub(crate) cleared_downloads: HashSet<u64>,
    pub(crate) downloads_seen: usize,
    /// The settings window, once opened (it may since have been closed).
    pub(crate) settings_window: Option<WindowHandle<crate::settings::SettingsView<H>>>,
    /// The Library, shown in the sidebar instead of the Space.
    pub(crate) library: Option<crate::library::Library>,
    pub(crate) library_focus: FocusHandle,
    /// Finished downloads, newest first, kept across launches.
    pub(crate) saved_downloads: Vec<DownloadRecord>,
    /// This session's downloads already saved.
    pub(crate) recorded_downloads: HashSet<u64>,
    /// A short confirmation at the bottom of the window, and how many so far.
    pub(crate) toast: Option<SharedString>,
    pub(crate) toasts: u64,
    _downloads: Subscription,
    _chrome_tabs: Subscription,
    pub(crate) extensions_page: crate::settings::ExtensionsPage,
    /// The skin last sent to the engine: Space profile and the colours it was made from.
    pub(crate) skin_sent: Option<(baa_model::ProfileId, u64)>,
    /// Back/forward pressed and held: which way, and when.
    pub(crate) nav_press: Option<(bool, Instant)>,
    pub(crate) history_menu: Option<HistoryMenu>,
    /// Two-finger swipe across the chrome, in sidebar widths, while one is on.
    swipe: Option<f32>,
    pub(crate) window: AnyWindowHandle,
    focus: FocusHandle,
    /// Shared by the command bar and find bar; only one is open at a time.
    pub(crate) input_focus: FocusHandle,
    pub(crate) sidebar_open: bool,
    pub(crate) sidebar_w: Spring,
    /// The hover card's width, so opening the Library in it grows rather than jumps.
    reveal_w: Spring,
    /// Collapsed sidebar shown over the page: 0 hidden, 1 shown.
    reveal: Spring,
    /// Pointer over the edge zone / over the card.
    /// The pointer is where the collapsed sidebar should stay out.
    reveal_hover: bool,
    /// Space-switch slide, in sidebar widths (−1…1).
    pub(crate) slide: Spring,
    fade: Spring,
    gradient_from: (Rgba, Rgba),
    pub(crate) gradient_to: (Rgba, Rgba),
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
        // The extensions page is the settings window's tab now; a restored tab for it has no page.
        // ponytail: auto-archive runs at launch and on Space switch only; a timer comes with milestone 4.
        session.on_launch(&settings, now(), |t| crate::pages::route(&t.url) == Some(crate::pages::Route::Extensions));
        host.apply_settings(&settings);
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
        // Taken by whichever window sees them first.
        let chrome_tabs = cx.observe_in(&host.chrome_tabs(), window, |this: &mut Self, list, window, cx| {
            let urls = list.update(cx, |list, _| std::mem::take(&mut list.0));
            for url in urls {
                this.open_url(&url, window, cx);
            }
        });
        let saved_downloads = store.as_ref().and_then(|s| s.downloads().ok()).unwrap_or_default();
        let mut this = Self {
            host,
            store,
            shown_space: session.active_space(),
            session,
            settings,
            pages: HashMap::new(),
            devtools: HashMap::new(),
            devtools_share: 0.4,
            devtools_drag: false,
            page_area: Default::default(),
            zoom: HashMap::new(),
            bar: None,
            find: None,
            fullscreen: None,
            page_state: Default::default(),
            menu_open: false,
            peek: None,
            renaming: None,
            renaming_folder: None,
            folder_motion: HashMap::new(),
            live_errors: HashMap::new(),
            live_fetched: HashMap::new(),
            live_items: HashMap::new(),
            copying: HashSet::new(),
            shared_to: Vec::new(),
            pending_install: None,
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
            settings_window: None,
            library: None,
            library_focus: cx.focus_handle(),
            saved_downloads,
            recorded_downloads: HashSet::new(),
            toast: None,
            toasts: 0,
            _downloads: downloads,
            _chrome_tabs: chrome_tabs,
            extensions_page: Default::default(),
            skin_sent: None,
            nav_press: None,
            history_menu: None,
            swipe: None,
            window: window.window_handle(),
            focus,
            input_focus: cx.focus_handle(),
            sidebar_open: true,
            sidebar_w: Spring::new(SIDEBAR_WIDTH),
            reveal_w: Spring::new(SIDEBAR_WIDTH),
            reveal: Spring::quick(0.0),
            reveal_hover: false,
            slide: Spring::new(0.0),
            fade: Spring::new(1.0),
            gradient_from: (rgb(PALETTE[0].0), rgb(PALETTE[0].1)),
            gradient_to: (rgb(PALETTE[0].0), rgb(PALETTE[0].1)),
            last_frame: cx.background_executor().now(),
        };
        this.sync(window, cx);
        this.start_live_refresh(window, cx);
        // ponytail: a fixed wait for Chrome to load each profile's extensions before the
        // launch pass looks for copies to make; later passes run when Spaces are added.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_secs(5)).await;
            this.update(cx, |this, cx| this.share_extensions(cx)).ok();
        })
        .detach();
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
        self.host.apply_settings(&self.settings);
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
        self.devtools.retain(|id, _| session.tab(*id).is_some());
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
        if self.fullscreen.is_some_and(|t| Some(t) != active) {
            self.end_fullscreen(true, window, cx);
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
        self.share_extensions_if_new_profiles(cx);
        self.sync_groups(cx);
        self.save();
        cx.notify();
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
                    // previews over the page instead, unless settings say tab.
                    let url = url.clone();
                    if !self.settings.peek_links {
                        return self.open_url_in(space, &url, window, cx);
                    }
                    return self.open_peek(space, &url, window, cx);
                }
            }
            PageEvent::Closed => self.session.close_tab(id, now()),
            // Only extension popups send it.
            PageEvent::PreferredSize(_) => return,
            // Only docked DevTools sends it, to its own subscription.
            PageEvent::DevToolsDock(_) => return,
            PageEvent::DevToolsRequested { x, y } => {
                // Inspect always opens (or moves to) the element, never closes.
                self.devtools.remove(&id);
                return self.toggle_devtools(Some((*x, *y)), window, cx);
            }
            PageEvent::PermissionRequested { .. }
            | PageEvent::AskClosed(_)
            | PageEvent::Dialog(_)
            | PageEvent::AudibleChanged(_) => return self.on_ask(id, event, window, cx),
            PageEvent::GroupChanged { group, title } => return self.on_group_changed(id, *group, title, window, cx),
            PageEvent::Fullscreen(_) | PageEvent::FindResult { .. } | PageEvent::Crashed(_) => {
                return self.on_page_state_event(id, event, window, cx)
            }
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

    pub(crate) fn window_handle(&self) -> AnyWindowHandle {
        self.window
    }

    pub(crate) fn activate_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        self.session.activate_tab(id, now());
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
        if self.open_own_page(url, window, cx) {
            return;
        }
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
        if self.open_own_page(url, window, cx) {
            return;
        }
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

    pub(crate) fn toggle_extension_pin(&mut self, id: &str, cx: &mut Context<Self>) {
        let unpinned = &mut self.settings.unpinned_extensions;
        match unpinned.iter().position(|x| x == id) {
            Some(i) => {
                unpinned.remove(i);
            }
            None => unpinned.push(id.into()),
        }
        self.save_settings(cx);
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
        self.popover = None;
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
                Key::Submit => self.commit_rename(window, cx),
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

    /// `step` moves along the ladder; `0` resets to 100%.
    fn zoom_by(&mut self, step: isize, cx: &mut Context<Self>) {
        let Some(id) = self.active_tab() else { return };
        let base = self.settings.web.default_zoom;
        let current = self.zoom.get(&id).copied().unwrap_or(base);
        let i = ZOOM_LEVELS
            .iter()
            .position(|z| *z >= current)
            .unwrap_or(ZOOM_LEVELS.len() - 1);
        let level = match step {
            0 => base,
            _ => ZOOM_LEVELS[i.saturating_add_signed(step).min(ZOOM_LEVELS.len() - 1)],
        };
        self.zoom.insert(id, level);
        self.with_page(cx, |p| p.set_zoom(level));
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

/// How long the header shows the check after copying the URL.
const COPIED_FOR: std::time::Duration = std::time::Duration::from_millis(1200);

/// The window's Space gradient, top to bottom.
pub(crate) fn gradient_fill(theme: &Theme) -> gpui::Background {
    linear_gradient(
        180.,
        linear_color_stop(theme.bg_top, 0.),
        linear_color_stop(theme.bg_bottom, 1.),
    )
}

pub(crate) fn shadow(color: Hsla, y: f32, blur: f32) -> BoxShadow {
    // ponytail: ASCII Chrome keeps only the deep overlay drop; row lifts would glow on navy.
    if crate::theme::ascii() {
        let a = if blur >= 24. { 0.66 } else { 0. };
        return BoxShadow {
            color: Hsla { a, ..gpui::black() },
            offset: point(px(0.), px(18.)),
            blur_radius: px(48.),
            spread_radius: px(0.),
            inset: false,
        };
    }
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
        crate::theme::set_ascii(self.settings.chrome == baa_model::ChromeStyle::Ascii);
        self.push_webui_skin();
        if let Some(view) = self.fullscreen_view(window, cx) {
            return self.register_actions(view, cx);
        }
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
        let bar = match (&self.bar, &self.closing_bar) {
            (Some(bar), _) => Some(self.render_command_bar(bar, true, &theme, window, cx)),
            (None, Some(bar)) => Some(self.render_command_bar(bar, false, &theme, window, cx)),
            _ => None,
        };
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
            .when(crate::theme::ascii(), |d| d.font_family(crate::theme::MONO).text_size(px(12.5)))
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
            .children(popover)
            .children(history);
        let root = self.register_actions(root, cx);
        if self.peek.is_some() {
            root.on_action(cx.listener(|this, _: &ClosePeek, window, cx| {
                if this.bar.is_none() {
                    this.close_peek(window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &ExpandPeek, window, cx| this.expand_peek(window, cx)))
        } else if self.popover.is_some() {
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
