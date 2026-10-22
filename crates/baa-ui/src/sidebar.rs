//! Sidebar (PLAN.md §3, pen.dev "Arc — Personal Space"): toolbar with room for
//! the traffic lights, URL bar, the Space's pinned and Today tabs with drag
//! reorder, New Tab, and the Space dots.

use std::time::Duration;

use baa_model::{Folder, SpaceId, Tab, TabId, Zone};
use baa_web::{PageHandle, WebHost};
use gpui::{
    div, img, prelude::*, pulsating_between, px, Animation, AnimationExt, AnyElement, Context,
    Hsla, MouseButton, SharedString, StyledImage, Window,
};

use crate::{
    browser::{shadow, tab_title, Browser, MenuKind, SIDEBAR_WIDTH},
    command_bar::Mode,
    icons::icon,
    theme::Theme,
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
            .shadow(vec![shadow(Hsla { a: 0.2, ..self.theme.ink }, 4., 12.)])
            .opacity(0.95)
            .child(div().truncate().child(self.title.clone()))
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
                div()
                    .id("new-tab")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.))
                    .h(px(ROW_H))
                    .px(px(10.))
                    .rounded(px(10.))
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.ink_muted)
                    .hover(|s| s.bg(theme.hover))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_bar(Mode::NewTab, window, cx)),
                    )
                    .child(icon("plus", 16., theme.ink_muted))
                    .child("New Tab"),
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
        let button = |id: &'static str, name: &'static str, enabled: bool| {
            div()
                .id(id)
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .when(enabled, |d| d.hover(|s| s.bg(theme.hover)))
                .child(icon(
                    name,
                    16.,
                    if enabled { theme.ink_muted } else { theme.ink_faint },
                ))
        };
        // Back and forward: click to go, hold (or right-click) for history.
        let history = |id: &'static str, name: &'static str, enabled: bool, back: bool| {
            button(id, name, enabled).when(enabled, |d| {
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
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            // Traffic lights: 52 wide, then a 14 gap.
            .child(div().w(px(58.)))
            .child(button("toggle-sidebar", "panel-left", true).on_click(|_, window, cx| {
                window.dispatch_action(Box::new(crate::actions::ToggleSidebar), cx)
            }))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(history("back", "arrow-left", back, true))
                    .child(history("forward", "arrow-right", fwd, false))
                    .child(
                        button(
                            "reload",
                            if loading { "x" } else { "rotate-cw" },
                            page.is_some(),
                        )
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(crate::actions::Reload), cx)
                        }),
                    ),
            )
    }

    fn render_zone(
        &self,
        space: SpaceId,
        zone: Zone,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = self.active_tab();
        let rows: Vec<_> = self
            .session
            .tabs(space, zone)
            .enumerate()
            .map(|(i, tab)| self.render_tab_row(tab, i, active, theme, cx))
            .collect();
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

    fn render_tab_row(
        &self,
        tab: &Tab,
        index: usize,
        active: Option<TabId>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = tab.id;
        let (space, zone, folder) = (tab.space, tab.zone, tab.folder);
        let title = SharedString::from(tab_title(tab).to_string());
        let group = SharedString::from(format!("tab-{}", id.0));
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
        let selected = active == Some(id);

        div()
            .id(("tab", id.0))
            .group(group.clone())
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .h(px(ROW_H))
            .px(px(10.))
            .rounded(px(10.))
            .text_size(px(13.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .when(selected, |d| {
                d.bg(theme.glass_strong)
                    .shadow(vec![shadow(Hsla { a: 0.14, ..theme.ink }, 1., 3.)])
            })
            .when(!selected, |d| d.hover(|s| s.bg(theme.hover)))
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
            .child(
                div()
                    .id(("close", id.0))
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .opacity(0.)
                    .group_hover(group, |s| s.opacity(1.))
                    .hover(|s| s.bg(theme.glass))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_tab_id(id, window, cx);
                    }))
                    .child(icon("x", 14., theme.ink_muted)),
            )
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
            div()
                .id(("favorite", id.0))
                .w(px(FAV_W))
                .h(px(FAV_H))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.))
                .bg(theme.glass)
                .when(selected, |d| {
                    d.bg(theme.glass_strong)
                        .shadow(vec![shadow(Hsla { a: 0.14, ..theme.ink }, 1., 3.)])
                })
                .when(!selected, |d| d.hover(|s| s.bg(theme.glass_strong)))
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
                }))
                .child(favicon(
                    tab.favicon.as_ref().and_then(|url| self.favicons.get(url)).cloned(),
                    &title,
                    loading,
                    20.,
                    theme,
                ))
        });
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
    /// tab still shows, as in Arc).
    fn render_folder(&self, folder: &Folder, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let id = folder.id;
        let open = !folder.collapsed;
        let active = self.active_tab();
        let rows: Vec<_> = self
            .session
            .folder_tabs(id)
            .enumerate()
            .filter(|(_, t)| open || active == Some(t.id))
            .map(|(i, tab)| self.render_tab_row(tab, i, active, theme, cx))
            .collect();
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
                .into_any_element(),
            None => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(SharedString::from(folder.name.clone()))
                .into_any_element(),
        };
        let glass = theme.glass;
        div()
            .flex()
            .flex_none()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .id(("folder", id.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.))
                    .h(px(ROW_H))
                    .px(px(10.))
                    .rounded(px(10.))
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .hover(|s| s.bg(theme.hover))
                    .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                        // The first click of a double-click already toggled it.
                        if let Some(f) = this.session.folder_mut(id) {
                            f.collapsed = !f.collapsed;
                        }
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
                        if let Some(f) = this.session.folder_mut(id) {
                            f.collapsed = false;
                        }
                        this.sync(window, cx);
                    }))
                    .child(icon(if open { "folder-open" } else { "folder" }, 16., theme.ink))
                    .child(name),
            )
            .when(!rows.is_empty(), |d| {
                d.child(div().pl(px(14.)).flex().flex_col().gap(px(6.)).children(rows))
            })
    }

    /// Library, the Space dots, and New Space.
    fn render_bottom_bar(
        &self,
        active: SpaceId,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let dots = self.session.spaces().iter().map(|space| {
            let id = space.id;
            let hover = theme.glass;
            div()
                .id(("space", id.0))
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .when(id == active, |d| d.bg(theme.glass))
                .hover(|s| s.bg(hover))
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
                        .text_size(px(13.))
                        .when(id != active, |d| d.opacity(0.45))
                        .child(SharedString::from(space.icon.clone())),
                )
        });
        let button = |id: &'static str, name: &'static str| {
            div()
                .id(id)
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .hover(|s| s.bg(theme.hover))
                .child(icon(name, 16., theme.ink_muted))
        };
        div()
            .h(px(32.))
            .px(px(2.))
            .flex()
            .items_center()
            .child(button("library", "archive").on_click(
                cx.listener(|this, _, window, cx| this.reopen_closed(window, cx)),
            ))
            .child(div().flex_1().flex().justify_center().children(dots))
            .child(
                button("add-space", "plus")
                    .on_click(cx.listener(|this, _, window, cx| this.new_space(window, cx))),
            )
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
