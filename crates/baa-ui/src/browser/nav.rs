//! Back and forward: the long-press history menu and the two-finger swipe.

use super::*;

/// Back/forward held down: the history in that direction.
pub(crate) struct HistoryMenu {
    pub at: Point<Pixels>,
    /// (offset from the current entry, title).
    pub entries: Vec<(i32, String)>,
}

/// Back/forward pressed this long shows history instead.
const HOLD: std::time::Duration = std::time::Duration::from_millis(400);

impl<H: WebHost> Browser<H> {
    /// Back (`back`) or forward pressed: a short press navigates on release, a
    /// long one shows that direction's history.
    pub(crate) fn nav_down(&mut self, back: bool, at: Point<Pixels>, cx: &mut Context<Self>) {
        let pressed = Instant::now();
        self.nav_press = Some((back, pressed));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOLD).await;
            this.update(cx, |this, cx| {
                if this.nav_press.is_some_and(|(_, t)| t == pressed) {
                    this.nav_press = None;
                    this.show_history(back, at, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn nav_up(&mut self, cx: &mut Context<Self>) {
        if let Some((back, _)) = self.nav_press.take() {
            self.with_page(cx, |p| if back { p.go_back() } else { p.go_forward() });
        }
    }

    pub(crate) fn show_history(&mut self, back: bool, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(page) = self.active_page() else { return };
        let (entries, current) = page.read(cx).history();
        let label = |(title, url): &(String, String)| {
            if title.is_empty() { url.clone() } else { title.clone() }
        };
        let entries: Vec<(i32, String)> = if back {
            entries[..current.min(entries.len())]
                .iter()
                .enumerate()
                .rev()
                .map(|(i, e)| (i as i32 - current as i32, label(e)))
                .collect()
        } else {
            entries
                .iter()
                .enumerate()
                .skip(current + 1)
                .map(|(i, e)| (i as i32 - current as i32, label(e)))
                .collect()
        };
        if !entries.is_empty() {
            self.history_menu = Some(HistoryMenu { at, entries });
            cx.notify();
        }
    }

    /// Two-finger horizontal swipe across the chrome switches Spaces; the list
    /// follows the fingers and a quarter width commits.
    pub(crate) fn on_chrome_scroll(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let gpui::ScrollDelta::Pixels(delta) = event.delta else {
            return;
        };
        match event.touch_phase {
            gpui::TouchPhase::Started => self.swipe = Some(0.0),
            gpui::TouchPhase::Moved => {
                let Some(swipe) = &mut self.swipe else { return };
                let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
                if dx.abs() <= dy.abs() {
                    return;
                }
                *swipe = (*swipe + dx / SIDEBAR_WIDTH).clamp(-1.0, 1.0);
                self.slide.snap(*swipe);
                cx.notify();
            }
            gpui::TouchPhase::Ended | gpui::TouchPhase::Cancelled => {
                let Some(swipe) = self.swipe.take() else { return };
                let first = self.session.spaces().first().map(|s| s.id) == self.session.active_space();
                if self.library.is_some() {
                    // Fingers left: back to the first Space, which lies to the right.
                    if swipe < -0.25 {
                        self.close_library(window, cx);
                    }
                    self.slide.set_target(0.0);
                } else if swipe > 0.25 && first {
                    // Past the first Space lies the Library (Arc).
                    self.slide.set_target(0.0);
                    self.open_library(crate::library::Section::Downloads, cx);
                } else if swipe.abs() > 0.25 {
                    // Fingers left: the next Space comes in from the right.
                    self.step_space(if swipe < 0.0 { 1 } else { -1 }, window, cx);
                } else {
                    self.slide.set_target(0.0);
                    cx.notify();
                }
            }
        }
    }

    pub(super) fn render_history_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let menu = self.history_menu.as_ref()?;
        let accent = self.theme().accent;
        let rows: Vec<_> = menu.entries.iter().enumerate().map(|(i, (offset, title))| {
            let offset = *offset;
            let key = ("menu", i as u64);
            let h = self.motion.get(key, Feel::Hover);
            let row = div()
                .id(("history", offset as i64 as u64))
                .h(px(30.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(r(7.))
                .max_w(px(360.))
                .bg(fade(accent, h))
                .text_color(mix(surface::ink().into(), gpui::white(), h))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.with_page(cx, |p| p.go_to_offset(offset));
                    this.close_overlays(window, cx);
                }))
                .child(div().truncate().child(SharedString::from(title.clone())));
            feel(row, key, cx)
        }).collect();
        Some(deferred(div().absolute().inset_0().occlude().child(
            anchored().position(menu.at).snap_to_window().child(drop_in(
                div()
                    .id("history-menu")
                    .occlude()
                    .min_w(px(220.))
                    .p(px(5.))
                    .rounded(r(10.))
                    .bg(surface::bg())
                    .border_1()
                    .border_color(surface::border())
                    .shadow(vec![shadow(gpui::rgba(0x18122C29).into(), 8., 24.)])
                    .text_size(px(13.))
                    .text_color(surface::ink())
                    .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_overlays(window, cx)))
                    .children(rows),
                "history-menu",
            )),
        )))
    }
}
