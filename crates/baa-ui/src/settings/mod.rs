//! The settings window (⌘,): its own gpui window, like Safari's, with a
//! toolbar of tabs and a search field. One per app. Every change is written
//! to baa-model's `Settings` at once and pushed to the engine through
//! `WebHost::apply_settings`; there is no Apply.
//!
//! The view holds no settings itself: it reads and writes the `Browser`
//! entity, so the browser window and this one cannot disagree.

mod extensions;
mod privacy;
pub(crate) mod rows;
pub(crate) mod tabs;

use std::{collections::HashMap, time::Instant};

use baa_model::{Permission, PermissionState, Settings, SpaceId};
use baa_web::WebHost;
use gpui::{
    div, prelude::*, px, size, AnyElement, App, Bounds, Context, Entity, FocusHandle, Hsla,
    KeyDownEvent, ScrollHandle, SharedString, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowOptions,
};

pub(crate) use extensions::ExtensionsPage;
pub use tabs::KeymapPath;
#[cfg(test)]
pub(crate) use tabs::ensure_keymap;
pub(crate) use rows::{Row, Tab};

use crate::{
    browser::{shadow, Browser},
    icons::icon,
    input::{Key, TextField},
    motion::{Feel, Motion},
    theme::{ascii, fade, mix, r, surface, Theme},
};

/// A row of a tab's body, with its id when the index has it (for jumping to it).
type Item = (Option<&'static str>, AnyElement);

const TAB_W: f32 = 66.;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Field {
    Search,
    Languages,
    EngineName,
    EngineUrl,
    DnsTemplate,
    SitePattern,
    SpaceName(SpaceId),
}

pub(crate) struct SettingsView<H: WebHost> {
    pub(crate) browser: Entity<Browser<H>>,
    _sub: Subscription,
    pub(crate) tab: Tab,
    /// The Space the per-Space tabs (sites, extensions) are about.
    pub(crate) space: SpaceId,
    fields: HashMap<Field, TextField>,
    active: Field,
    /// Highlighted search result.
    pub(crate) selected: usize,
    pub(crate) motion: Motion,
    last_frame: Instant,
    focus: FocusHandle,
    scroll: ScrollHandle,
    /// A row to scroll to and flash at the next frame, and the one flashing.
    jump: Option<&'static str>,
    flash: Option<&'static str>,
    theme: Theme,
    pub(crate) clear: Option<privacy::ClearSheet>,
    /// The origin whose own permissions the Sites tab shows instead of the list.
    site: Option<String>,
    /// The kind whose "add exception" form is open.
    adding: Option<Permission>,
    /// Chrome's rules per kind for `space`, read when needed rather than per frame.
    rules: HashMap<Permission, Vec<(String, PermissionState)>>,
    /// Reset asked once: whether site exceptions go too.
    reset: Option<bool>,
    confirm_delete: Option<SpaceId>,
    renaming: bool,
    /// `is_default_browser`, read when the window opens or regains focus.
    pub(crate) default_browser: bool,
    /// Custom DNS chosen, its address not valid yet: the engine keeps the old mode meanwhile.
    dns_pending: bool,
    /// Why the exception form could not add its rule.
    add_error: Option<String>,
    _activation: Subscription,
}

pub(crate) fn window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(900.), px(640.)), cx))),
        titlebar: Some(TitlebarOptions { title: Some("Settings".into()), appears_transparent: false, traffic_light_position: None }),
        is_resizable: false,
        is_minimizable: true,
        ..Default::default()
    }
}

impl<H: WebHost> Browser<H> {
    /// The one entry point: opens the settings window at `tab` and `row`, or
    /// brings the open one there. `None` keeps the tab it shows.
    /// Deleting a Space goes through the Spaces tab, where the confirmation says what goes with it.
    pub(crate) fn ask_delete_space(&mut self, id: SpaceId, cx: &mut Context<Self>) {
        self.open_settings_at(Some(Tab::Spaces), Some("spaces"), cx);
        let browser = cx.entity();
        // After open_settings_at's own deferred step, so the window exists.
        cx.defer(move |cx| {
            if let Some(handle) = browser.read(cx).settings_window {
                handle
                    .update(cx, |view, _, cx| {
                        view.confirm_delete = Some(id);
                        cx.notify();
                    })
                    .ok();
            }
        });
    }

    pub(crate) fn open_settings_at(&mut self, tab: Option<Tab>, row: Option<&'static str>, cx: &mut Context<Self>) {
        let browser = cx.entity();
        // Later, outside this update: a new window draws once before it returns,
        // and that draw reads this entity.
        cx.defer(move |cx| {
            if let Some(handle) = browser.read(cx).settings_window {
                let went = handle.update(cx, |view, window, cx| {
                    view.go_to(tab, row, cx);
                    window.activate_window();
                });
                if went.is_ok() {
                    return;
                }
            }
            let this = browser.clone();
            let opened = cx.open_window(window_options(cx), |window, cx| {
                cx.new(|cx| SettingsView::new(this, tab.unwrap_or(Tab::General), row, window, cx))
            });
            match opened {
                Ok(handle) => browser.update(cx, |b, _| b.settings_window = Some(handle)),
                Err(e) => log::error!("opening settings: {e}"),
            }
        });
    }
}

impl<H: WebHost> Browser<H> {
    /// Opens settings on the Sites tab, showing only `site`'s permissions.
    pub(crate) fn open_site_settings(&mut self, site: String, cx: &mut Context<Self>) {
        self.open_settings_at(Some(Tab::Sites), None, cx);
        // After open_settings_at's own deferred open, so the window exists by now.
        let browser = cx.entity();
        cx.defer(move |cx| {
            let Some(handle) = browser.read(cx).settings_window else { return };
            // The site popover's Space, not whichever one settings last showed.
            let b = browser.read(cx);
            let space = b.session.active_space().filter(|id| b.session.space(*id).is_some_and(|s| !s.ephemeral));
            let _ = handle.update(cx, |view, _, cx| {
                view.site = Some(site);
                match space {
                    Some(id) => view.pick_space(id, cx),
                    None => cx.notify(),
                }
            });
        });
    }
}

impl<H: WebHost> SettingsView<H> {
    fn new(browser: Entity<Browser<H>>, tab: Tab, row: Option<&'static str>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let default_browser = browser.read(cx).host.is_default_browser();
        let (space, theme) = {
            let b = browser.read(cx);
            let first = b.session.spaces().iter().find(|s| !s.ephemeral).map(|s| s.id);
            let active = b.session.active_space().filter(|id| b.session.space(*id).is_some_and(|s| !s.ephemeral));
            (active.or(first).unwrap_or(SpaceId(0)), b.theme())
        };
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        window.activate_window();
        let mut motion = Motion::default();
        let i = tab.index() as f32;
        motion.from(("settings-pill", 0), Feel::Glide, i, i);
        motion.from(("settings-tab", 0), Feel::Glide, 1., 1.);
        let mut view = Self {
            _sub: cx.observe(&browser, |_, _, cx| cx.notify()),
            browser,
            tab,
            space,
            fields: HashMap::new(),
            active: Field::Search,
            selected: 0,
            motion,
            last_frame: cx.background_executor().now(),
            focus,
            scroll: ScrollHandle::new(),
            jump: None,
            flash: None,
            theme,
            clear: None,
            site: None,
            adding: None,
            rules: HashMap::new(),
            reset: None,
            confirm_delete: None,
            renaming: false,
            default_browser,
            dns_pending: false,
            add_error: None,
            _activation: cx.observe_window_activation(window, |this, _, cx| this.on_activate(cx)),
        };
        view.seed_fields(cx);
        view.go_to(Some(tab), row, cx);
        view
    }

    /// Text boxes that mirror a setting start with its value.
    pub(super) fn seed_fields(&mut self, cx: &mut Context<Self>) {
        let s = self.browser.read(cx).settings.clone();
        self.field_mut(Field::Languages).text = s.web.languages.unwrap_or_default();
        if let baa_model::SecureDns::Custom(t) = &s.web.secure_dns {
            self.field_mut(Field::DnsTemplate).text = t.clone();
        }
    }

    pub(crate) fn text(&self, field: Field) -> &str {
        self.fields.get(&field).map_or("", |f| f.text.as_str())
    }

    fn field_mut(&mut self, field: Field) -> &mut TextField {
        self.fields.entry(field).or_default()
    }

    pub(crate) fn searching(&self) -> bool {
        !self.text(Field::Search).trim().is_empty()
    }

    /// Search hits in tab order, then by score.
    pub(crate) fn results(&self) -> Vec<&'static Row> {
        let mut hits = rows::search(self.text(Field::Search).trim());
        hits.sort_by_key(|r| r.tab.index());
        hits
    }

    pub(crate) fn go_to(&mut self, tab: Option<Tab>, row: Option<&'static str>, cx: &mut Context<Self>) {
        self.field_mut(Field::Search).text.clear();
        self.selected = 0;
        if let Some(tab) = tab {
            self.select_tab(tab, cx);
        }
        // Routed to Extensions (the toolbar's Manage…): the Space in front is the one meant.
        if tab == Some(Tab::Extensions) {
            let b = self.browser.read(cx);
            if let Some(id) = b.session.active_space().filter(|id| b.session.space(*id).is_some_and(|s| !s.ephemeral)) {
                self.space = id;
            }
        }
        self.jump = row;
        self.flash = None;
        // The clear-data page of Chrome's settings is a sheet here.
        if row == Some("clear_data") {
            self.clear = Some(privacy::ClearSheet::default());
        }
        self.refresh(cx);
        cx.notify();
    }

    /// Regaining focus: the default browser, Chrome's rules and its extensions may have changed meanwhile.
    pub(crate) fn on_activate(&mut self, cx: &mut Context<Self>) {
        self.default_browser = self.browser.read(cx).host.is_default_browser();
        self.refresh(cx);
        cx.notify();
    }

    /// Moves the keys to `field`; what was typed in a box that applies on leaving applies now.
    pub(crate) fn focus_field(&mut self, field: Field, cx: &mut Context<Self>) {
        let left = self.active;
        self.active = field;
        if left != field {
            self.commit(left, cx);
        }
        cx.notify();
    }

    /// Writes a text box that is not applied per keystroke: languages and the DNS address.
    fn commit(&mut self, field: Field, cx: &mut Context<Self>) {
        let text = self.text(field).trim().to_string();
        match field {
            Field::Languages => {
                let new = (!text.is_empty()).then_some(text);
                if self.browser.read(cx).settings.web.languages != new {
                    self.change(cx, |s| s.web.languages = new);
                }
            }
            Field::DnsTemplate if baa_model::SecureDns::valid_template(&text) => {
                self.dns_pending = false;
                self.change(cx, |s| s.web.secure_dns = baa_model::SecureDns::Custom(text));
            }
            _ => {}
        }
    }

    pub(crate) fn select_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.field_mut(Field::Search).text.clear();
        self.focus_field(Field::Search, cx);
        self.add_error = None;
        self.adding = None;
        self.site = None;
        self.reset = None;
        self.confirm_delete = None;
        self.renaming = false;
        if tab != self.tab {
            self.tab = tab;
            self.scroll.set_offset(gpui::point(px(0.), px(0.)));
            self.motion.from(("settings-tab", 0), Feel::Glide, 0., 1.);
            self.motion.to(("settings-pill", 0), Feel::Glide, tab.index() as f32);
        }
        self.refresh(cx);
        cx.notify();
    }

    /// Reads what the tab shows from the engine.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        match self.tab {
            Tab::Sites => self.rules = self.read_rules(cx),
            Tab::Extensions => {
                let space = self.space;
                self.browser.update(cx, |b, cx| b.refresh_extensions_for(space, cx));
            }
            _ => {}
        }
    }

    fn read_rules(&self, cx: &App) -> HashMap<Permission, Vec<(String, PermissionState)>> {
        let b = self.browser.read(cx);
        let Some(profile) = b.session.space(self.space).map(|s| s.profile) else { return HashMap::new() };
        Permission::SITE_KINDS.into_iter().map(|k| (k, b.host.site_rules(profile, k))).collect()
    }

    pub(crate) fn pick_space(&mut self, id: SpaceId, cx: &mut Context<Self>) {
        self.space = id;
        self.adding = None;
        self.refresh(cx);
        cx.notify();
    }

    /// Writes a setting and pushes it to the engine.
    pub(crate) fn change(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Settings)) {
        self.browser.update(cx, |b, cx| {
            f(&mut b.settings);
            b.save_settings(cx);
        });
    }

    /// Runs `f` in the browser window, after this update: it needs that
    /// window (new tabs, Space changes), and the browser may be mid-update.
    /// `show` brings the browser window forward, for actions that open a page.
    pub(crate) fn in_browser(
        &mut self,
        cx: &mut Context<Self>,
        show: bool,
        f: impl FnOnce(&mut Browser<H>, &mut Window, &mut Context<Browser<H>>) + 'static,
    ) {
        let browser = self.browser.clone();
        let window = browser.read(cx).window_handle();
        cx.defer(move |cx| {
            window
                .update(cx, |_, window, cx| {
                    browser.update(cx, |b, cx| f(b, window, cx));
                    if show {
                        window.activate_window();
                    }
                })
                .ok();
        });
    }

    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit(self.active, cx);
        window.remove_window();
    }

    // Keys: the search field has focus on open; clicking another field moves it.
    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        let m = &ks.modifiers;
        if m.secondary() && ks.key == "w" {
            return self.close(window, cx);
        }
        if m.control && ks.key == "tab" {
            let n = Tab::ALL.len() as isize;
            let i = (self.tab.index() as isize + if m.shift { -1 } else { 1 }).rem_euclid(n);
            return self.select_tab(Tab::ALL[i as usize], cx);
        }
        let active = self.active;
        let key = self.field_mut(active).handle(ks, cx);
        match key {
            Key::Edited => self.edited(active),
            Key::Cancel => self.escape(window, cx),
            Key::Submit => self.submitted(active, cx),
            Key::Up if active == Field::Search => self.selected = self.selected.saturating_sub(1),
            Key::Down if active == Field::Search => {
                self.selected = (self.selected + 1).min(self.results().len().saturating_sub(1))
            }
            _ => return,
        }
        cx.notify();
    }

    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.clear.take().is_some() {
            return;
        }
        if self.searching() {
            self.field_mut(Field::Search).text.clear();
            return;
        }
        if self.active != Field::Search {
            self.focus_field(Field::Search, cx);
            return;
        }
        self.close(window, cx);
    }

    fn edited(&mut self, field: Field) {
        if field == Field::Search {
            self.selected = 0;
        }
        self.add_error = None;
    }

    fn submitted(&mut self, field: Field, cx: &mut Context<Self>) {
        match field {
            Field::Search => {
                if let Some(row) = self.results().get(self.selected) {
                    self.go_to(Some(row.tab), Some(row.id), cx);
                }
            }
            Field::SpaceName(id) => self.finish_rename(id, cx),
            Field::EngineName | Field::EngineUrl => self.add_engine(cx),
            Field::Languages | Field::DnsTemplate => self.commit(field, cx),
            _ => {}
        }
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = cx.background_executor().now(); // the fake clock under test, so motion is deterministic there
        let dt = (now - self.last_frame).as_secs_f32().min(1. / 30.);
        self.last_frame = now;
        if self.motion.tick(dt, cx.reduce_motion()) {
            window.request_animation_frame();
        }
    }

    // Controls.

    /// Hover and press feel for anything the pointer touches here.
    pub(crate) fn feel(&self, el: gpui::Stateful<gpui::Div>, key: crate::motion::Key, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let (down, up) = (
            cx.listener(move |this, _: &gpui::MouseDownEvent, _, cx| {
                this.motion.to(key, Feel::Press, 1.);
                cx.notify();
            }),
            cx.listener(move |this, _: &gpui::MouseUpEvent, _, cx| {
                this.motion.to(key, Feel::Press, 0.);
                cx.notify();
            }),
        );
        el.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            this.motion.to(key, Feel::Hover, *hovered as u8 as f32);
            cx.notify();
        }))
        .capture_any_mouse_down(move |e, w, cx| down(e, w, cx))
        .capture_any_mouse_up(move |e, w, cx| up(e, w, cx))
    }

    /// The look of a switch; the caller adds the click.
    pub(crate) fn switch(&self, id: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
        let base = div().id(id).debug_selector(|| id.to_string()).flex_none();
        if ascii() {
            return base.text_color(if on { self.theme.accent } else { surface::icon().into() }).child(if on { "[x]" } else { "[ ]" });
        }
        // The knob springs once it has been flipped; before that it just sits.
        let t = match self.motion.target((id, 0), Feel::Glide) {
            Some(target) if (target > 0.5) == on => self.motion.get((id, 0), Feel::Glide),
            _ => on as u8 as f32,
        };
        base.w(px(36.))
            .h(px(22.))
            .p(px(2.))
            .rounded(r(11.))
            .bg(mix(surface::line().into(), self.theme.accent, t.clamp(0., 1.)))
            .child(
                div()
                    .size(px(18.))
                    .ml(px(14. * t))
                    .rounded(r(9.))
                    .bg(gpui::white())
                    .shadow(vec![shadow(gpui::rgba(0x00000033).into(), 1., 3.)]),
            )
    }

    pub(crate) fn toggle(&self, id: &'static str, on: bool, set: fn(&mut Settings, bool), cx: &mut Context<Self>) -> AnyElement {
        self.switch(id, on)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.motion.from((id, 0), Feel::Glide, on as u8 as f32, !on as u8 as f32);
                this.change(cx, |s| set(s, !on));
            }))
            .into_any_element()
    }

    /// A segmented control; `on` picks the highlighted segment.
    pub(crate) fn segments<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        options: Vec<(T, SharedString)>,
        on: T,
        set: impl Fn(T, &mut Settings) + Clone + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.segments_with(id, options, on, move |this, v, cx| {
            let set = set.clone();
            this.change(cx, move |s| set(v, s))
        }, cx)
    }

    /// Segments whose click does something other than write a setting.
    pub(crate) fn segments_with<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        options: Vec<(T, SharedString)>,
        on: T,
        pick: impl Fn(&mut Self, T, &mut Context<Self>) + Clone + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(id)
            .p(px(2.))
            .flex()
            .flex_wrap()
            .gap(px(2.))
            .rounded(r(8.))
            .when(!ascii(), |d| d.bg(surface::key()))
            .children(options.into_iter().enumerate().map(|(i, (value, label))| {
                let selected = value == on;
                let key = (id, i as u64);
                let h = self.motion.get(key, Feel::Hover);
                let pick = pick.clone();
                let el = div()
                    .id((id, i))
                    .debug_selector(|| format!("{id}:{i}"))
                    .h(px(24.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(r(6.))
                    .text_size(px(12.))
                    .when(selected, |d| d.bg(surface::raised()).shadow(vec![shadow(gpui::rgba(0x0000001F).into(), 1., 2.)]))
                    .when(!selected, |d| d.bg(fade(surface::raised().into(), 0.5 * h)))
                    .text_color(if selected { surface::ink() } else { surface::muted() })
                    .when(ascii() && selected, |d| d.bg(surface::key()).text_color(self.theme.accent))
                    .on_click(cx.listener(move |this, _, _, cx| pick(this, value, cx)))
                    .child(if ascii() {
                        SharedString::from(format!("[{} {label} ]", if selected { "◉" } else { " " }))
                    } else {
                        label
                    });
                self.feel(el, key, cx)
            }))
            .into_any_element()
    }

    pub(crate) fn button(&self, id: &'static str, label: impl Into<SharedString>, strong: bool, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let key = (id, 0);
        let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
        let el = div()
            .id(id)
            .debug_selector(|| id.to_string())
            .h(px(26.))
            .px(px(11.))
            .mx(px(1.5 * p))
            .flex()
            .flex_none()
            .items_center()
            .rounded(r(7.))
            .text_size(px(12.5))
            .font_weight(gpui::FontWeight::MEDIUM)
            .bg(if strong {
                mix(self.theme.accent, gpui::black(), 0.12 * h)
            } else {
                mix(surface::key().into(), surface::line().into(), h)
            })
            .text_color(if strong { gpui::white() } else { surface::ink().into() })
            .child(label.into());
        self.feel(el, key, cx)
    }

    /// A one-line text box; clicking it sends the keys there.
    pub(crate) fn field(&self, field: Field, id: &'static str, placeholder: &'static str, w: f32, cx: &mut Context<Self>) -> AnyElement {
        let active = self.active == field;
        let empty = self.text(field).is_empty();
        let accent = self.theme.accent;
        let shown: AnyElement = match self.fields.get(&field) {
            Some(f) if active => f.render(placeholder, surface::ink().into(), surface::muted().into(), Hsla { a: 0.25, ..accent }).into_any_element(),
            Some(f) if !empty => div().truncate().child(SharedString::from(f.text.clone())).into_any_element(),
            _ if active => TextField::default().render(placeholder, surface::ink().into(), surface::muted().into(), accent).into_any_element(),
            _ => div().truncate().text_color(surface::muted()).child(placeholder).into_any_element(),
        };
        div()
            .id(id)
            .debug_selector(|| id.to_string())
            .w(px(w))
            .h(px(26.))
            .px(px(8.))
            .flex()
            .items_center()
            .overflow_hidden()
            .rounded(r(7.))
            .border_1()
            .border_color(if active { accent } else { surface::border().into() })
            .bg(surface::key())
            .text_size(px(12.5))
            .on_click(cx.listener(move |this, _, _, cx| this.focus_field(field, cx)))
            .child(shown)
            .into_any_element()
    }

    pub(crate) fn section(&self, title: &'static str) -> Item {
        let el = if ascii() {
            crate::theme::ascii::rule_label(title, surface::icon().into()).pt(px(16.)).pb(px(2.))
        } else {
            div()
                .pt(px(18.))
                .pb(px(2.))
                .px(px(10.))
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(surface::muted())
                .child(title.to_uppercase())
        };
        (None, el.into_any_element())
    }

    /// One setting: its label from the index, a hint, and the control.
    pub(crate) fn row(&self, id: &'static str, detail: &str, control: AnyElement) -> Item {
        debug_assert!(rows::find(id).is_some(), "{id} is not in ROWS");
        let flash = if self.flash == Some(id) { self.motion.get(("settings-flash", 0), Feel::Glide) } else { 0. };
        let detail = (!detail.is_empty()).then(|| SharedString::from(detail.to_string()));
        let el = div()
            .id(("row", rows::ROWS.iter().position(|r| r.id == id).unwrap_or(0)))
            .debug_selector(|| format!("row:{id}"))
            .py(px(9.))
            .px(px(10.))
            .rounded(r(8.))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(16.))
            .bg(fade(self.theme.accent, 0.16 * flash.clamp(0., 1.)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(13.)).child(rows::label(id)))
                    .children(detail.map(|d| div().text_size(px(11.5)).text_color(surface::muted()).child(d))),
            )
            .child(div().flex_none().max_w(px(430.)).flex().justify_end().child(control));
        (Some(id), el.into_any_element())
    }

    /// A line under a row (an exception, a shortcut); not in the index.
    pub(crate) fn sub_row(&self, selector: String, left: AnyElement, right: AnyElement) -> Item {
        let el = div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector)
            .py(px(5.))
            .pl(px(26.))
            .pr(px(10.))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .child(div().flex_1().min_w_0().truncate().text_size(px(12.5)).child(left))
            .child(div().flex_none().flex().items_center().gap(px(8.)).child(right));
        (None, el.into_any_element())
    }

    pub(crate) fn note(&self, text: impl Into<SharedString>) -> Item {
        (None, div().py(px(14.)).px(px(10.)).text_size(px(12.5)).text_color(surface::muted()).child(text.into()).into_any_element())
    }

    /// The Spaces of the per-Space tabs, to pick from.
    pub(crate) fn space_picker(&self, cx: &mut Context<Self>) -> Item {
        let b = self.browser.read(cx);
        let options: Vec<(SpaceId, SharedString)> = b
            .session
            .spaces()
            .iter()
            .filter(|s| !s.ephemeral)
            .map(|s| (s.id, SharedString::from(format!("{} {}", s.icon, s.name))))
            .collect();
        let el = div()
            .py(px(8.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(12.))
            .child(div().text_size(px(12.5)).text_color(surface::muted()).child("Space"))
            .child(self.segments_with("space-picker", options, self.space, |this, id, cx| this.pick_space(id, cx), cx));
        (None, el.into_any_element())
    }

    fn tabs_for(&self, cx: &mut Context<Self>) -> Vec<Item> {
        let s = self.browser.read(cx).settings.clone();
        match self.tab {
            Tab::General => self.general(&s, cx),
            Tab::Appearance => self.appearance(&s, cx),
            Tab::Search => self.search_tab(&s, cx),
            Tab::Privacy => self.privacy(&s, cx),
            Tab::Sites => self.sites(&s, cx),
            Tab::Passwords => self.passwords(cx),
            Tab::Extensions => self.extensions_tab(&s, cx),
            Tab::Spaces => self.spaces_tab(cx),
            Tab::Keyboard => self.keyboard(cx),
            Tab::System => self.system(&s, cx),
        }
    }

    fn render_results(&self, cx: &mut Context<Self>) -> Vec<Item> {
        let hits = self.results();
        if hits.is_empty() {
            return vec![self.note("No settings match.")];
        }
        let mut out = Vec::new();
        let mut group = None;
        for (i, row) in hits.into_iter().enumerate() {
            if group != Some(row.tab) {
                group = Some(row.tab);
                out.push((None, div().pt(px(14.)).pb(px(2.)).px(px(10.)).text_size(px(11.)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(surface::muted()).child(row.tab.title().to_uppercase()).into_any_element()));
            }
            let (tab, id) = (row.tab, row.id);
            let on = i == self.selected;
            let el = div()
                .id(("result", i))
                .debug_selector(|| format!("result:{id}"))
                .h(px(34.))
                .px(px(10.))
                .flex()
                .flex_none()
                .items_center()
                .rounded(r(8.))
                .bg(if on { surface::key() } else { gpui::transparent_black().into() })
                .hover(|d| d.bg(surface::key()))
                .on_click(cx.listener(move |this, _, _, cx| this.go_to(Some(tab), Some(id), cx)))
                .child(row.label);
            out.push((None, el.into_any_element()));
        }
        out
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pill = self.motion.get(("settings-pill", 0), Feel::Glide);
        let tabs = Tab::ALL.into_iter().enumerate().map(|(i, tab)| {
            let on = tab == self.tab && !self.searching();
            let key = ("settings-tab-button", i as u64);
            let h = self.motion.get(key, Feel::Hover);
            let ink = mix(surface::muted().into(), surface::ink().into(), if on { 1. } else { h });
            let el = div()
                .id(("settings-tab-button", i))
                .debug_selector(|| format!("tab:{}", tab.label()))
                .w(px(TAB_W))
                .h(px(52.))
                .flex_none()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .text_color(ink)
                .on_click(cx.listener(move |this, _, _, cx| this.select_tab(tab, cx)))
                .child(icon(tab.icon(), 18., mix(surface::icon().into(), if on { self.theme.accent } else { ink }, if on { 1. } else { h })))
                .child(div().text_size(px(11.)).child(tab.label()));
            self.feel(el, key, cx)
        }).collect::<Vec<_>>();
        let empty = TextField::default();
        let search = div()
            .w(px(170.))
            .h(px(28.))
            .px(px(9.))
            .flex()
            .items_center()
            .gap(px(7.))
            .rounded(r(8.))
            .border_1()
            .border_color(if self.active == Field::Search { self.theme.accent } else { surface::border().into() })
            .bg(surface::key())
            .id("settings-search")
            .debug_selector(|| "search".to_string())
            .text_size(px(12.5))
            .on_click(cx.listener(|this, _, _, cx| this.focus_field(Field::Search, cx)))
            .child(icon("search", 13., surface::icon().into()))
            .child(
                self.fields
                    .get(&Field::Search)
                    .unwrap_or(&empty)
                    .render("Search", surface::ink().into(), surface::muted().into(), Hsla { a: 0.25, ..self.theme.accent }),
            );
        div()
            .h(px(64.))
            .px(px(10.))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(surface::line())
            .child(
                div()
                    .relative()
                    .flex()
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .h(px(52.))
                            .w(px(TAB_W))
                            .left(px(TAB_W * pill))
                            .rounded(r(9.))
                            .bg(surface::key())
                            .opacity(if self.searching() { 0. } else { 1. }),
                    )
                    .children(tabs),
            )
            .child(div().flex_1())
            .child(search)
    }
}

impl<H: WebHost> gpui::Focusable for SettingsView<H> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<H: WebHost> Render for SettingsView<H> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = self.browser.read(cx).theme();
        crate::theme::set_ascii(self.browser.read(cx).settings.chrome == baa_model::ChromeStyle::Ascii);
        self.tick(window, cx);
        if !self.browser.read(cx).session.space(self.space).is_some_and(|s| !s.ephemeral) {
            let first = self.browser.read(cx).session.spaces().iter().find(|s| !s.ephemeral).map(|s| s.id);
            if let Some(first) = first {
                self.space = first;
            }
        }
        let items = if self.searching() { self.render_results(cx) } else { self.tabs_for(cx) };
        if let Some(id) = self.jump.take() {
            if let Some(i) = items.iter().position(|(row, _)| *row == Some(id)) {
                // The title is the body's first child.
                self.scroll.scroll_to_item(i + 1);
                self.flash = Some(id);
                self.motion.from(("settings-flash", 0), Feel::Glide, 1., 0.);
                window.request_animation_frame();
            }
        }
        let t = self.motion.get(("settings-tab", 0), Feel::Glide).clamp(0., 1.);
        let title = if self.searching() { "Search" } else { self.tab.title() };
        let sheet = self.render_clear_sheet(cx);
        let body = div()
            .id("settings-body")
            .debug_selector(|| "body".to_string())
            .flex_1()
            .min_h_0()
            .px(px(26.))
            .pb(px(24.))
            .mt(px(6. * (1. - t)))
            .opacity(t)
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .child(div().pt(px(16.)).pb(px(4.)).px(px(10.)).flex_none().text_size(px(20.)).font_weight(gpui::FontWeight::SEMIBOLD).child(title))
            .children(items.into_iter().map(|(_, el)| el));
        div()
            .key_context("Settings")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(gpui::Rgba { a: 1., ..surface::bg() })
            .text_color(surface::ink())
            .text_size(px(13.))
            .when(ascii(), |d| d.font_family(crate::theme::MONO).text_size(px(12.5)))
            .child(self.toolbar(cx))
            .child(body)
            .children(sheet)
    }
}
