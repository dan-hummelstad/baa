//! Sidebar (PLAN.md §3, pen.dev "Arc — Personal Space"): toolbar with room for
//! the traffic lights, URL bar, the Space's pinned and Today tabs with drag
//! reorder, New Tab, and the Space dots.

use std::time::Duration;

use baa_model::{Folder, SpaceId, Tab, TabId, Zone};
use baa_web::{PageHandle, WebHost};
use gpui::{
    div, img, prelude::*, pulsating_between, px, Animation, AnimationExt, AnyElement, Context,
    Hsla, MouseButton, SharedString, Stateful, StyledImage, Transformation, Window, Div,
};

use crate::{
    browser::{feel, shadow, tab_title, Browser, Ghost, List, MenuKind, SIDEBAR_WIDTH, SPACE_DOT},
    command_bar::Mode,
    icons::icon,
    motion::Feel,
    theme::{fade, mix, Theme},
};

const ROW_H: f32 = 38.;
/// Favourite tiles, four to a row (pen.dev "Favorite").
const FAV_W: f32 = 53.;
const FAV_H: f32 = 44.;

#[derive(Clone)]
pub(crate) struct DraggedTab {
    id: TabId,
    title: SharedString,
    theme: Theme,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(SIDEBAR_WIDTH - 22.))
            .h(px(ROW_H))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(px(10.))
            .bg(self.theme.glass_strong)
            .text_color(self.theme.ink)
            .text_size(px(13.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .child(div().truncate().child(self.title.clone()))
            // Lifts off the list: the shadow deepens as it lands under the pointer.
            .with_animation("lift", fade_in(), {
                let ink = self.theme.ink;
                move |d, t| {
                    d.opacity(0.7 + 0.25 * t)
                        .shadow(vec![shadow(Hsla { a: 0.22 * t, ..ink }, 2. + 6. * t, 6. + 14. * t)])
                }
            })
    }
}

impl<H: WebHost> Browser<H> {
    pub(crate) fn render_sidebar(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(space) = self.active_space().map(|s| s.id) else {
            return div().into_any_element();
        };
        let (space_icon, space_name) = self
            .active_space()
            .map(|s| (s.icon.clone(), s.name.clone()))
            .unwrap_or_default();

        let lists = div()
            .id("zones")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, e: &gpui::MouseDownEvent, _, cx| {
                    this.open_menu(MenuKind::Sidebar, e.position, cx)
                }),
            )
            .relative()
            .left(px(self.slide.value() * SIDEBAR_WIDTH))
            .opacity(1.0 - self.slide.value().abs().min(1.0) * 0.6)
            .pt(px(4.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex_none()
                    .h(px(20.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.5))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.ink_muted)
                    .child(div().text_size(px(11.)).child(SharedString::from(space_icon)))
                    .child(match self.renaming.as_ref().filter(|_| self.renaming_folder.is_none()) {
                        // Named in place: Enter keeps it, Esc leaves it as it was.
                        Some(field) => div()
                            .id("space-name")
                            .flex_1()
                            .min_w_0()
                            .px(px(4.))
                            .rounded(px(4.))
                            .bg(theme.glass_strong)
                            .text_color(theme.ink)
                            .track_focus(&self.input_focus)
                            .on_key_down(cx.listener(Self::on_input_key))
                            .child(field.render(
                                "Space name",
                                theme.ink,
                                theme.ink_muted,
                                Hsla { a: 0.3, ..theme.accent },
                            ))
                            .with_animation("rename-in", fade_in(), |d, t| d.opacity(t))
                            .into_any_element(),
                        None => div()
                            .id("space-name")
                            .on_click(cx.listener(|this, e: &gpui::ClickEvent, window, cx| {
                                if e.click_count() == 2 {
                                    this.start_rename(window, cx);
                                }
                            }))
                            .child(SharedString::from(space_name))
                            .into_any_element(),
                    }),
            )
            .children(
                self.session
                    .folders(space)
                    .map(|f| self.render_folder(f, theme, cx))
                    .collect::<Vec<_>>(),
            )
            .child(self.render_zone(space, Zone::Pinned, theme, cx))
            .child(
                div()
                    .flex_none()
                    .h(px(22.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .child(div().w_full().h(px(1.)).bg(theme.divider)),
            )
            .child(
                {
                    let key = ("new-tab", 0);
                    let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
                    feel(
                        div()
                            .id("new-tab")
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(10.))
                            .h(px(ROW_H))
                            .px(px(10.))
                            .mx(px(2.5 * p))
                            .rounded(px(10.))
                            .text_size(px(13.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(mix(theme.ink_muted, theme.ink, h))
                            .bg(fade(theme.hover, h))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_bar(Mode::NewTab, window, cx)),
                            )
                            // The plus turns a quarter as the pointer arrives.
                            .child(icon("plus", 16., mix(theme.ink_muted, theme.ink, h)).with_transformation(
                                Transformation::rotate(gpui::radians(h * std::f32::consts::FRAC_PI_2)),
                            ))
                            .child("New Tab"),
                        key,
                        cx,
                    )
                },
            )
            .child(self.render_zone(space, Zone::Today, theme, cx));

        div()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .pr(px(12.))
            .pb(px(4.))
            .pl(px(10.))
            .on_scroll_wheel(cx.listener(|this, e, window, cx| this.on_chrome_scroll(e, window, cx)))
            .child(self.render_favorites(space, theme, cx))
            .child(lists)
            .child(self.render_bottom_bar(space, theme, cx))
            .into_any_element()
    }

    /// Room for the traffic lights, then the sidebar toggle and back/forward/reload.
    pub(crate) fn render_window_controls(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self
            .active_tab()
            .and_then(|id| self.pages.get(&id))
            .map(|(p, _)| p.read(cx));
        let (back, fwd, loading) = page.map_or((false, false, false), |p| {
            (p.can_go_back(), p.can_go_forward(), p.is_loading())
        });
        let has_page = page.is_some();
        let spin = self.motion.get(("reload-spin", 0), Feel::Glide) * std::f32::consts::TAU;
        let button = |id: &'static str, name: &'static str, enabled: bool, cx: &mut Context<Self>| {
            // Back and forward lean the way they go; reload turns once per click.
            let lean = match id {
                "back" => -2.,
                "forward" => 2.,
                _ => 0.,
            };
            let turn = if id == "reload" { spin } else { 0. };
            self.icon_button(id, name, enabled, lean, turn, theme, cx)
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        // Back and forward: click to go, hold (or right-click) for history.
        let history = |id: &'static str, name: &'static str, enabled: bool, back: bool, cx: &mut Context<Self>| {
            button(id, name, enabled, cx).when(enabled, |d| {
                d.on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.nav_down(back, e.position, cx);
                    }),
                )
                .on_mouse_up(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.nav_up(cx)),
                )
                .on_mouse_down(
                    gpui::MouseButton::Right,
                    cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                        this.show_history(back, e.position, cx)
                    }),
                )
            })
        };
        let toggle = button("toggle-sidebar", "panel-left", true, cx).on_click(|_, window, cx| {
            window.dispatch_action(Box::new(crate::actions::ToggleSidebar), cx)
        });
        let back_button = history("back", "arrow-left", back, true, cx);
        let forward_button = history("forward", "arrow-right", fwd, false, cx);
        let reload = button("reload", if loading { "x" } else { "rotate-cw" }, has_page, cx).on_click(
            cx.listener(|this, _, window, cx| {
                this.motion.from(("reload-spin", 0), Feel::Glide, 0.0, 1.0);
                window.dispatch_action(Box::new(crate::actions::Reload), cx)
            }),
        );
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            // Traffic lights: 52 wide, then a 14 gap.
            .child(div().w(px(58.)))
            .child(toggle)
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(back_button)
                    .child(forward_button)
                    .child(reload),
            )
    }

    fn render_zone(
        &self,
        space: SpaceId,
        zone: Zone,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let rows: Vec<AnyElement> = self
            .session
            .tabs(space, zone)
            .enumerate()
            .map(|(i, tab)| self.render_tab_row(tab, i, theme, cx).into_any_element())
            .collect();
        let rows = self.with_ghosts(rows, (space, zone, None), theme);
        let hover = theme.glass;
        div()
            .id(match zone {
                Zone::Pinned => "pinned",
                Zone::Today => "today",
                Zone::Favorite => "favorites",
            })
            .flex()
            .flex_none()
            .flex_col()
            .gap(px(6.))
            // An empty zone still needs somewhere to drop.
            .min_h(px(8.))
            .rounded(px(10.))
            .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(hover))
            .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                this.session.move_tab(d.id, space, zone, usize::MAX);
                this.sync(window, cx);
            }))
            .children(rows)
    }

    /// `rows` with the rows that just left `list` collapsing at their old places.
    fn with_ghosts(&self, mut rows: Vec<AnyElement>, list: List, theme: &Theme) -> Vec<AnyElement> {
        let mut ghosts: Vec<&Ghost> = self.ghosts.iter().filter(|g| g.list == list).collect();
        ghosts.sort_by_key(|g| g.index);
        for g in ghosts {
            let t = self.motion.get(("ghost", g.id.0), Feel::Glide).clamp(0.0, 1.0);
            let image = g.favicon.as_ref().and_then(|url| self.favicons.get(url)).cloned();
            let row = div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(10.))
                .h(px(ROW_H * t))
                .mt(px(-6. * (1. - t)))
                .px(px(10.))
                .overflow_hidden()
                .opacity(t)
                .rounded(px(10.))
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(favicon(image, &g.title, false, 16., theme))
                .child(div().flex_1().min_w_0().truncate().child(g.title.clone()));
            rows.insert(g.index.min(rows.len()), row.into_any_element());
        }
        rows
    }

    /// An icon button: hover fades its well in, a press shrinks the icon.
    /// `lean` nudges the icon sideways on hover; `turn` rotates it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn icon_button(
        &self,
        id: &'static str,
        name: &'static str,
        enabled: bool,
        lean: f32,
        turn: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let key = (id, 0);
        let (h, p) = if enabled {
            (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press))
        } else {
            (0., 0.)
        };
        let scale = 1. - 0.18 * p;
        let color = if enabled { mix(theme.ink_muted, theme.ink, h) } else { theme.ink_faint };
        let el = div()
            .id(id)
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .bg(fade(theme.hover, h))
            .child(
                icon(name, 16., color).with_transformation(
                    Transformation::rotate(gpui::radians(turn))
                        .with_scaling(gpui::size(scale, scale))
                        .with_translation(gpui::point(px(lean * h), px(0.))),
                ),
            );
        feel(el, key, cx)
    }

    fn render_tab_row(
        &self,
        tab: &Tab,
        index: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = tab.id;
        let (space, zone, folder) = (tab.space, tab.zone, tab.folder);
        let title = SharedString::from(tab_title(tab).to_string());
        let loading = self
            .pages
            .get(&id)
            .is_some_and(|(p, _)| p.read(cx).is_loading());
        let accent = theme.accent;
        let drag = DraggedTab {
            id,
            title: title.clone(),
            theme: *theme,
        };
        let key = ("tab", id.0);
        let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
        let sel = self.motion.get(("sel", id.0), Feel::Glide).clamp(0.0, 1.0);
        // 1 while a row that just arrived is still growing in.
        let hidden = self.motion.get(("tab-in", id.0), Feel::Glide).clamp(0.0, 1.0);
        let close_key = ("tab-x", id.0);
        let (ch, cp) = (self.motion.get(close_key, Feel::Hover), self.motion.get(close_key, Feel::Press));

        let row = div()
            .id(("tab", id.0))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .h(px(ROW_H * (1. - hidden)))
            .mt(px(-6. * hidden))
            .opacity(1. - hidden)
            .overflow_hidden()
            .px(px(10.))
            .mx(px(2.5 * p))
            .rounded(px(10.))
            .text_size(px(13.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .bg(mix(fade(theme.hover, h), theme.glass_strong, sel))
            .when(sel > 0.0, |d| {
                d.shadow(vec![shadow(Hsla { a: 0.14 * sel, ..theme.ink }, 1., 3.)])
            })
            .on_click(cx.listener(move |this, _, window, cx| this.activate_tab(id, window, cx)))
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| this.close_tab_id(id, window, cx)),
            )
            .on_drag(drag, |d, _, _, cx| cx.new(|_| d.clone()))
            .drag_over::<DraggedTab>(move |s, _, _, _| s.border_t_2().border_color(accent))
            .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                match folder {
                    Some(folder) => this.session.move_to_folder(d.id, folder, index),
                    None => this.session.move_tab(d.id, space, zone, index),
                }
                this.sync(window, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_menu(MenuKind::Tab(id), e.position, cx);
                }),
            )
            .child(favicon(
                tab.favicon.as_ref().and_then(|url| self.favicons.get(url)).cloned(),
                &title,
                loading,
                16.,
                theme,
            ))
            .child(div().flex_1().min_w_0().truncate().child(title))
            .children(self.render_speaker(id, theme, cx))
            .child(feel(
                div()
                    .id(("close", id.0))
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .opacity(h.max(sel * 0.6))
                    .bg(fade(theme.glass_strong, ch))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_tab_id(id, window, cx);
                    }))
                    .child(icon("x", 14., mix(theme.ink_muted, theme.ink, ch)).with_transformation(
                        Transformation::scale(gpui::size(1. - 0.2 * cp, 1. - 0.2 * cp))
                            .with_rotation(gpui::radians(ch * std::f32::consts::FRAC_PI_2)),
                    )),
                close_key,
                cx,
            ));
        feel(row, key, cx)
    }

    /// A playing or muted tab's speaker; clicking it mutes or unmutes.
    fn render_speaker(&self, id: TabId, theme: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let muted = self.muted.contains(&id);
        if !muted && !self.audible.contains(&id) {
            return None;
        }
        let key = ("speaker", id.0);
        let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
        // Overshoots past 1 on a toggle: the icon pops.
        let pop = self.motion.get(("mute-pop", id.0), Feel::Glide);
        let pop = if pop == 0.0 { 1.0 } else { pop };
        let scale = pop * (1. - 0.2 * p);
        let el = div()
            .id(("speaker", id.0))
            .size(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.))
            .bg(fade(theme.glass_strong, h))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_mute(id, cx);
            }))
            .child(
                icon(if muted { "volume-x" } else { "volume-2" }, 14., mix(theme.ink_muted, theme.ink, h))
                    .with_transformation(Transformation::scale(gpui::size(scale, scale))),
            );
        Some(feel(el, key, cx))
    }

    /// Favourites: the same tiles in every Space. With none yet, a thin strip
    /// that opens into a row while a tab is dragged over it, so starting a
    /// drag never shifts the list.
    fn render_favorites(&self, space: SpaceId, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let favorites: Vec<&Tab> = self.session.tabs(space, Zone::Favorite).collect();
        let empty = favorites.is_empty();
        let active = self.active_tab();
        let (accent, glass) = (theme.accent, theme.glass);
        let tiles = favorites.into_iter().enumerate().map(|(i, tab)| {
            let id = tab.id;
            let selected = active == Some(id);
            let title = SharedString::from(tab_title(tab).to_string());
            let loading = self.pages.get(&id).is_some_and(|(p, _)| p.read(cx).is_loading());
            let drag = DraggedTab {
                id,
                title: title.clone(),
                theme: *theme,
            };
            let key = ("favorite", id.0);
            let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
            let sel = self.motion.get(("sel", id.0), Feel::Glide).clamp(0.0, 1.0);
            let hidden = self.motion.get(("tab-in", id.0), Feel::Glide).clamp(0.0, 1.0);
            // The tile shrinks inside a fixed cell, so pressing never moves its neighbours.
            let inset = 3. * p + 16. * hidden;
            let tile = div()
                .w(px(FAV_W - 2. * inset))
                .h(px(FAV_H - 2. * inset))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.))
                .opacity(1. - hidden)
                .bg(mix(mix(theme.glass, theme.glass_strong, 0.6 * h), theme.glass_strong, sel))
                .when(sel > 0.0 || selected, |d| {
                    d.shadow(vec![shadow(Hsla { a: 0.14 * sel, ..theme.ink }, 1., 3.)])
                })
                .child(favicon(
                    tab.favicon.as_ref().and_then(|url| self.favicons.get(url)).cloned(),
                    &title,
                    loading,
                    20. * (1. - 0.1 * p),
                    theme,
                ));
            let cell = div()
                .id(("favorite", id.0))
                .w(px(FAV_W))
                .h(px(FAV_H))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.))
                .child(tile)
                .on_click(cx.listener(move |this, _, window, cx| this.activate_tab(id, window, cx)))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                        this.open_menu(MenuKind::Tab(id), e.position, cx)
                    }),
                )
                .on_drag(drag, |d, _, _, cx| cx.new(|_| d.clone()))
                .drag_over::<DraggedTab>(move |s, _, _, _| s.border_2().border_color(accent))
                .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                    this.session.move_tab(d.id, space, Zone::Favorite, i);
                    this.sync(window, cx);
                }));
            feel(cell, key, cx)
        }).collect::<Vec<_>>();
        div()
            .id("favorites")
            .flex_none()
            .when(empty, |d| d.h(px(8.)))
            .when(!empty, |d| d.mt(px(4.)).mb(px(8.)))
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .rounded(px(10.))
            .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(glass).min_h(px(FAV_H)))
            .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                this.session.move_tab(d.id, space, Zone::Favorite, usize::MAX);
                this.sync(window, cx);
            }))
            .children(tiles)
    }

    /// A folder row, then its tabs indented unless it is collapsed (its active
    /// tab still shows, as in Arc). Opening and closing springs the height.
    fn render_folder(&self, folder: &Folder, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let id = folder.id;
        let open = !folder.collapsed;
        // Overshoot would grow past the rows, so the spring is clamped.
        let t = self
            .folder_motion
            .get(&id)
            .map_or(if open { 1.0 } else { 0.0 }, |s| s.value().clamp(0.0, 1.0));
        let animating = self.folder_motion.contains_key(&id);
        let active = self.active_tab();
        let rows: Vec<AnyElement> = self
            .session
            .folder_tabs(id)
            .enumerate()
            .filter(|(_, t)| open || animating || active == Some(t.id))
            .map(|(i, tab)| self.render_tab_row(tab, i, theme, cx).into_any_element())
            .collect();
        let rows = self.with_ghosts(rows, (folder.space, Zone::Pinned, Some(id)), theme);
        let n = rows.len() as f32;
        let full = 6. + n * ROW_H + (n - 1.).max(0.) * 6.;
        let name = match self.renaming.as_ref().filter(|_| self.renaming_folder == Some(id)) {
            Some(field) => div()
                .flex_1()
                .min_w_0()
                .px(px(4.))
                .rounded(px(4.))
                .bg(theme.glass_strong)
                .track_focus(&self.input_focus)
                .on_key_down(cx.listener(Self::on_input_key))
                .child(field.render("Folder name", theme.ink, theme.ink_muted, Hsla { a: 0.3, ..theme.accent }))
                .with_animation("rename-in", fade_in(), |d, t| d.opacity(t))
                .into_any_element(),
            None => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(SharedString::from(folder.name.clone()))
                .into_any_element(),
        };
        let glass = theme.glass;
        let key = ("folder", id.0);
        let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
        div()
            .flex()
            .flex_none()
            .flex_col()
            .child(feel(
                div()
                    .id(("folder", id.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.))
                    .h(px(ROW_H))
                    .px(px(10.))
                    .rounded(px(10.))
                    .mx(px(2.5 * p))
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .bg(fade(theme.hover, h))
                    .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                        // The first click of a double-click already toggled it.
                        let open = this.session.folder(id).is_some_and(|f| f.collapsed);
                        this.set_folder_open(id, open);
                        if e.click_count() == 2 {
                            this.start_folder_rename(id, window, cx);
                        }
                        this.sync(window, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.open_menu(MenuKind::Folder(id), e.position, cx);
                        }),
                    )
                    .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(glass))
                    .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                        this.session.move_to_folder(d.id, id, usize::MAX);
                        this.set_folder_open(id, true);
                        this.sync(window, cx);
                    }))
                    .child(icon(if open { "folder-open" } else { "folder" }, 16., theme.ink).with_transformation(
                        Transformation::scale(gpui::size(1. - 0.15 * p, 1. - 0.15 * p)),
                    ))
                    .child(name),
                key,
                cx,
            ))
            .when(!rows.is_empty(), |d| {
                d.child(
                    div()
                        .pt(px(6.))
                        .pl(px(14.))
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .when(animating, |d| d.h(px(full * t)).overflow_hidden().opacity(t))
                        .children(rows),
                )
            })
    }

    /// Library, the Space dots, and New Space.
    fn render_bottom_bar(
        &self,
        active: SpaceId,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let _ = active;
        let dots: Vec<_> = self
            .session
            .spaces()
            .iter()
            .map(|space| {
                let id = space.id;
                let key = ("space", id.0);
                let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
                let sel = self.motion.get(("space-sel", id.0), Feel::Glide).clamp(0.0, 1.0);
                let hover = theme.glass;
                let dot = div()
                    .id(("space", id.0))
                    .size(px(SPACE_DOT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .bg(fade(theme.hover, h * (1. - sel)))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.activate_space(id, window, cx)),
                    )
                    .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(hover))
                    .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                        this.session.move_tab(d.id, id, Zone::Today, 0);
                        this.sync(window, cx);
                    }))
                    .child(
                        div()
                            // Emoji are text: size stands in for scale.
                            .text_size(px(13. * (1. + 0.12 * h - 0.15 * p)))
                            .opacity(0.45 + 0.55 * sel.max(h * 0.6))
                            .child(SharedString::from(space.icon.clone())),
                    );
                feel(dot, key, cx)
            })
            .collect();
        // One pill that slides to the active Space rather than a highlight per dot.
        let pill = div()
            .absolute()
            .top_0()
            .left(px(self.motion.get(("space-pill", 0), Feel::Glide)))
            .size(px(SPACE_DOT))
            .rounded(px(6.))
            .bg(theme.glass);
        let plus_turn = self.motion.get(("add-space", 0), Feel::Hover) * std::f32::consts::FRAC_PI_2;
        let library = self
            .icon_button("library", "archive", true, 0., 0., theme, cx)
            .on_click(cx.listener(|this, _, window, cx| this.reopen_closed(window, cx)));
        let add = self
            .icon_button("add-space", "plus", true, 0., plus_turn, theme, cx)
            .on_click(cx.listener(|this, _, window, cx| this.new_space(window, cx)));
        div()
            .h(px(32.))
            .px(px(2.))
            .flex()
            .items_center()
            .child(library)
            .child(
                div()
                    .flex_1()
                    .flex()
                    .justify_center()
                    .child(div().relative().flex().child(pill).children(dots)),
            )
            .child(add)
    }
}

fn favicon(
    image: Option<std::sync::Arc<gpui::Image>>,
    title: &str,
    loading: bool,
    size: f32,
    theme: &Theme,
) -> AnyElement {
    let letter: SharedString = title
        .split("://")
        .last()
        .and_then(|s| s.trim_start_matches("www.").chars().next())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default()
        .into();
    let accent = theme.accent;
    let fallback = move || {
        div()
            .size(px(size))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(size / 4.))
            .bg(accent)
            .text_size(px(size * 0.56))
            .font_weight(gpui::FontWeight::BOLD)
            .text_color(gpui::white())
            .child(letter.clone())
            .into_any_element()
    };
    // The page fetches its favicon through its own Space's storage (gpui-cef).
    let icon = match image {
        Some(image) => img(image)
            .size(px(size))
            .rounded(px(size / 4.))
            .with_fallback(fallback)
            .into_any_element(),
        None => fallback(),
    };
    if !loading {
        return icon;
    }
    // with_animation honours gpui's reduce_motion on its own.
    div()
        .size(px(size))
        .child(icon)
        .with_animation(
            "loading",
            Animation::new(Duration::from_millis(900))
                .repeat()
                .with_easing(pulsating_between(0.25, 1.0)),
            |d, t| d.opacity(t),
        )
        .into_any_element()
}

/// Empty toolbar space: drags the window; a double-click zooms it (macOS).
pub(crate) fn drag_or_zoom(event: &gpui::MouseDownEvent, window: &mut Window, _: &mut gpui::App) {
    if event.click_count == 2 {
        window.zoom_window();
    } else {
        window.start_window_move();
    }
}

fn fade_in() -> Animation {
    Animation::new(Duration::from_millis(160)).with_easing(gpui::ease_out_quint())
}
