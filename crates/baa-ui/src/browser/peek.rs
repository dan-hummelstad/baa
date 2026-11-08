//! A link previewed over the page (Peek), and an extension's toolbar popup.

use super::*;

/// A link previewed over the page.
pub(crate) struct PeekPage<P> {
    pub(crate) page: Entity<P>,
    pub(super) _sub: Subscription,
    pub(crate) space: SpaceId,
    pub(crate) url: String,
    pub(super) open: Spring,
    /// An extension popup rather than a link preview.
    pub(crate) popup: bool,
    /// A popup's card size, following its content; `None` until it reports one.
    pub(crate) size: Option<(f32, f32)>,
    /// Whether a popup is on screen yet: it waits, laid out at its size but
    /// clipped away, until its size stops changing, so it never shows resizing.
    pub(crate) shown: bool,
    /// Bumped by each size a waiting popup reports; only the last one reveals it.
    pub(super) settle: u64,
}

/// How long a popup's size has to hold before it shows. Popups report a few
/// sizes in quick succession as their content loads.
const POPUP_SETTLE: std::time::Duration = std::time::Duration::from_millis(60);

impl<H: WebHost> Browser<H> {
    /// A cmd- or middle-clicked link: a tab for later, the current one stays.
    pub(super) fn open_background_tab(&mut self, space: SpaceId, url: &str) {
        let current = self.session.space(space).and_then(|s| s.active_tab);
        self.session.open_tab(space, url, now());
        if let Some(current) = current {
            self.session.activate_tab(current, now());
        }
    }

    pub(crate) fn open_peek(
        &mut self,
        space: SpaceId,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_preview(space, url, false, window, cx)
    }

    /// Peek, or with `popup` an extension's toolbar popup under the address bar.
    pub(super) fn open_preview(
        &mut self,
        space: SpaceId,
        url: &str,
        popup: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = self.session.space(space).map(|s| s.profile) else {
            return;
        };
        self.close_peek(window, cx);
        let page = if popup {
            self.host.create_popup(profile, url, window, cx)
        } else {
            self.host.create_page(profile, url, window, cx)
        };
        page.read(cx).set_corner_radius(f32::from(r(PEEK_RADIUS)));
        page.read(cx).set_floating(true);
        let sub = cx.subscribe_in(&page, window, |this, _, event, window, cx| {
            this.on_peek_event(event, window, cx)
        });
        let mut open = Spring::new(0.0);
        open.set_target(1.0);
        window.focus(&page.read(cx).focus_handle(cx), cx);
        self.peek = Some(PeekPage {
            page,
            _sub: sub,
            space,
            url: url.into(),
            open,
            popup,
            size: None,
            shown: !popup,
            settle: 0,
        });
        // ponytail: a popup that never reports a size still shows, at Chrome's default-ish one.
        if popup {
            let page = self.peek.as_ref().map(|p| p.page.entity_id());
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(400)).await;
                this.update(cx, |this, cx| {
                    let Some(peek) = this.peek.as_mut().filter(|p| Some(p.page.entity_id()) == page) else { return };
                    peek.size.get_or_insert((380., 520.));
                    this.reveal_popup(cx);
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    pub(super) fn on_peek_event(&mut self, event: &PageEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(peek) = &mut self.peek else { return };
        match event {
            PageEvent::UrlChanged(url) => peek.url = url.clone(),
            PageEvent::OpenRequested {
                url, background, ..
            } => {
                if *background {
                    let space = peek.space;
                    self.open_background_tab(space, url);
                    return self.sync(window, cx);
                }
                // A popup's links and its options page are tabs, as in Chrome.
                if peek.popup {
                    let (space, url) = (peek.space, url.clone());
                    self.close_peek(window, cx);
                    return self.open_url_in(space, &url, window, cx);
                }
                peek.page.read(cx).load_url(url);
            }
            PageEvent::Closed => return self.close_peek(window, cx),
            PageEvent::PreferredSize(size) => return self.fit_popup(*size, cx),
            // ponytail: Peek has no room for prompts and an unanswered one
            // freezes the page; opening it as a tab gets them.
            PageEvent::Dialog(d) => peek.page.read(cx).answer_dialog(d.id, Default::default()),
            PageEvent::PermissionRequested { id, .. } => peek.page.read(cx).answer_permission(*id, false),
            _ => {}
        }
        cx.notify();
    }

    /// Sizes the popup card to its content: the first size shows the card
    /// (it drops in at that size), later ones animate to it.
    pub(crate) fn fit_popup(&mut self, size: gpui::Size<Pixels>, cx: &mut Context<Self>) {
        let Some(peek) = self.peek.as_mut().filter(|p| p.popup) else { return };
        peek.size = Some((f32::from(size.width), f32::from(size.height)));
        cx.notify();
        if peek.shown {
            return;
        }
        peek.settle += 1;
        let settle = peek.settle;
        let page = peek.page.entity_id();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(POPUP_SETTLE).await;
            this.update(cx, |this, cx| {
                if this.peek.as_ref().is_some_and(|p| p.page.entity_id() == page && p.settle == settle) {
                    this.reveal_popup(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn reveal_popup(&mut self, cx: &mut Context<Self>) {
        let Some(peek) = self.peek.as_mut().filter(|p| !p.shown) else { return };
        peek.shown = true;
        peek.open = Spring::new(0.0);
        peek.open.set_target(1.0);
        cx.notify();
    }

    pub(crate) fn close_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(peek) = self.peek.take() {
            peek.page.read(cx).set_visible(false);
            self.focus_page(window, cx);
            cx.notify();
        }
    }

    /// Peek → a real tab, keeping the live page.
    pub(crate) fn expand_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(peek) = self.peek.take() else { return };
        self.adopt_page(peek.page, peek.space, &peek.url, window, cx);
    }

    pub(crate) fn open_extension(
        &mut self,
        extension: &baa_web::Extension,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(space) = self.session.active_space() else {
            return;
        };
        match &extension.popup {
            Some(popup) => self.open_preview(space, popup, true, window, cx),
            None => {
                self.open_own_page(&format!("baa://extensions/?id={}", extension.id), window, cx);
            }
        }
    }

    /// A link from another app: a lil-baa window, which can be expanded into
    /// the browser (Arc's Little Arc).
    pub fn open_external(&mut self, url: &str, cx: &mut Context<Self>) {
        let Some(space) = self.session.active_space() else {
            return;
        };
        if self.settings.external_links == baa_model::ExternalLinks::Tab {
            let (url, window, browser) = (url.to_string(), self.window, cx.entity());
            cx.defer(move |cx| {
                window
                    .update(cx, |_, window, cx| {
                        browser.update(cx, |b, cx| b.open_url_in(space, &url, window, cx));
                        window.activate_window();
                    })
                    .ok();
                cx.activate(true);
            });
            return;
        }
        let Some(profile) = self.session.space(space).map(|s| s.profile) else {
            return;
        };
        let browser = cx.entity().downgrade();
        let host = &self.host;
        let url = url.to_string();
        let spaces: Vec<crate::lil_baa::SpaceChoice> = self
            .session
            .spaces()
            .iter()
            .filter(|s| !s.ephemeral)
            .map(|s| crate::lil_baa::SpaceChoice {
                id: s.id,
                label: format!("{} {}", s.icon, s.name),
                name: s.name.clone(),
                gradient: (s.accent, s.accent_to),
            })
            .collect();
        let options = crate::lil_baa::window_options(cx);
        let opened = cx.open_window(options, |window, cx| {
            let page = host.create_page(profile, &url, window, cx);
            cx.new(|cx| LilBaa::new(browser, page, (space, profile), url.clone(), spaces, window, cx))
        });
        if let Err(e) = opened {
            log::error!("opening lil-baa: {e}");
        }
    }

    /// Peek: a scrim over the browser and the link's page in a card, with
    /// close / open-as-tab controls above it.
    pub(super) fn render_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let peek = self.peek.as_ref()?;
        let t = peek.open.value();
        let size = peek.size;
        let host = SharedString::from(host_of(&peek.url));
        let content = peek.page.update(cx, |p, cx| p.render(window, cx));
        let control = |id: &'static str, name: &'static str| {
            div()
                .id(id)
                .size(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(r(9.))
                .bg(gpui::rgba(0xFFFFFFEB))
                .shadow(vec![shadow(gpui::rgba(0x0000002E).into(), 2., 8.)])
                .hover(|s| s.bg(gpui::white()))
                .child(icon(name, 15., rgb(0x3A3644).into()))
        };
        let controls = div()
            .absolute()
            .top(px(-40.))
            .left_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .occlude()
            .child(control("peek-close", "x").on_click(
                cx.listener(|this, _, window, cx| this.close_peek(window, cx)),
            ))
            .child(control("peek-expand", "maximize-2").on_click(
                cx.listener(|this, _, window, cx| this.expand_peek(window, cx)),
            ))
            .child(
                div()
                    .px(px(10.))
                    .py(px(5.))
                    .rounded(r(8.))
                    .bg(gpui::rgba(0x14101E66))
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(gpui::white())
                    .child(host),
            );
        if peek.popup {
            // An extension's popup: a small card under the address bar's right end.
            return Some(
                div()
                    .absolute()
                    .inset_0()
                    .child(
                        div()
                            .id("popup-backdrop")
                            .absolute()
                            .inset_0()
                            .occlude()
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, window, cx| this.close_peek(window, cx))),
                    )
                    .child(
                        div()
                            .id("popup-card")
                            // Clicks in the popup are not clicks on the backdrop.
                            .occlude()
                            .absolute()
                            .top(px(HEADER_H + 2.))
                            .right(px(INSET))
                            .mt(px((1.0 - t) * -8.0))
                            .w(px(size.map_or(380., |s| s.0)))
                            .h(px(size.map_or(520., |s| s.1)))
                            .opacity(if peek.shown { t.clamp(0.0, 1.0) } else { 0. })
                            .rounded(r(PEEK_RADIUS))
                            .overflow_hidden()
                            .bg(gpui::white())
                            .border_1()
                            .border_color(gpui::rgba(0x0000001F))
                            .shadow(vec![shadow(gpui::rgba(0x0A08143D).into(), 12., 32.)])
                            .child(if peek.shown {
                                content.into_any_element()
                            } else {
                                // The page is a native view gpui's opacity cannot hide, and a
                                // hidden one stops rendering (and reporting its size): it is laid
                                // out full size behind a one-point clip.
                                div()
                                    .size(px(1.))
                                    .overflow_hidden()
                                    .child(div().absolute().w(px(size.map_or(380., |s| s.0))).h(px(size.map_or(520., |s| s.1))).child(content))
                                    .into_any_element()
                            }),
                    )
                    .into_any_element(),
            );
        }
        // Rises a little as it fades in; gpui has no transforms, so offset and opacity carry it.
        let rise = (1.0 - t) * 24.0;
        let card = div()
            .absolute()
            .left(relative(0.139))
            .right(relative(0.139))
            .top(relative(0.093))
            .bottom(relative(0.084))
            .mt(px(rise))
            .mb(px(-rise))
            .opacity(t.clamp(0.0, 1.0))
            .child(controls)
            .child(
                div()
                    .id("peek-card")
                    .debug_selector(|| "peek-card".into())
                    // Clicks in the card are not clicks on the scrim. The page's own hitbox does not
                    // block it, and while the page is blank (loading) Chromium leaves the mouse-up to
                    // gpui, so the scrim saw a whole click and closed the Peek.
                    .occlude()
                    .size_full()
                    .rounded(r(PEEK_RADIUS))
                    .overflow_hidden()
                    .bg(rgb(0xFBFAF7))
                    .border_1()
                    .border_color(gpui::rgba(0x0000001F))
                    .shadow(vec![
                        shadow(gpui::rgba(0x0A081461).into(), 40., 100.),
                        shadow(gpui::rgba(0x0A08142E).into(), 8., 24.),
                    ])
                    .child(content),
            );
        Some(
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .id("peek-scrim")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .bg(surface::scrim())
                        .opacity(t.clamp(0.0, 1.0))
                        .on_click(cx.listener(|this, _, window, cx| this.close_peek(window, cx))),
                )
                .child(card)
                .into_any_element(),
        )
    }
}
