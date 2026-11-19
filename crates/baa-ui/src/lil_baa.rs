//! lil-baa (Arc's Little Arc, pen.dev "Arc — Little Arc"): a link from another
//! app opens in a small window instead of the browser. "Open in <Space>" moves
//! the live page into the browser as a tab.

use crate::theme::r;
use baa_model::{ProfileId, SpaceId};
use baa_web::{PageEvent, PageHandle, WebHost};
use gpui::{
    anchored, deferred, div, point, prelude::*, px, size, App, Bounds, Context, Entity,
    Div, FocusHandle, Hsla, KeyDownEvent, SharedString, Stateful, Subscription, TitlebarOptions, WeakEntity, Window,
    WindowBackgroundAppearance, WindowBounds, WindowOptions,
};

use crate::{
    actions::OpenInBrowser,
    browser::{cut_left, gradient_fill, host_of, mid_stop, shadow, Browser, PAGE_RADIUS},
    icons::icon,
    input,
    prompts::{ask_layer, button_look, page_asks, Asks, Prompter},
    theme::{surface, Theme},
};

pub fn window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(720.), px(612.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("lil-baa".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(14.), px(16.))),
        }),
        window_background: WindowBackgroundAppearance::Transparent,
        // The smallest that still fits a prompt's sheet over the page.
        window_min_size: Some(size(px(400.), px(320.))),
        ..Default::default()
    }
}

pub struct LilBaa<H: WebHost> {
    browser: WeakEntity<Browser<H>>,
    /// Taken when the page moves into the browser.
    pub(crate) page: Option<Entity<H::Page>>,
    _sub: Subscription,
    /// The page's storage; opening it in a Space with another one reloads it there.
    profile: ProfileId,
    /// Where "Open in" sends it.
    space: SpaceId,
    url: String,
    /// The Spaces as of opening (name with icon, gradient). lil-baa never reads
    /// the browser while drawing: it first draws inside the browser's update.
    spaces: Vec<SpaceChoice>,
    choosing_space: bool,
    focus: FocusHandle,
    pub(crate) asks: Asks,
    pub(crate) ask_focus: FocusHandle,
}

#[derive(Clone)]
pub struct SpaceChoice {
    pub id: SpaceId,
    pub label: String,
    pub name: String,
    /// In baa's light look and its dark one.
    pub gradient: [crate::theme::Stops; 2],
}

impl<H: WebHost> LilBaa<H> {
    pub fn new(
        browser: WeakEntity<Browser<H>>,
        page: Entity<H::Page>,
        (space, profile): (SpaceId, ProfileId),
        url: String,
        spaces: Vec<SpaceChoice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let sub = cx.subscribe_in(&page, window, |this, _, event, window, cx| {
            this.on_page_event(event, window, cx)
        });
        window.focus(&page.read(cx).focus_handle(cx), cx);
        // The browser's own observer has taken the new look by the time this draws.
        // ponytail: a change of the setting shows here at the next draw, like ASCII.
        cx.observe_window_appearance(window, |_, _, cx| cx.notify()).detach();
        Self {
            browser,
            page: Some(page),
            _sub: sub,
            profile,
            space,
            url,
            spaces,
            choosing_space: false,
            focus: cx.focus_handle(),
            asks: Asks::default(),
            ask_focus: cx.focus_handle(),
        }
    }

    fn on_page_event(&mut self, event: &PageEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            PageEvent::UrlChanged(url) => self.url = url.clone(),
            PageEvent::TitleChanged(title) => window.set_window_title(title),
            // A tab in the browser, adopting the one Chrome opened for the link.
            PageEvent::OpenRequested { url, .. } => {
                let Some(browser) = self.browser.upgrade() else { return };
                let (space, main) = (self.space, browser.read(cx).window_handle());
                main.update(cx, |_, main_window, cx| {
                    browser.update(cx, |b, cx| b.open_supplied_in(space, url, main_window, cx))
                })
                .ok();
            }
            PageEvent::Closed => window.remove_window(),
            PageEvent::Dialog(_) | PageEvent::PermissionRequested { .. } | PageEvent::AskClosed(_) => {
                if self.asks.on_event(event, cx.background_executor().now()) {
                    self.focus_page(window, cx);
                }
            }
            PageEvent::Focused if self.asks.dialog.is_some() => self.focus_page(window, cx),
            _ => {}
        }
        cx.notify();
    }

    /// The page, unless a dialog is up: it keeps the keyboard.
    fn focus_page(&self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.page {
            Some(page) if self.asks.dialog.is_none() => window.focus(&page.read(cx).focus_handle(cx), cx),
            _ => window.focus(&self.ask_focus, cx),
        }
    }

    pub(crate) fn open_in_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(browser), Some(page)) = (self.browser.upgrade(), self.page.take()) else {
            return;
        };
        let (space, url) = (self.space, self.url.clone());
        let same_profile = browser
            .read(cx)
            .session()
            .space(space)
            .is_some_and(|s| s.profile == self.profile);
        let main = browser.read(cx).window_handle();
        let asks = std::mem::take(&mut self.asks);
        let moved = main.update(cx, |_, main_window, cx| {
            browser.update(cx, |browser, cx| {
                if same_profile {
                    browser.adopt_page(page, space, &url, main_window, cx);
                    if let Some(tab) = browser.active_tab() {
                        browser.asks.insert(tab, asks);
                    }
                    browser.focus_page(main_window, cx);
                } else {
                    // Another Space's storage: the page starts over there.
                    browser.open_url_in(space, &url, main_window, cx);
                }
            });
            main_window.activate_window();
        });
        if moved.is_ok() {
            window.remove_window();
        }
    }

    fn render_space_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.choosing_space {
            return None;
        }
        let rows = self.spaces.iter().map(|choice| {
            let (id, label) = (choice.id, choice.label.clone());
            div()
                .id(("lil-space", id.0))
                .h(px(30.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(r(7.))
                .hover(|s| s.bg(theme.accent).text_color(gpui::white()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.space = id;
                    this.choosing_space = false;
                    cx.notify();
                }))
                .child(SharedString::from(label))
        });
        Some(deferred(
            anchored().anchor(gpui::Anchor::TopRight).child(
                div()
                    .id("lil-space-menu")
                    .occlude()
                    .mt(px(36.))
                    .min_w(px(180.))
                    .p(px(5.))
                    .rounded(r(10.))
                    .bg(surface::bg())
                    .border_1()
                    .border_color(surface::border())
                    .shadow(vec![shadow(gpui::rgba(0x18122C29).into(), 8., 24.)])
                    .text_size(px(13.))
                    .text_color(surface::ink())
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.choosing_space = false;
                        cx.notify();
                    }))
                    .children(rows),
            ),
        ))
    }
}

impl<H: WebHost> Render for LilBaa<H> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let choice = self.spaces.iter().find(|s| s.id == self.space);
        let name = choice.map_or("baa".into(), |s| s.name.clone());
        let (top, mid, bottom) = choice.map_or((gpui::rgb(0xDCCFFB), None, gpui::rgb(0xA592EC)), |s| s.gradient[crate::theme::dark() as usize]);
        let theme = Theme { bg_mid: mid, ..Theme::from_gradient(top, bottom) };
        let content = self
            .page
            .clone()
            .map(|page| page.update(cx, |p, cx| p.render(window, cx)));
        let host = SharedString::from(host_of(&self.url));
        let menu = self.render_space_menu(&theme, cx);
        let asks = page_asks(self, &self.asks, &self.url, &theme, &self.ask_focus, 1., None, cx);

        let root = div()
            .key_context("LilBaa")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &OpenInBrowser, window, cx| this.open_in_browser(window, cx)))
            .size_full()
            .flex()
            .flex_col()
            .px(px(8.))
            .pb(px(8.))
            .bg(gradient_fill(&theme))
            .relative()
            .children(mid_stop(&theme, 0.))
            .text_color(theme.ink)
            .text_size(px(13.))
            .when(crate::theme::ascii(), |d| d.font_family(crate::theme::MONO).text_size(px(12.5)))
            .child(
                div()
                    .id("lil-toolbar")
                    .h(px(46.))
                    .pl(px(6.))
                    .pr(px(2.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .on_mouse_down(gpui::MouseButton::Left, |_, window, _| window.start_window_move())
                    // Traffic lights.
                    .child(div().w(px(52.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(30.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .rounded(r(9.))
                            .bg(theme.glass)
                            .child(icon("globe", 14., theme.ink))
                            .child(
                                cut_left(div())
                                    .flex_1()
                                    .min_w_0()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child(host),
                            )
                            .child(icon("link", 14., theme.ink_muted))
                            .child(icon("sliders-horizontal", 14., theme.ink_muted)),
                    )
                    .child(
                        div()
                            .h(px(30.))
                            .pl(px(10.))
                            .pr(px(9.))
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .rounded(r(9.))
                            .bg(theme.glass_strong)
                            .shadow(vec![shadow(Hsla { a: 0.14, ..theme.ink }, 1., 3.)])
                            .text_size(px(12.5))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(
                                div()
                                    .id("lil-open")
                                    .flex()
                                    .items_center()
                                    .gap(px(7.))
                                    .on_click(cx.listener(|this, _, window, cx| this.open_in_browser(window, cx)))
                                    .child(icon("house", 13., theme.ink))
                                    .child(SharedString::from(format!("Open in {name}"))),
                            )
                            .child(div().w(px(1.)).h(px(14.)).bg(theme.divider))
                            .child(
                                div()
                                    .id("lil-choose-space")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.choosing_space = !this.choosing_space;
                                        cx.notify();
                                    }))
                                    .child(icon("chevron-down", 13., theme.ink_muted)),
                            )
                            .children(menu),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .rounded(r(PAGE_RADIUS))
                    .overflow_hidden()
                    .bg(gpui::white())
                    .shadow(vec![shadow(Hsla { a: 0.12, ..theme.ink }, 1., 4.)])
                    .relative()
                    .children(content)
                    .when(!asks.is_empty(), |d| d.child(ask_layer().children(asks))),
            );
        let has_field = self.asks.dialog.as_mut().and_then(|d| d.field()).is_some();
        input::wire(root, &self.ask_focus, has_field, window, cx, |l| l.asks.dialog.as_mut()?.field(), Self::key)
    }
}

impl<H: WebHost> Prompter for LilBaa<H> {
    fn answer(&mut self, ok: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ask) = self.asks.take_dialog(ok, cx.background_executor().now()) else { return };
        if let Some(page) = &self.page {
            page.read(cx).answer_dialog(ask.dialog.id, ask.answer(ok));
        }
        self.focus_page(window, cx);
        cx.notify();
    }

    fn allow(&mut self, allow: bool, cx: &mut Context<Self>) {
        let Some(id) = self.asks.take_permission(allow, cx.background_executor().now()) else { return };
        if let Some(page) = &self.page {
            page.read(cx).answer_permission(id, allow);
        }
        cx.notify();
    }

    fn toggle_check(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.asks.dialog.as_mut() {
            d.checked = !d.checked;
            cx.notify();
        }
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ask) = self.asks.dialog.as_mut() else { return };
        let (key, answer) = ask.on_key(event, cx);
        if let Some(ok) = answer {
            self.answer(ok, window, cx);
        }
        key.stop_unless_typed(cx);
        cx.notify();
    }

    fn button(&self, key: (&'static str, u64), label: SharedString, primary: bool, theme: &Theme, _: &mut Context<Self>) -> Stateful<Div> {
        button_look(key, label, primary, theme, 0., 0.)
    }
}

impl<H: WebHost> gpui::Focusable for LilBaa<H> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
