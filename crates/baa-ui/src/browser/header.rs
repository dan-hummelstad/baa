//! The window-wide row with the traffic lights, navigation, URL and extensions, and
//! the same controls at the top of a left-hand sidebar.

use super::*;

impl<H: WebHost> Browser<H> {
    /// The window's top row (Arc's top-bar layout): traffic lights, sidebar
    /// toggle and navigation at the left, the short URL centred on the window,
    /// extensions at the right. Empty space drags the window.
    pub(super) fn render_header(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("header").debug_selector(|| "header".into())
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
                    .child(self.render_window_controls(theme, false, cx)),
            )
            .child(self.render_address(theme, false, cx))
            .child(self.render_header_tools(theme, cx).flex_1().justify_end())
    }

    /// Arc's sidebar layout (sidebar on the left): the sidebar runs the window's full
    /// height and starts with the traffic lights' row (toggle, navigation), then the URL
    /// field with the header's tools. `card`: in the revealed card, whose top is `INSET`
    /// down; the row is pulled up so it stays level with the traffic lights.
    pub(super) fn render_sidebar_top(&self, theme: &Theme, card: bool, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("sidebar-top")
            .flex_none()
            .w(px(self.sidebar_width()))
            .px(px(8.))
            .flex()
            .flex_col()
            .on_scroll_wheel(cx.listener(|this, e, window, cx| this.on_chrome_scroll(e, window, cx)))
            .child(
                div()
                    .id("toolbar").debug_selector(|| "toolbar".into())
                    .h(px(HEADER_H))
                    .when(card, |d| d.mt(px(-INSET - 1.)))
                    .pl(px(8.))
                    .flex()
                    .items_center()
                    .on_mouse_down(gpui::MouseButton::Left, crate::sidebar::drag_or_zoom)
                    .child(self.render_window_controls(theme, true, cx)),
            )
            // ponytail: many pinned extensions squeeze the field; an overflow menu if that bites.
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .mb(px(4.))
                    .child(self.render_address(theme, true, cx))
                    .child(self.render_header_tools(theme, cx).flex_none()),
            )
    }

    /// Downloads, pinned extensions, split and the extensions menu.
    fn render_header_tools(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let unpinned = &self.settings.unpinned_extensions;
        let pinned = self
            .extensions
            .iter()
            .enumerate()
            .filter(|(_, ext)| !unpinned.contains(&ext.id) && self.extension_enabled(&ext.id));
        let extensions: Vec<_> = pinned.map(|(i, ext)| {
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
                .rounded(r(6.))
                .bg(fade(theme.hover, h))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| this.open_extension(&ext, window, cx)))
                .child(match icon_path {
                    // A component extension (uBlock) names an icon that is only in the resource pak.
                    Some(path) if path.exists() => gpui::img(path).size(px(size)).into_any_element(),
                    _ => icon("sliders-horizontal", size, theme.ink_muted).into_any_element(),
                });
            feel(el, key, cx)
        }).collect();
        div()
            .flex()
            .items_center()
            .gap(px(4.))
            .children(self.render_download_button(theme, cx))
            .children(extensions)
            .child(self.render_split_button(theme, cx))
            .child(self.render_extensions_button(theme, cx))
    }

    /// The short URL (click to edit): centred in the header, or the sidebar's field.
    fn render_address(&self, theme: &Theme, field: bool, cx: &mut Context<Self>) -> AnyElement {
        let url = self
            .active_tab()
            .and_then(|id| self.session.tab(id))
            .map(|t| t.url.clone())
            .unwrap_or_default();
        let host = host_of(&url);
        let loading = self.active_page().is_some_and(|p| p.read(cx).is_loading());
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
                .rounded(r(4.))
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
            .child(cut_left(div().min_w_0()).child(label.clone()))
            .children(self.render_site_button(theme, ink, cx))
            .with_animation(
                ElementId::Name(label),
                Animation::new(std::time::Duration::from_millis(220)).with_easing(gpui::ease_out_quint()),
                |d, t| d.opacity(t).ml(px(4. * (1. - t))),
            );
        // The env the page is on, as Settings → Developer names it; production in red.
        let env = self.settings.env_of(&url).map(|(_, e)| {
            let prod = ["prod", "production"].iter().any(|p| e.label.trim().eq_ignore_ascii_case(p));
            let tint: Hsla = if prod { gpui::rgb(0xDC2626).into() } else { theme.accent };
            div()
                .debug_selector(|| "env-badge".into())
                .flex_none()
                .mr(px(6.))
                .px(px(5.))
                .rounded(r(4.))
                .bg(fade(tint, 0.16))
                .text_color(tint)
                .text_size(px(10.))
                .font_weight(gpui::FontWeight::BOLD)
                .child(SharedString::from(e.label.trim().to_uppercase()))
        });
        let accent = theme.accent;
        let progress = loading.then(|| {
            div()
                .absolute()
                .bottom_0()
                .h(px(2.))
                .w(relative(0.3))
                .rounded(r(1.))
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
            .rounded(r(7.))
            .text_size(px(13.))
            .text_color(mix(theme.ink_muted, theme.ink, h))
            .bg(fade(theme.hover, h))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.open_bar(Mode::Address, window, cx)))
            .when(field, |d| {
                d.flex_1().min_w_0().h(px(32.)).px(px(12.)).rounded(r(10.)).bg(mix(theme.glass, theme.glass_strong, h))
            })
            // ASCII Chrome: "│ ▸ host  ⌘L │".
            .when(crate::theme::ascii(), |d| {
                d.gap(px(8.))
                    .border_1()
                    .border_color(crate::theme::ascii::frame())
                    .child(div().text_color(theme.accent).child("▸"))
            })
            .children(env)
            .child(label)
            .when(crate::theme::ascii(), |d| d.child(div().text_color(theme.ink_faint).child("⌘L")))
            .children(progress);
        feel(address, key, cx).into_any_element()
    }

    /// Same as ⌃⇧=: splits with the Space's most recent other tab. Lit while a split is shown.
    fn render_split_button(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let key = ("split", 0);
        let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
        let on = self.session.active_space().is_some_and(|s| self.session.shown_split(s).is_some());
        let el = div()
            .id("split-button")
            .size(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(r(6.))
            .bg(fade(theme.hover, h.max(on as u8 as f32)))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(|_, window, cx| window.dispatch_action(Box::new(SplitView), cx))
            .child(icon("columns-2", 15. * (1. - 0.15 * p), mix(theme.ink_muted, theme.ink, h)));
        feel(el, key, cx)
    }
}
