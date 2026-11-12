//! Sidebar (PLAN.md §3, pen.dev "Arc — Personal Space"): toolbar with room for
//! the traffic lights, URL bar, the Space's pinned and Today tabs with drag
//! reorder, New Tab, and the Space dots.

use crate::theme::r;
use std::time::Duration;

use baa_model::{Folder, FolderId, SpaceId, Tab, TabId, Zone};
use baa_web::{PageHandle, WebHost};
use gpui::{
    div, img, prelude::*, pulsating_between, px, Animation, AnimationExt, AnyElement, Context,
    DragMoveEvent, Hsla, MouseButton, SharedString, Stateful, StyledImage, Transformation, Window, Div,
};

use crate::{
    browser::{feel, shadow, tab_title, Browser, Ghost, List, MenuKind, SIDEBAR_WIDTH, SPACE_DOT},
    command_bar::Mode,
    icons::icon,
    motion::Feel,
    theme::{ascii, fade, mix, Theme},
};

/// Arc's rows, or ASCII Chrome's one-line rows.
fn row_h() -> f32 {
    if ascii() { 24. } else { 38. }
}
/// Favourite tiles share their row's width, at most this many to a row.
const FAV_PER_ROW: usize = 4;
const FAV_H: f32 = 44.;

#[derive(Clone)]
pub(crate) struct DraggedTab {
    pub(crate) id: TabId,
    title: SharedString,
    theme: Theme,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(SIDEBAR_WIDTH - 22.))
            .h(px(row_h()))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(r(10.))
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
    pub(crate) fn render_sidebar(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.library.is_some() {
            return self.render_library(theme, window, cx);
        }
        let Some(space) = self.active_space().map(|s| s.id) else {
            return div().into_any_element();
        };
        let (space_icon, space_name) = self
            .active_space()
            .map(|s| (s.icon.clone(), s.name.clone()))
            .unwrap_or_default();

        let today: List = (space, Zone::Today, None);
        let pinned: List = (space, Zone::Pinned, None);
        let lists = div()
            .id("zones")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            // One place takes the drop, wherever in the lists the pointer is: the tab lands
            // where the line shows, also from the gap between two rows.
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<DraggedTab>, _, cx| {
                if !e.bounds.contains(&e.event.position) {
                    this.aim_drop(None, cx);
                }
            }))
            .on_drop(cx.listener(|this, d: &DraggedTab, window, cx| this.drop_tab(d.id, window, cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, e: &gpui::MouseDownEvent, window, cx| {
                    this.open_menu(MenuKind::Sidebar, e.position, window, cx)
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
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_menu(MenuKind::Space(space), e.position, window, cx);
                        }),
                    )
                    .child(
                        div()
                            .id("space-icon")
                            .text_size(px(11.))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                                this.open_space_picker(space, true, e.position(), window, cx)
                            }))
                            .child(SharedString::from(space_icon)),
                    )
                    .child(match self.renaming.as_ref().filter(|_| self.renaming_folder.is_none()) {
                        // Named in place: Enter keeps it, Esc leaves it as it was.
                        Some(field) => div()
                            .id("space-name")
                            .flex_1()
                            .min_w_0()
                            .px(px(4.))
                            .rounded(r(4.))
                            .bg(theme.glass_strong)
                            .text_color(theme.ink)
                            .track_focus(&self.input_focus)
                            .on_key_down(cx.listener(Self::on_input_key))
                            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.commit_rename(window, cx)))
                            .child(field.render(
                                true,
                                "Space name",
                                theme.ink,
                                theme.ink_muted,
                                Hsla { a: 0.3, ..theme.accent },
                            ))
                            .with_animation("rename-in", rename_in(), |d, t| d.opacity(0.6 + 0.4 * t))
                            .into_any_element(),
                        None => div()
                            .id("space-name")
                            .on_click(cx.listener(|this, _, window, cx| {
                                if crate::motion::double_click(("space-name", 0)) {
                                    this.start_rename(window, cx);
                                }
                            }))
                            .child(SharedString::from(space_name))
                            .into_any_element(),
                    }),
            )
            .when(ascii(), |d| d.child(crate::theme::ascii::rule_label("PINNED", theme.ink_faint).flex_none().h(px(20.))))
            // Folders belong to the pinned section: one column, so they move as one.
            .child(
                div()
                    .flex()
                    .flex_none()
                    .flex_col()
                    .gap(px(6.))
                    .children(
                        self.session
                            .subfolders(space, None)
                            .map(|f| self.render_folder(f, 1.0, theme, cx))
                            .collect::<Vec<_>>(),
                    )
                    .child(self.render_zone(space, Zone::Pinned, theme, cx)),
            )
            .child(if ascii() {
                crate::theme::ascii::rule_label("TODAY", theme.ink_faint).debug_selector(|| "today-rule".into()).flex_none().h(px(20.)).mt(px(4.))
            } else {
                div()
                    .debug_selector(|| "today-rule".into())
                    .flex_none()
                    .h(px(22.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .child(div().w_full().h(px(1.)).bg(theme.divider))
            }
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, e: &gpui::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_menu(MenuKind::Today, e.position, window, cx);
                }),
            )
            // The whole divider is the end of Pinned.
            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<DraggedTab>, _, cx| {
                if e.bounds.contains(&e.event.position) {
                    this.aim_drop(Some((pinned, this.list_len(pinned))), cx);
                }
            })))
            .child(
                {
                    let key = ("new-tab", 0);
                    let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
                    feel(
                        div()
                            .id("new-tab")
                            .debug_selector(|| "new-tab".into())
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(10.))
                            .h(px(row_h()))
                            .px(px(10.))
                            .mx(px(2.5 * p))
                            .rounded(r(10.))
                            .text_size(px(13.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(mix(theme.ink_muted, theme.ink, h))
                            .bg(fade(theme.hover, h))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_bar(Mode::NewTab, window, cx)),
                            )
                            // New tabs open under this row, and a dropped one lands there too.
                            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<DraggedTab>, _, cx| {
                                if e.bounds.contains(&e.event.position) {
                                    this.aim_drop(Some((today, 0)), cx);
                                }
                            }))
                            // The plus turns a quarter as the pointer arrives.
                            .child(icon("plus", 16., mix(theme.ink_muted, theme.ink, h)).with_transformation(
                                Transformation::rotate(gpui::radians(h * std::f32::consts::FRAC_PI_2)),
                            ))
                            .child("New Tab")
                            .when(ascii(), |d| {
                                d.child(div().flex_1()).child(div().text_color(theme.ink_faint).child("⌘T"))
                            }),
                        key,
                        cx,
                    )
                },
            )
            .child(self.render_zone(space, Zone::Today, theme, cx))
            // Under the last tab: the end of Today.
            .child(div().flex_1().min_h(px(row_h())).on_drag_move(cx.listener(
                move |this, e: &DragMoveEvent<DraggedTab>, _, cx| {
                    if e.bounds.contains(&e.event.position) {
                        this.aim_drop(Some((today, this.list_len(today))), cx);
                    }
                },
            )));

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
            .children(self.render_mini_player(theme, cx))
            .children(self.render_agent_strip(theme, cx))
            .children(self.render_update_card(theme, cx))
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
        let rows: Vec<(f32, AnyElement)> = self
            .session
            .tabs(space, zone)
            .enumerate()
            .filter(|(_, t)| self.split_lead(t))
            .map(|(i, tab)| (self.row_shown(tab.id, 1.0), self.render_tab_entry(tab, i, 1.0, theme, cx)))
            .collect();
        let list: List = (space, zone, None);
        // An empty zone has no row to carry the line.
        let empty = rows.is_empty();
        let line = (empty && self.drop_slot(cx).is_some_and(|(l, _)| l == list)).then(|| drop_line(theme).top(px(3.)));
        let rows = stack(self.with_ghosts(rows, list, theme), false);
        div()
            .id(match zone {
                Zone::Pinned => "pinned",
                Zone::Today => "today",
                Zone::Favorite => "favorites",
            })
            .flex()
            .flex_none()
            .flex_col()
            // No gap: `stack` spaces the rows.
            // An empty zone still needs somewhere to drop.
            .min_h(px(8.))
            .relative()
            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<DraggedTab>, _, cx| {
                if empty && e.bounds.contains(&e.event.position) {
                    this.aim_drop(Some((list, 0)), cx);
                }
            }))
            .children(rows)
            .children(line)
    }

    /// `rows` with the rows that just left `list` collapsing at their old places.
    fn with_ghosts(&self, mut rows: Vec<(f32, AnyElement)>, list: List, theme: &Theme) -> Vec<(f32, AnyElement)> {
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
                .h(px(row_h() * t))
                .px(px(10.))
                .overflow_hidden()
                .opacity(t)
                .rounded(r(10.))
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(favicon(image, &g.title, false, 16., theme))
                .child(div().flex_1().min_w_0().truncate().child(g.title.clone()));
            rows.insert(g.index.min(rows.len()), (t, row.into_any_element()));
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
            .rounded(r(6.))
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

    /// How much of a row is in: below 1 while it grows in, or folds away
    /// (`shown`, a closing folder).
    fn row_shown(&self, id: TabId, shown: f32) -> f32 {
        let growing = self.motion.get(("tab-in", id.0), Feel::Glide).clamp(0.0, 1.0);
        shown.min(1. - growing)
    }

    /// The panes of the split `tab` is in.
    fn split_of(&self, tab: &Tab) -> Option<&[TabId]> {
        let split = self.session.space(tab.space)?.split.as_ref()?;
        split.tabs.contains(&tab.id).then_some(&split.tabs[..])
    }

    /// A split shows as one row where its first pane is; its other panes are left out.
    fn split_lead(&self, tab: &Tab) -> bool {
        self.split_of(tab).is_none_or(|panes| panes[0] == tab.id)
    }

    /// A tab's row, or its split's panes side by side in one row, as in Arc.
    // ponytail: the group row takes the lead pane's drop index; dropping above it puts a
    // tab next to that pane only.
    fn render_tab_entry(&self, tab: &Tab, index: usize, shown: f32, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        // The line sits in the gap above the row the tab would land before, or under the
        // last row of its list. ponytail: no line where the next place is a split's hidden
        // pane; the drop still lands there.
        let list: List = (tab.space, tab.zone, tab.folder);
        let line = self.drop_slot(cx).filter(|(l, _)| *l == list).and_then(|(_, slot)| {
            let at = px(-(ROW_GAP / 2.) - 1.);
            if slot == index {
                Some(drop_line(theme).top(at))
            } else if slot == index + 1 && slot == self.list_len(list) {
                Some(drop_line(theme).bottom(at))
            } else {
                None
            }
        });
        let Some(panes) = self.split_of(tab) else {
            let row = self.render_tab_row(tab, index, shown, theme, cx);
            return div().relative().child(row).children(line).into_any_element();
        };
        let mut rows = Vec::new();
        for t in panes.iter().filter_map(|id| self.session.tab(*id)) {
            rows.push(div().flex_1().min_w_0().child(self.render_tab_row(t, index, shown, theme, cx)));
        }
        div().relative().flex().gap(px(2.)).rounded(r(10.)).bg(theme.glass).children(rows).children(line).into_any_element()
    }

    fn render_tab_row(
        &self,
        tab: &Tab,
        index: usize,
        shown: f32,
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
        let hidden = 1. - self.row_shown(id, shown);
        let close_key = ("tab-x", id.0);
        let (ch, cp) = (self.motion.get(close_key, Feel::Hover), self.motion.get(close_key, Feel::Press));

        let row = div()
            .id(("tab", id.0))
            .debug_selector(move || format!("tab-row:{}", id.0))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .h(px(row_h() * (1. - hidden)))
            .opacity(1. - hidden)
            .overflow_hidden()
            .px(px(10.))
            .mx(px(2.5 * p))
            .rounded(r(10.))
            .text_size(px(13.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .bg(mix(fade(theme.hover, h), theme.glass_strong, sel))
            // Claude is driving it (PLAN-m4 D4: visible at all times).
            .when(self.agent.driven.contains(&id), |d| d.border_1().border_color(Hsla { a: 0.7, ..accent }))
            .when(sel > 0.0, |d| {
                d.shadow(vec![shadow(Hsla { a: 0.14 * sel, ..theme.ink }, 1., 3.)])
            })
            .when(ascii(), |d| {
                d.relative()
                    .gap(px(8.))
                    .font_weight(if sel > 0.5 { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL })
                    .text_color(mix(theme.ink_muted, theme.ink, sel.max(h)))
                    .child(div().absolute().left_0().opacity(sel).text_color(accent).child("▌"))
            })
            .on_click(cx.listener(move |this, _, window, cx| this.activate_tab(id, window, cx)))
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| this.close_tab_id(id, window, cx)),
            )
            .on_drag(drag, |d, _, _, cx| cx.new(|_| d.clone()))
            // Its upper half is the place before it, its lower half the place after: the end
            // of a list (Pinned, a folder, Today) is the lower half of its last row.
            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<DraggedTab>, _, cx| {
                if e.bounds.contains(&e.event.position) {
                    let after = e.event.position.y > e.bounds.center().y;
                    this.aim_drop(Some(((space, zone, folder), index + after as usize)), cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_menu(MenuKind::Tab(id), e.position, window, cx);
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
            .children(self.console_badge(("console-badge", id.0), &[id], cx))
            .child(feel(
                div()
                    .id(("close", id.0))
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(r(5.))
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
            .rounded(r(5.))
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
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded(r(10.))
                .opacity(1. - hidden)
                .bg(mix(mix(theme.glass, theme.glass_strong, 0.6 * h), theme.glass_strong, sel))
                .when(ascii(), |d| d.border_1().border_color(mix(crate::theme::ascii::rule().into(), accent, sel)))
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
                .debug_selector(move || format!("favorite-{i}"))
                .flex_1()
                .min_w_0()
                .h(px(FAV_H))
                .p(px(inset))
                .flex()
                .items_center()
                .justify_center()
                .rounded(r(10.))
                .child(tile)
                .on_click(cx.listener(move |this, _, window, cx| this.activate_tab(id, window, cx)))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                        this.open_menu(MenuKind::Tab(id), e.position, window, cx)
                    }),
                )
                .on_drag(drag, |d, _, _, cx| cx.new(|_| d.clone()))
                .drag_over::<DraggedTab>(move |s, _, _, _| s.border_2().border_color(accent))
                .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                    this.session.move_tab(d.id, space, Zone::Favorite, i);
                    this.sync(window, cx);
                }));
            feel(cell, key, cx).into_any_element()
        });
        let mut rows: Vec<Vec<AnyElement>> = Vec::new();
        for (i, tile) in tiles.enumerate() {
            if i % FAV_PER_ROW == 0 {
                rows.push(Vec::new());
            }
            rows.last_mut().expect("pushed above").push(tile);
        }
        let rows = rows.into_iter().map(|row| div().flex().gap(px(8.)).children(row));
        div()
            .id("favorites")
            .flex_none()
            .when(empty, |d| d.h(px(8.)))
            .when(!empty, |d| d.mt(px(4.)).mb(px(8.)))
            .flex()
            .flex_col()
            .gap(px(8.))
            .rounded(r(10.))
            .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(glass).min_h(px(FAV_H)))
            .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                this.session.move_tab(d.id, space, Zone::Favorite, usize::MAX);
                this.sync(window, cx);
            }))
            .children(rows)
    }

    /// A folder row, then its subfolders and tabs indented unless it is collapsed (its
    /// active tab still shows, as in Arc). Opening and closing springs the height.
    /// The active tab's folder and every folder above it.
    fn active_folders(&self) -> Vec<FolderId> {
        let mut chain = Vec::new();
        let mut at = self.active_tab().and_then(|t| self.session.tab(t)).and_then(|t| t.folder);
        while let Some(id) = at.filter(|id| !chain.contains(id)) {
            chain.push(id);
            at = self.session.folder(id).and_then(|f| f.parent);
        }
        chain
    }

    /// `shown`: how far the folder's parent shows it (1 at the top level). A subfolder
    /// folds with its parent: its row and tabs go as the parent's spring closes.
    fn render_folder(&self, folder: &Folder, shown: f32, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (id, top) = (folder.id, folder.parent.is_none());
        let open = !folder.collapsed;
        // Overshoot would grow past the rows, so the spring is clamped.
        let t = self
            .folder_motion
            .get(&id)
            .map_or(if open { 1.0 } else { 0.0 }, |s| s.value().clamp(0.0, 1.0));
        // How far its contents show: its own spring, within what its parent shows.
        let inner = t.min(shown);
        let active = self.active_tab();
        // The folders holding the active tab: they stay to show it, however closed.
        let holds_active = self.active_folders();
        let rows: Vec<(f32, AnyElement)> = self
            .session
            .folder_tabs(id)
            .enumerate()
            .filter(|(_, t)| (inner > 0.0 || active == Some(t.id)) && self.split_lead(t))
            // The active tab stays put while the rest fold around it.
            .map(|(i, tab)| {
                let shown = if active == Some(tab.id) { 1.0 } else { inner };
                (self.row_shown(tab.id, shown), self.render_tab_entry(tab, i, shown, theme, cx))
            })
            .collect();
        let mut entries: Vec<(f32, AnyElement)> = self
            .session
            .subfolders(folder.space, Some(id))
            .filter(|f| inner > 0.0 || holds_active.contains(&f.id))
            .map(|f| {
                let gap = if holds_active.contains(&f.id) { 1.0 } else { inner };
                (gap, self.render_folder(f, inner, theme, cx))
            })
            .collect();
        entries.extend(self.with_ghosts(rows, (folder.space, Zone::Pinned, Some(id)), theme));
        let rows = stack(entries, true);
        let error = self.live_errors.get(&id).filter(|_| open).map(|e| {
            div()
                .px(px(10.))
                .py(px(4.))
                .text_size(px(11.))
                .text_color(theme.ink_muted)
                .child(SharedString::from(e.clone()))
        });
        let glyph = match &folder.live {
            Some(baa_model::LiveSource::GitHub { .. }) => "git-pull-request",
            Some(baa_model::LiveSource::Feed { .. }) => "rss",
            Some(baa_model::LiveSource::DevServers) => "terminal",
            None if open => "folder-open",
            None => "folder",
        };
        let name =match self.renaming.as_ref().filter(|_| self.renaming_folder == Some(id)) {
            Some(field) => div()
                .flex_1()
                .min_w_0()
                .px(px(4.))
                .rounded(r(4.))
                .bg(theme.glass_strong)
                .track_focus(&self.input_focus)
                .on_key_down(cx.listener(Self::on_input_key))
                            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.commit_rename(window, cx)))
                .child(field.render(true, "Folder name", theme.ink, theme.ink_muted, Hsla { a: 0.3, ..theme.accent }))
                .with_animation("rename-in", rename_in(), |d, t| d.opacity(0.6 + 0.4 * t))
                .into_any_element(),
            None => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(SharedString::from(folder.name.clone()))
                .into_any_element(),
        };
        // A closed dev servers folder still shows its servers' console errors.
        let badge = (!open && matches!(folder.live, Some(baa_model::LiveSource::DevServers))).then(|| {
            let tabs: Vec<TabId> = self.session.folder_tabs(id).map(|t| t.id).collect();
            self.console_badge(("folder-console-badge", id.0), &tabs, cx)
        });
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
                    .debug_selector(move || if top { "folder-row" } else { "subfolder-row" }.into())
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.))
                    .h(px(row_h() * shown))
                    .opacity(shown)
                    .overflow_hidden()
                    .px(px(10.))
                    .rounded(r(10.))
                    .mx(px(2.5 * p))
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .bg(fade(theme.hover, h))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // The first click of a double-click already toggled it.
                        let open = this.session.folder(id).is_some_and(|f| f.collapsed);
                        this.set_folder_open(id, open);
                        if crate::motion::double_click(("folder", id.0)) {
                            this.start_folder_rename(id, window, cx);
                        }
                        this.sync(window, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_menu(MenuKind::Folder(id), e.position, window, cx);
                        }),
                    )
                    // Over the folder itself the tab goes into it (its end): no line elsewhere.
                    .on_drag_move(cx.listener(|this, e: &DragMoveEvent<DraggedTab>, _, cx| {
                        if e.bounds.contains(&e.event.position) {
                            this.aim_drop(None, cx);
                        }
                    }))
                    .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(glass))
                    .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                        this.session.move_to_folder(d.id, id, usize::MAX);
                        this.set_folder_open(id, true);
                        this.sync(window, cx);
                    }))
                    .child(icon(glyph, 16., theme.ink).with_transformation(
                        Transformation::scale(gpui::size(1. - 0.15 * p, 1. - 0.15 * p)),
                    ))
                    .child(name)
                    .children(badge.flatten()),
                key,
                cx,
            ))
            .when(!rows.is_empty() || error.is_some(), |d| {
                d.child(
                    div()
                        .pl(px(14.))
                        .flex()
                        .flex_col()
                        .debug_selector(move || if top { "folder-rows" } else { "subfolder-rows" }.into())
                        .children(error)
                        .children(rows),
                )
            })
            .into_any_element()
    }

    /// Tabs in `list`, split panes and all: the place after its last one.
    fn list_len(&self, (space, zone, folder): List) -> usize {
        match folder {
            Some(folder) => self.session.folder_tabs(folder).count(),
            None => self.session.tabs(space, zone).count(),
        }
    }

    /// The slot to draw a line at: only while something is being dragged.
    fn drop_slot(&self, cx: &Context<Self>) -> Option<(List, usize)> {
        self.drop_at.filter(|_| cx.has_active_drag())
    }

    pub(crate) fn aim_drop(&mut self, at: Option<(List, usize)>, cx: &mut Context<Self>) {
        if self.drop_at != at {
            self.drop_at = at;
            cx.notify();
        }
    }

    /// Lands the dragged tab where the line shows; nowhere when there is none.
    pub(crate) fn drop_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some((list, slot)) = self.drop_at.take() else { return };
        let (space, zone, folder) = list;
        let here = self.session.tab(id).filter(|t| (t.space, t.zone, t.folder) == list).is_some();
        let from = here
            .then(|| match folder {
                Some(folder) => self.session.folder_tabs(folder).position(|t| t.id == id),
                None => self.session.tabs(space, zone).position(|t| t.id == id),
            })
            .flatten();
        let index = landing(slot, from);
        match folder {
            Some(folder) => self.session.move_to_folder(id, folder, index),
            None => self.session.move_tab(id, space, zone, index),
        }
        self.sync(window, cx);
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
                    .rounded(r(6.))
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
            .rounded(r(6.))
            .bg(theme.glass)
            .when(ascii(), |d| {
                let bracket = |c: &'static str| div().absolute().top(px(3.)).text_color(theme.accent).child(c);
                d.bg(theme.glass_strong)
                    .child(bracket("[").left(px(-4.)))
                    .child(bracket("]").right(px(-4.)))
            });
        let plus_turn = self.motion.get(("add-space", 0), Feel::Hover) * std::f32::consts::FRAC_PI_2;
        // A download's icon is thrown at this button; it hops when one sinks in.
        let at = self.library_at.clone();
        let hop = self.motion.get(("download-hop", 1), Feel::Glide);
        let library = self
            .icon_button("library", "archive", true, 0., 0., theme, cx)
            .relative()
            .top(px(-4. * hop))
            .child(gpui::canvas(move |bounds, _, _| at.set(bounds), |_, _, _, _| {}).absolute().size_full())
            .on_click(cx.listener(|this, _, window, cx| this.open_library_last(window, cx)));
        let add = self
            .icon_button("add-space", "plus", true, 0., plus_turn, theme, cx)
            .on_click(cx.listener(|this, _, window, cx| this.new_space(window, cx)));
        div()
            .h(px(32.))
            .px(px(2.))
            .flex()
            .items_center()
            .when(ascii(), |d| d.border_t_1().border_color(crate::theme::ascii::rule()))
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

/// Rows 6px apart, each `(shown, row)` bringing the gap above it scaled by
/// how far it and the rows before it are in, so a growing or folding row moves
/// everything below at its own pace. `lead`: a gap above the first row too.
/// (Margins only ever grow: gpui's layout keeps a list's height stale when its
/// rows pull a gap back with a negative margin, and what is below then lags.)
/// Where a tab dropped on `slot` goes once it is lifted out of the list: slots count the
/// list as drawn, the dragged tab (at `from`, if it is in this list) included.
pub(crate) fn landing(slot: usize, from: Option<usize>) -> usize {
    slot - from.is_some_and(|from| from < slot) as usize
}

/// The insertion line, for the gap between two rows; the caller places it.
fn drop_line(theme: &Theme) -> Div {
    div().debug_selector(|| "drop-line".into()).absolute().left(px(6.)).right(px(6.)).h(px(2.)).rounded_full().bg(theme.accent)
}

fn stack(rows: Vec<(f32, AnyElement)>, lead: bool) -> Vec<AnyElement> {
    let mut before: f32 = if lead { 1. } else { 0. };
    rows.into_iter()
        .map(|(shown, row)| {
            let gap = ROW_GAP * shown * before;
            before = before.max(shown);
            div().flex_none().mt(px(gap)).child(row).into_any_element()
        })
        .collect()
}

const ROW_GAP: f32 = 6.;

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
            .rounded(r(size / 4.))
            .bg(accent)
            .text_size(px(size * 0.56))
            .font_weight(gpui::FontWeight::BOLD)
            .text_color(gpui::white())
            .child(letter.clone())
            .into_any_element()
    };
    // The page fetches its favicon through its own Space's storage.
    let icon = match image {
        Some(image) => img(image)
            .size(px(size))
            .rounded(r(size / 4.))
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
pub(crate) fn drag_or_zoom(_: &gpui::MouseDownEvent, window: &mut Window, _: &mut gpui::App) {
    if crate::motion::double_click(("toolbar", 0)) {
        window.zoom_window();
    } else {
        window.start_window_move();
    }
}

/// The name turns editable at once: a short settle, never blinking out.
fn rename_in() -> Animation {
    Animation::new(Duration::from_millis(80)).with_easing(gpui::ease_out_quint())
}

fn fade_in() -> Animation {
    Animation::new(Duration::from_millis(160)).with_easing(gpui::ease_out_quint())
}
