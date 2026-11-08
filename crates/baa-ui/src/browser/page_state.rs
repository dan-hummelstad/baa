//! Page fullscreen, the find bar's match count, and the crashed-page overlay (PLAN-m3 §2.3).

use super::*;
use baa_web::ExclusiveKind;

/// How long Chrome's fullscreen / pointer lock notice stays up.
pub(crate) const NOTICE_FOR: std::time::Duration = std::time::Duration::from_secs(3);

/// What the notice says: who took the screen, keyboard or cursor, and how to get
/// it back, so a page cannot pose as the browser or the desktop.
pub(crate) fn notice_text(origin: &str, kind: ExclusiveKind) -> String {
    let site = Some(host_of(origin)).filter(|h| !h.is_empty()).unwrap_or_else(|| origin.to_string());
    match kind {
        ExclusiveKind::Fullscreen => format!("{site} is full screen. Press Esc to exit"),
        ExclusiveKind::KeyboardLock => format!("{site} is full screen. Press and hold Esc to exit"),
        ExclusiveKind::PointerLock => format!("{site} hid your cursor. Press Esc to show your cursor"),
    }
}

#[derive(Default)]
pub(crate) struct PageState {
    /// baa put the window into macOS fullscreen for this page, so it leaves again.
    entered: bool,
    /// The window was seen fullscreen; leaving it from then on ends page fullscreen.
    seen: bool,
    /// Find bar's `(active, total)` for the active tab.
    pub(crate) find_result: Option<(u32, u32)>,
    pub(crate) crashed: HashSet<TabId>,
    pub(crate) groups: super::groups::Groups,
    /// The fullscreen / pointer lock notice while it shows, and a count so only
    /// the newest one's timer hides it.
    pub(crate) notice: Option<String>,
    notice_count: u64,
}

impl<H: WebHost> Browser<H> {
    pub(super) fn on_page_state_event(&mut self, id: TabId, event: &PageEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            PageEvent::Fullscreen(true) => {
                if self.active_tab() != Some(id) {
                    if let Some((page, _)) = self.pages.get(&id) {
                        page.read(cx).exit_fullscreen();
                    }
                } else if self.fullscreen != Some(id) {
                    // A repeat (another element going fullscreen) keeps `entered`.
                    self.fullscreen = Some(id);
                    self.page_state.seen = false;
                    self.page_state.entered = !window.is_fullscreen();
                    if self.page_state.entered {
                        window.toggle_fullscreen();
                    }
                }
            }
            PageEvent::Fullscreen(false) if self.fullscreen == Some(id) => self.end_fullscreen(false, window, cx),
            PageEvent::FindResult { active, total } if self.active_tab() == Some(id) => {
                self.page_state.find_result = Some((*active, *total));
            }
            PageEvent::ExclusiveAccess { origin, kind } if self.active_tab() == Some(id) => {
                self.page_state.notice = kind.map(|k| notice_text(origin, k));
                self.page_state.notice_count += 1;
                let count = self.page_state.notice_count;
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(NOTICE_FOR).await;
                    this.update(cx, |this, cx| {
                        if this.page_state.notice_count == count {
                            this.page_state.notice = None;
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
            PageEvent::Crashed(on) => {
                if *on {
                    self.page_state.crashed.insert(id);
                } else {
                    self.page_state.crashed.remove(&id);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Back to the card layout; `tell_page` when the page did not leave by itself.
    pub(super) fn end_fullscreen(&mut self, tell_page: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.fullscreen.take() else { return };
        if tell_page {
            if let Some((page, _)) = self.pages.get(&tab) {
                page.read(cx).exit_fullscreen();
            }
        }
        if std::mem::take(&mut self.page_state.entered) && window.is_fullscreen() {
            window.toggle_fullscreen();
        }
    }

    /// The page alone, filling the window; `None` outside page fullscreen.
    pub(super) fn fullscreen_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<gpui::Div> {
        self.fullscreen?;
        // The window leaving macOS fullscreen (⌃⌘F, a Space switch) ends it, whoever entered it;
        // so does anything baa draws over the page (bar, find, Peek).
        if window.is_fullscreen() {
            self.page_state.seen = true;
        }
        let left = self.page_state.seen && !window.is_fullscreen();
        if left || self.bar.is_some() || self.find.is_some() || self.peek.is_some() {
            self.end_fullscreen(true, window, cx);
            return None;
        }
        let page = self.active_page().filter(|_| self.fullscreen == self.active_tab())?.clone();
        page.read(cx).set_corner_radius(0.);
        page.read(cx).set_square_corners(0);
        let content = page.update(cx, |p, cx| p.render(window, cx));
        let theme = self.theme();
        Some(
            div()
                .key_context("Browser")
                .track_focus(&self.focus)
                .relative()
                .size_full()
                .bg(gpui::black())
                .child(content)
                .children(self.render_asks(&theme, cx))
                .children(self.exclusive_notice()),
        )
    }

    pub(super) fn exclusive_notice(&self) -> Option<impl IntoElement> {
        let text = self.page_state.notice.clone()?;
        Some(
            div().absolute().top(px(24.)).left_0().right_0().flex().justify_center().child(
                cut_left(div())
                    .id("exclusive-notice")
                    .debug_selector(|| "exclusive-notice".into())
                    .max_w(relative(0.9))
                    .px(px(16.))
                    .py(px(10.))
                    .rounded(r(10.))
                    .bg(gpui::rgba(0x141018E6))
                    .text_color(gpui::white())
                    .text_size(px(14.))
                    .child(SharedString::from(text)),
            ),
        )
    }

    pub(super) fn crash_overlay(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        self.active_tab().filter(|id| self.page_state.crashed.contains(id))?;
        Some(
            div()
                .id("crashed")
                .occlude()
                .absolute()
                .inset_0()
                .rounded(r(PAGE_RADIUS))
                .bg(surface::bg())
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(14.))
                .child(div().text_size(px(16.)).text_color(surface::ink()).child("This page crashed"))
                .child(
                    div()
                        .id("crashed-reload")
                        .px(px(16.))
                        .py(px(7.))
                        .rounded(r(8.))
                        .bg(self.theme().accent)
                        .text_color(gpui::white())
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.reload_crashed(cx)))
                        .child("Reload"),
                ),
        )
    }

    pub(crate) fn reload_crashed(&mut self, cx: &mut Context<Self>) {
        self.with_page(cx, |p| p.reload(false));
    }
}
