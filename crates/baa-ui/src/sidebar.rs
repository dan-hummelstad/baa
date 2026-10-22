//! Sidebar (PLAN.md §3): nav buttons, address pill, Pinned and Today zones with
//! drag reorder, New Tab, and the Space switcher strip.

use std::time::Duration;

use baa_model::{SpaceId, Tab, TabId, Zone};
use baa_web::{PageHandle, WebHost};
use gpui::{
    div, img, prelude::*, pulsating_between, px, Animation, AnimationExt, AnyElement, Context,
    Hsla, MouseButton, SharedString, StyledImage, Window,
};

use crate::{
    browser::{tab_title, Browser, SIDEBAR_WIDTH},
    command_bar::Mode,
    theme::Theme,
};

#[derive(Clone)]
pub(crate) struct DraggedTab {
    id: TabId,
    title: SharedString,
    theme: Theme,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(SIDEBAR_WIDTH - 16.))
            .h(px(32.))
            .px_2()
            .flex()
            .items_center()
            .rounded_md()
            .bg(self.theme.chrome_active)
            .text_color(self.theme.text)
            .text_sm()
            .shadow_lg()
            .opacity(0.9)
            .child(div().truncate().child(self.title.clone()))
    }
}

impl<H: WebHost> Browser<H> {
    pub(crate) fn render_sidebar(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(space) = self.active_space().map(|s| s.id) else {
            return div().into_any_element();
        };

        let lists = div()
            .id("zones")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .relative()
            .left(px(self.slide.value() * SIDEBAR_WIDTH))
            .opacity(1.0 - self.slide.value().abs().min(1.0) * 0.6)
            .flex()
            .flex_col()
            .gap_1()
            .child(self.render_zone(space, Zone::Pinned, theme, cx))
            .child(div().mx_2().my_1().h(px(1.)).bg(theme.border))
            .child(
                div()
                    .id("new-tab")
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(32.))
                    .px_2()
                    .rounded_md()
                    .text_sm()
                    .text_color(theme.text_dim)
                    .hover(|s| s.bg(theme.chrome_hover))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_bar(Mode::NewTab, window, cx)),
                    )
                    .child(div().w(px(16.)).flex().justify_center().child("+"))
                    .child("New Tab"),
            )
            .child(self.render_zone(space, Zone::Today, theme, cx));

        div()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .child(self.render_nav_row(theme, cx))
            .child(self.render_address_pill(theme, cx))
            .child(lists)
            .child(self.render_space_strip(space, theme, cx))
            .into_any_element()
    }

    fn render_nav_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self
            .active_tab()
            .and_then(|id| self.pages.get(&id))
            .map(|(p, _)| p.read(cx));
        let (back, fwd, loading) = page.map_or((false, false, false), |p| {
            (p.can_go_back(), p.can_go_forward(), p.is_loading())
        });
        let button = |id: &'static str,
                      label: &'static str,
                      enabled: bool,
                      action: Box<dyn gpui::Action>| {
            div()
                .id(id)
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .text_color(if enabled { theme.text } else { theme.text_dim })
                .when(enabled, |d| {
                    d.hover(|s| s.bg(theme.chrome_hover))
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(action.boxed_clone(), cx)
                        })
                })
                .child(label)
        };
        div()
            .flex()
            .gap_1()
            .child(button("back", "←", back, Box::new(crate::actions::Back)))
            .child(button(
                "forward",
                "→",
                fwd,
                Box::new(crate::actions::Forward),
            ))
            .child(button(
                "reload",
                if loading { "✕" } else { "↻" },
                page.is_some(),
                Box::new(crate::actions::Reload),
            ))
    }

    /// Arc's URL pill: shows the host; click edits the full URL in the command bar.
    fn render_address_pill(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let url = self
            .active_tab()
            .and_then(|id| self.session.tab(id))
            .map(|t| t.url.clone())
            .unwrap_or_default();
        let host = url
            .split("://")
            .nth(1)
            .unwrap_or(&url)
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default();
        let host = host.strip_prefix("www.").unwrap_or(host).to_string();
        div()
            .id("address-pill")
            .h(px(32.))
            .px_3()
            .flex()
            .items_center()
            .rounded_lg()
            .bg(theme.chrome_hover)
            .text_sm()
            .text_color(if host.is_empty() {
                theme.text_dim
            } else {
                theme.text
            })
            .hover(|s| s.bg(theme.border))
            .on_click(cx.listener(|this, _, window, cx| this.open_bar(Mode::Address, window, cx)))
            .child(div().truncate().child(if host.is_empty() {
                "Search or enter address".into()
            } else {
                host
            }))
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
        let hover = theme.chrome_hover;
        div()
            .id(match zone {
                Zone::Pinned => "pinned",
                Zone::Today => "today",
            })
            .flex()
            .flex_col()
            .gap_px()
            // An empty zone still needs somewhere to drop.
            .min_h(px(12.))
            .rounded_md()
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
        let accent = Hsla::from(theme.accent);
        let drag = DraggedTab {
            id,
            title: title.clone(),
            theme: *theme,
        };

        div()
            .id(("tab", id.0))
            .group(group.clone())
            .flex()
            .items_center()
            .gap_2()
            .h(px(32.))
            .px_2()
            .rounded_md()
            .text_sm()
            .when(active == Some(id), |d| {
                d.bg(theme.chrome_active).shadow_sm()
            })
            .hover(|s| s.bg(theme.chrome_hover))
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
                    .rounded_sm()
                    .text_color(theme.text_dim)
                    .opacity(0.)
                    .group_hover(group, |s| s.opacity(1.))
                    .hover(|s| s.bg(theme.border))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_tab_id(id, window, cx);
                    }))
                    .child("×"),
            )
    }

    fn render_space_strip(
        &self,
        active: SpaceId,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let hover = theme.chrome_hover;
        let spaces = self.session.spaces().iter().map(|space| {
            let id = space.id;
            div()
                .id(("space", id.0))
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .when(id == active, |d| d.bg(theme.chrome_active).shadow_sm())
                .when(id != active, |d| d.opacity(0.6))
                .hover(|s| s.bg(hover).opacity(1.))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.activate_space(id, window, cx)),
                )
                .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(hover).opacity(1.))
                .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                    this.session.move_tab(d.id, id, Zone::Today, 0);
                    this.sync(window, cx);
                }))
                .child(SharedString::from(space.icon.clone()))
        });
        div()
            .flex()
            .items_center()
            .justify_center()
            .gap_1()
            .children(spaces)
            .child(
                div()
                    .id("add-space")
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .text_color(theme.text_dim)
                    .hover(|s| s.bg(hover))
                    .on_click(cx.listener(|this, _, window, cx| this.new_space(window, cx)))
                    .child("+"),
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
    let text_dim = theme.text_dim;
    let fallback = move || {
        div()
            .size(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .text_xs()
            .text_color(text_dim)
            .child(letter.clone())
            .into_any_element()
    };
    let icon = match url {
        // ponytail: favicons load via gpui's HTTP client (none set = letter fallback), not the
        // Space's CEF context; fine for public icons, wrong for icons behind a login.
        Some(url) => img(url)
            .size(px(16.))
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
