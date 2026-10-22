//! Sidebar (PLAN.md §3, pen.dev "Arc — Personal Space"): toolbar with room for
//! the traffic lights, URL bar, the Space's pinned and Today tabs with drag
//! reorder, New Tab, and the Space dots.

use std::time::Duration;

use baa_model::{SpaceId, Tab, TabId, Zone};
use baa_web::{PageHandle, WebHost};
use gpui::{
    div, img, prelude::*, pulsating_between, px, Animation, AnimationExt, AnyElement, Context,
    Hsla, MouseButton, SharedString, StyledImage, Window,
};

use crate::{
    browser::{host_of, shadow, tab_title, Browser, SIDEBAR_WIDTH},
    command_bar::Mode,
    icons::icon,
    theme::Theme,
};

const ROW_H: f32 = 38.;

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
            .relative()
            .left(px(self.slide.value() * SIDEBAR_WIDTH))
            .opacity(1.0 - self.slide.value().abs().min(1.0) * 0.6)
            .pt(px(18.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .h(px(20.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.5))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.ink_muted)
                    .child(div().text_size(px(11.)).child(SharedString::from(space_icon)))
                    .child(SharedString::from(space_name)),
            )
            .child(self.render_zone(space, Zone::Pinned, theme, cx))
            .child(
                div()
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
                    .hover(|s| s.bg(theme.glass))
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
            .pt(px(5.))
            .pr(px(12.))
            .pb(px(4.))
            .pl(px(10.))
            .child(self.render_toolbar(theme, cx))
            .child(div().pt(px(12.)).child(self.render_url_bar(theme, cx)))
            .child(lists)
            .child(self.render_bottom_bar(space, theme, cx))
            .into_any_element()
    }

    /// Room for the traffic lights, the sidebar toggle, and back/forward/reload.
    /// The empty part drags the window.
    fn render_toolbar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self
            .active_tab()
            .and_then(|id| self.pages.get(&id))
            .map(|(p, _)| p.read(cx));
        let (back, fwd, loading) = page.map_or((false, false, false), |p| {
            (p.can_go_back(), p.can_go_forward(), p.is_loading())
        });
        let button = |id: &'static str,
                      name: &'static str,
                      enabled: bool,
                      action: Box<dyn gpui::Action>| {
            div()
                .id(id)
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .when(enabled, |d| {
                    d.hover(|s| s.bg(theme.glass))
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(action.boxed_clone(), cx)
                        })
                })
                .child(icon(
                    name,
                    16.,
                    if enabled { theme.ink_muted } else { theme.ink_faint },
                ))
        };
        div()
            .id("toolbar")
            .h(px(32.))
            .pl(px(6.))
            .pr(px(1.))
            .flex()
            .items_center()
            .on_mouse_down(gpui::MouseButton::Left, |_, window, _| window.start_window_move())
            // Traffic lights: 52 wide, then the same 14 gap the mock leaves.
            .child(div().w(px(62.)))
            .child(button(
                "toggle-sidebar",
                "panel-left",
                true,
                Box::new(crate::actions::ToggleSidebar),
            ))
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .child(button("back", "arrow-left", back, Box::new(crate::actions::Back)))
                    .child(button(
                        "forward",
                        "arrow-right",
                        fwd,
                        Box::new(crate::actions::Forward),
                    ))
                    .child(button(
                        "reload",
                        if loading { "x" } else { "rotate-cw" },
                        page.is_some(),
                        Box::new(crate::actions::Reload),
                    )),
            )
    }

    /// Arc's URL bar: the host; click edits the full URL in the command bar.
    fn render_url_bar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let url = self
            .active_tab()
            .and_then(|id| self.session.tab(id))
            .map(|t| t.url.clone())
            .unwrap_or_default();
        let host = host_of(&url);
        let copy = url.clone();
        div()
            .id("url-bar")
            .h(px(34.))
            .pl(px(12.))
            .pr(px(10.))
            .flex()
            .items_center()
            .gap(px(12.))
            .rounded(px(10.))
            .bg(theme.glass)
            .text_size(px(13.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(if host.is_empty() { theme.ink_muted } else { theme.ink })
            .hover(|s| s.bg(Hsla { a: theme.glass.a * 1.5, ..theme.glass }))
            .on_click(cx.listener(|this, _, window, cx| this.open_bar(Mode::Address, window, cx)))
            .child(div().flex_1().min_w_0().truncate().child(if host.is_empty() {
                "Search or enter address".into()
            } else {
                host
            }))
            .when(!url.is_empty(), |d| {
                d.child(
                    div()
                        .id("copy-link")
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy.clone()));
                        })
                        .child(icon("link", 14., theme.ink_muted)),
                )
            })
            .child(icon("sliders-horizontal", 14., theme.ink_muted))
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
            })
            .flex()
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
        let (space, zone) = (tab.space, tab.zone);
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
            .when(!selected, |d| d.hover(|s| s.bg(theme.glass)))
            .on_click(cx.listener(move |this, _, window, cx| this.activate_tab(id, window, cx)))
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| this.close_tab_id(id, window, cx)),
            )
            .on_drag(drag, |d, _, _, cx| cx.new(|_| d.clone()))
            .drag_over::<DraggedTab>(move |s, _, _, _| s.border_t_2().border_color(accent))
            .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                this.session.move_tab(d.id, space, zone, index);
                this.sync(window, cx);
            }))
            .child(favicon(tab.favicon.clone(), &title, loading, theme))
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

    /// Library, the Space dots, and New Space.
    fn render_bottom_bar(
        &self,
        active: SpaceId,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let dots = self.session.spaces().iter().map(|space| {
            let id = space.id;
            let ink = theme.ink;
            let hover = theme.glass;
            div()
                .id(("space", id.0))
                .size(px(18.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(|s| s.bg(hover))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.activate_space(id, window, cx)),
                )
                .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(hover))
                .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                    this.session.move_tab(d.id, id, Zone::Today, 0);
                    this.sync(window, cx);
                }))
                .child(div().size(px(6.)).rounded_full().bg(if id == active {
                    ink
                } else {
                    theme.ink_faint
                }))
        });
        let button = |id: &'static str, name: &'static str| {
            div()
                .id(id)
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .hover(|s| s.bg(theme.glass))
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

fn favicon(url: Option<String>, title: &str, loading: bool, theme: &Theme) -> AnyElement {
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
            .size(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .bg(accent)
            .text_size(px(9.))
            .font_weight(gpui::FontWeight::BOLD)
            .text_color(gpui::white())
            .child(letter.clone())
            .into_any_element()
    };
    let icon = match url {
        // ponytail: favicons load via gpui's HTTP client (none set = letter fallback), not the
        // Space's CEF context; fine for public icons, wrong for icons behind a login.
        Some(url) => img(url)
            .size(px(16.))
            .rounded(px(4.))
            .with_fallback(fallback)
            .into_any_element(),
        None => fallback(),
    };
    if !loading {
        return icon;
    }
    // with_animation honours gpui's reduce_motion on its own.
    div()
        .size(px(16.))
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
