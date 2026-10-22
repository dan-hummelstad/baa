//! Root window view. Owns the `Session` (baa-model is the source of truth for
//! spaces/tabs), the live pages, and every §8 action handler.

use std::{collections::HashMap, time::Instant};

use baa_model::{now, Session, SidebarSide, Space, SpaceId, Store, Tab, TabId};
use baa_web::{PageEvent, PageHandle, WebHost};
use gpui::{
    anchored, deferred, div, point, prelude::*, px, rgb, ClipboardItem, Context, Entity,
    FocusHandle, KeyDownEvent, Pixels, Point, Rgba, Subscription, Window,
};

use crate::{
    actions::*,
    command_bar::{self, Bar, Mode},
    input::{Key, TextField},
    motion::Spring,
    theme::{lerp_oklch, Theme},
};

pub(crate) const SIDEBAR_WIDTH: f32 = 240.0;
/// Chrome's zoom ladder, as factors (`PageHandle::set_zoom` takes 1.0 = 100%).
const ZOOM_LEVELS: [f64; 15] = [
    0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
];
const PALETTE: [u32; 7] = [
    0x6c9cff, 0xff8a65, 0x81c784, 0xba68c8, 0xffd54f, 0x4dd0e1, 0xf06292,
];
const ICONS: [&str; 7] = ["🐑", "🌿", "🔥", "🌊", "⭐", "🍋", "🪐"];
const INCOGNITO_ACCENT: u32 = 0x8a8a96;

type MenuAction<H> = Box<dyn Fn(&mut Browser<H>, &mut Window, &mut Context<Browser<H>>)>;

pub(crate) struct ContextMenu {
    pub tab: TabId,
    pub at: Point<Pixels>,
    pub link: Option<String>,
    pub selection: Option<String>,
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
    focus: FocusHandle,
    /// Shared by the command bar and find bar; only one is open at a time.
    pub(crate) input_focus: FocusHandle,
    sidebar_open: bool,
    pub(crate) sidebar_w: Spring,
    /// Space-switch slide, in sidebar widths (−1…1).
    pub(crate) slide: Spring,
    fade: Spring,
    accent_from: Rgba,
    accent_to: Rgba,
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
        // gpui's flag also drives `with_animation`; the app shell sets it from the OS.
        if settings.reduce_motion {
            cx.set_reduce_motion(true);
        }
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
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
            focus,
            input_focus: cx.focus_handle(),
            sidebar_open: true,
            sidebar_w: Spring::new(SIDEBAR_WIDTH),
            slide: Spring::new(0.0),
            fade: Spring::new(1.0),
            accent_from: rgb(PALETTE[0]),
            accent_to: rgb(PALETTE[0]),
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

    fn active_page(&self) -> Option<&Entity<H::Page>> {
        self.pages.get(&self.active_tab()?).map(|(p, _)| p)
    }

    fn with_page(&self, cx: &mut Context<Self>, f: impl FnOnce(&H::Page)) {
        if let Some(page) = self.active_page() {
            f(page.read(cx));
        }
    }

    pub(crate) fn side(&self) -> SidebarSide {
        self.active_space()
            .and_then(|s| s.sidebar_side)
            .unwrap_or(self.settings.sidebar_side)
    }

    fn save(&mut self) {
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
            self.shown_space = space;
            self.session
                .archive_expired(now(), self.settings.archive_after_hours);
        }
        // ponytail: `accent_to` gradients render as the first stop until the gradient theme lands.
        let accent = self
            .active_space()
            .map_or(self.accent_to, |s| rgb(s.accent));
        if accent != self.accent_to {
            self.accent_from = self.accent(); // mid-flight colour, so rapid switches never jump
            self.accent_to = accent;
            self.fade.snap(0.0);
            self.fade.set_target(1.0);
        }

        let session = &self.session;
        self.pages.retain(|id, _| session.tab(*id).is_some());
        let active = self.active_tab();
        if let Some(id) = active.filter(|id| !self.pages.contains_key(id)) {
            self.create_page(id, window, cx);
        }
        for (id, (page, _)) in &self.pages {
            page.read(cx).set_visible(Some(*id) == active);
        }
        if self.bar.is_none() && self.find.is_none() {
            self.focus_page(window, cx);
        }
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
        match event {
            PageEvent::UrlChanged(url) => tab.url = url.clone(),
            PageEvent::TitleChanged(title) => tab.title = title.clone(),
            PageEvent::FaviconChanged(favicon) => tab.favicon = favicon.clone(),
            PageEvent::LoadingChanged(_) | PageEvent::HistoryChanged { .. } => return cx.notify(),
            PageEvent::OpenRequested { url, .. } => {
                // ponytail: links from pinned tabs should open in Peek (§3); milestone 4.
                let space = tab.space;
                self.session.open_tab(space, url, now());
            }
            PageEvent::ContextMenu {
                x,
                y,
                link_url,
                selection,
            } => {
                self.menu = Some(ContextMenu {
                    tab: id,
                    at: point(px(*x), px(*y)),
                    link: link_url.clone(),
                    selection: selection.clone(),
                });
                return cx.notify();
            }
            PageEvent::Closed => self.session.close_tab(id, now()),
        }
        self.sync(window, cx);
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
            space.accent = PALETTE[n % PALETTE.len()];
        }
        self.activate_space(id, window, cx);
        self.open_bar(Mode::RenameSpace, window, cx);
    }

    pub(crate) fn new_incognito_space(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self
            .session
            .add_ephemeral_space("Incognito", self.host.ephemeral_profile());
        if let Some(space) = self.session.space_mut(id) {
            space.accent = INCOGNITO_ACCENT;
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
            .position(|c| *c == space.accent)
            .map_or(0, |i| i + 1)
            % PALETTE.len();
        space.accent = PALETTE[next];
        self.sync(window, cx);
    }

    pub(crate) fn open_extensions(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
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
            Mode::RenameSpace => TextField::selected(
                self.active_space()
                    .map(|s| s.name.clone())
                    .unwrap_or_default(),
            ),
            Mode::NewTab => TextField::default(),
        };
        self.bar = Some(Bar::new(mode, field));
        self.find = None;
        window.focus(&self.input_focus, cx);
        cx.notify();
    }

    pub(crate) fn close_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.take().is_some() {
            self.with_page(cx, |p| p.find("", true));
        }
        self.bar = None;
        self.menu = None;
        self.focus_page(window, cx);
        cx.notify();
    }

    pub(crate) fn on_input_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shift = event.keystroke.modifiers.shift;
        if let Some(bar) = &mut self.bar {
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

    fn step_space(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
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
                SIDEBAR_WIDTH
            } else {
                0.0
            });
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
        .on_action(cx.listener(|this, _: &NewIncognitoSpace, window, cx| {
            this.new_incognito_space(window, cx)
        }))
    }

    fn accent(&self) -> Rgba {
        lerp_oklch(self.accent_from, self.accent_to, self.fade.value())
    }

    pub(crate) fn theme(&self) -> Theme {
        Theme::from_accent(self.accent())
    }

    fn tick_motion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        let reduce = cx.reduce_motion();
        let mut moving = false;
        for spring in [&mut self.sidebar_w, &mut self.slide, &mut self.fade] {
            moving |= spring.tick(dt, reduce);
        }
        if let Some(bar) = &mut self.bar {
            moving |= bar.scale.tick(dt, reduce);
        }
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
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.text_dim)
                .child(if cfg!(target_os = "macos") {
                    "⌘T to open a tab"
                } else {
                    "Ctrl+T to open a tab"
                })
                .into_any_element(),
        };
        let find = self.find.as_ref().map(|field| {
            div()
                .id("find")
                .absolute()
                .top_2()
                .right_2()
                .w(px(260.))
                .px_3()
                .py_1()
                .rounded_md()
                .bg(theme.chrome_active)
                .border_1()
                .border_color(theme.border)
                .shadow_md()
                .text_sm()
                .track_focus(&self.input_focus)
                .on_key_down(cx.listener(Self::on_input_key))
                .child(field.render(
                    "Find in page",
                    theme.text,
                    theme.text_dim,
                    theme.chrome_hover,
                ))
        });
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .p_2()
            .child(
                div()
                    .size_full()
                    .rounded_lg()
                    .overflow_hidden()
                    .bg(gpui::white())
                    .shadow_md()
                    .child(content),
            )
            .children(find)
    }

    fn render_context_menu(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let menu = self.menu.as_ref()?;
        let mut items: Vec<(&'static str, MenuAction<H>)> = Vec::new();
        if let Some(link) = menu.link.clone() {
            let space = self.session.tab(menu.tab).map(|t| t.space);
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
        if let Some(text) = menu.selection.clone().filter(|s| !s.is_empty()) {
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

        let rows = items.into_iter().enumerate().map(|(i, (label, run))| {
            div()
                .id(("menu", i))
                .px_3()
                .py_1()
                .rounded_sm()
                .hover(|s| s.bg(theme.chrome_hover))
                .on_click(cx.listener(move |this, _, window, cx| {
                    run(this, window, cx);
                    this.close_overlays(window, cx);
                }))
                .child(label)
        });
        Some(deferred(
            anchored().position(menu.at).snap_to_window().child(
                div()
                    .id("context-menu")
                    .occlude()
                    .min_w(px(180.))
                    .p_1()
                    .rounded_md()
                    .bg(theme.chrome_active)
                    .border_1()
                    .border_color(theme.border)
                    .shadow_lg()
                    .text_sm()
                    .text_color(theme.text)
                    .on_mouse_down_out(
                        cx.listener(|this, _, window, cx| this.close_overlays(window, cx)),
                    )
                    .children(rows),
            ),
        ))
    }
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
        let bar = self.render_command_bar(&theme, cx);
        let menu = self.render_context_menu(&theme, cx);

        let root = div()
            .key_context("Browser")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .when(self.side() == SidebarSide::Right, |d| d.flex_row_reverse())
            .bg(theme.chrome)
            .text_color(theme.text)
            .child(
                div()
                    .flex_shrink_0()
                    .h_full()
                    .w(px(self.sidebar_w.value().max(0.0)))
                    .overflow_hidden()
                    .child(sidebar),
            )
            .child(page)
            .children(bar)
            .children(menu);
        self.register_actions(root, cx)
    }
}

impl<H: WebHost> gpui::Focusable for Browser<H> {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}
