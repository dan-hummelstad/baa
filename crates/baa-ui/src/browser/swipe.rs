//! Safari's history swipe: two fingers over the page slide it aside to show the
//! page before it as a sheet beneath (back), or pull the next page in over it
//! from the right (forward). The engine tracks the gesture and navigates; this
//! only draws it.

use std::{sync::Arc, time::Duration};

use baa_web::{SwipePhase, SwipePicture};
use gpui::{img, AnyElement, ObjectFit, RenderImage, StyledImage};

use super::*;

/// How far the page beneath trails behind, as a share of the width (Safari's parallax).
const TRAIL: f32 = 0.3;
/// Once through, the picture stays over the page this long after the page it
/// went to commits, so its first frame is up; at most `HOLD_MAX` in all.
const HOLD: Duration = Duration::from_millis(150);
const HOLD_MAX: Duration = Duration::from_millis(1500);

pub(crate) struct HistorySwipe {
    tab: TabId,
    forward: bool,
    amount: f32,
    picture: Option<Arc<RenderImage>>,
    /// Gone through: the picture covers the page until the page it went to has drawn.
    done: bool,
    /// Which swipe a timer belongs to.
    started: Instant,
}

impl<H: WebHost> Browser<H> {
    pub(super) fn on_swipe(&mut self, tab: TabId, phase: &SwipePhase, forward: bool, amount: f32, cx: &mut Context<Self>) {
        match phase {
            SwipePhase::Began(picture) => {
                self.end_swipe(None);
                if Some(tab) == self.active_tab() {
                    let picture = picture.as_ref().and_then(render_image);
                    let started = Instant::now();
                    self.history_swipe = Some(HistorySwipe { tab, forward, amount, picture, done: false, started });
                }
            }
            SwipePhase::Changed => {
                if let Some(s) = self.history_swipe.as_mut().filter(|s| s.tab == tab && !s.done) {
                    s.amount = amount;
                }
            }
            SwipePhase::Done => {
                let Some(s) = self.history_swipe.as_mut().filter(|s| s.tab == tab) else { return };
                s.done = true;
                s.amount = 1.0;
                Self::clear_swipe_after(s.started, HOLD_MAX, cx);
            }
            SwipePhase::Cancelled => {
                if self.history_swipe.as_ref().is_some_and(|s| s.tab == tab) {
                    self.end_swipe(None);
                }
            }
        }
        cx.notify();
    }

    /// The tab's page changed: a finished swipe's picture goes once it has drawn.
    pub(super) fn swipe_landed(&mut self, tab: TabId, cx: &mut Context<Self>) {
        if let Some(s) = self.history_swipe.as_ref().filter(|s| s.tab == tab && s.done) {
            Self::clear_swipe_after(s.started, HOLD, cx);
        }
    }

    fn clear_swipe_after(started: Instant, wait: Duration, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                this.end_swipe(Some(started));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Ends the swipe (only `started`'s, if given). Its picture leaves the atlas on the next frame.
    fn end_swipe(&mut self, started: Option<Instant>) {
        if started.is_some_and(|t| self.history_swipe.as_ref().is_none_or(|s| s.started != t)) {
            return;
        }
        if let Some(picture) = self.history_swipe.take().and_then(|s| s.picture) {
            self.stale_pictures.push(picture);
        }
    }

    /// The page with the swipe around it: what it slides over or under.
    pub(super) fn render_swipe(&mut self, content: AnyElement, window: &mut Window) -> impl IntoElement {
        for picture in self.stale_pictures.drain(..) {
            window.drop_image(picture).ok();
        }
        let swipe = self.history_swipe.as_ref().filter(|s| Some(s.tab) == self.active_tab());
        let (forward, a, done) = swipe.map_or((false, 0.0, false), |s| (s.forward, s.amount, s.done));
        let sheet = || -> AnyElement {
            match swipe.and_then(|s| s.picture.clone()) {
                Some(picture) => img(picture).size_full().object_fit(ObjectFit::Fill).into_any_element(),
                None => div().size_full().bg(gpui::white()).into_any_element(),
            }
        };
        let edge = || vec![shadow(gpui::black().opacity(0.25), 0., 16.)];
        let dim = |alpha: f32| div().absolute().inset_0().bg(gpui::black().opacity(0.2 * alpha));
        // Back: the page slides right off the previous one, which trails in from the left.
        let back = swipe.is_some() && !forward && !done;
        let under = back.then(|| {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w_full()
                .left(relative(-(1.0 - a) * TRAIL))
                .child(sheet())
                .child(dim(1.0 - a))
        });
        let page = div()
            .absolute()
            .top_0()
            .bottom_0()
            .w_full()
            .left(relative(if back { a } else { 0.0 }))
            .when(back, |d| d.shadow(edge()))
            .child(content);
        // Forward: the next page comes in over this one from the right.
        let over = swipe.filter(|_| forward && !done).map(|_| {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w_full()
                .left(relative(1.0 - a))
                .shadow(edge())
                .child(sheet())
        });
        // Through: the page it went to, as it was, until that page draws itself.
        let cover = done.then(|| div().absolute().inset_0().occlude().child(sheet()));
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .children(under)
            .child(page)
            .children((swipe.is_some() && forward && !done).then(|| dim(a)))
            .children(over)
            .children(cover)
    }
}

/// The engine's BGRA rows as a gpui image (gpui wants BGRA in its "RGBA" frames).
fn render_image(p: &SwipePicture) -> Option<Arc<RenderImage>> {
    let buffer = image::RgbaImage::from_raw(p.width, p.height, p.bgra.to_vec())?;
    Some(Arc::new(RenderImage::new([image::Frame::new(buffer)])))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use baa_model::Store;
    use baa_web::{PageEvent, SwipePhase, SwipePicture};
    use gpui::{Entity, TestAppContext, VisualTestContext};

    use crate::{mock::MockHost, Browser};

    fn browser(cx: &mut TestAppContext) -> (Entity<Browser<MockHost>>, &mut VisualTestContext) {
        cx.update(crate::init);
        let (b, cx) = cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx));
        cx.simulate_keystrokes("secondary-t");
        cx.simulate_input("a.example");
        cx.simulate_keystrokes("enter");
        (b, cx)
    }

    fn emit(b: &Entity<Browser<MockHost>>, event: PageEvent, cx: &mut VisualTestContext) {
        let page = b.read_with(cx, |b, _| b.pages.get(&b.active_tab().unwrap()).unwrap().0.clone());
        page.update(cx, |_, cx| cx.emit(event));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }

    fn swipe(b: &Entity<Browser<MockHost>>, phase: SwipePhase, amount: f32, cx: &mut VisualTestContext) {
        emit(b, PageEvent::Swipe { phase, forward: false, amount }, cx);
    }

    fn state(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) -> Option<(f32, bool, bool)> {
        b.read_with(cx, |b, _| b.history_swipe.as_ref().map(|s| (s.amount, s.done, s.picture.is_some())))
    }

    #[gpui::test]
    fn the_sheet_follows_the_swipe_and_covers_the_page_until_it_has_drawn(cx: &mut TestAppContext) {
        let (b, cx) = browser(cx);
        let picture = SwipePicture { width: 2, height: 1, bgra: Arc::new(vec![255; 8]) };
        swipe(&b, SwipePhase::Began(Some(picture)), 0.0, cx);
        swipe(&b, SwipePhase::Changed, 0.4, cx);
        assert_eq!(state(&b, cx), Some((0.4, false, true)));
        swipe(&b, SwipePhase::Done, 1.0, cx);
        assert_eq!(state(&b, cx), Some((1.0, true, true)), "held over the page after the swipe");
        emit(&b, PageEvent::UrlChanged("https://before.example/".into()), cx);
        cx.executor().advance_clock(super::HOLD);
        cx.run_until_parked();
        assert_eq!(state(&b, cx), None, "gone once the page it went to drew");
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert!(b.read_with(cx, |b, _| b.stale_pictures.is_empty()), "the picture left the atlas");
    }

    #[gpui::test]
    fn a_cancelled_swipe_leaves_the_page_as_it_was(cx: &mut TestAppContext) {
        let (b, cx) = browser(cx);
        swipe(&b, SwipePhase::Began(None), 0.0, cx);
        swipe(&b, SwipePhase::Changed, 0.2, cx);
        assert_eq!(state(&b, cx), Some((0.2, false, false)), "no picture: a blank sheet");
        swipe(&b, SwipePhase::Cancelled, 0.0, cx);
        assert_eq!(state(&b, cx), None);
    }

    #[gpui::test]
    fn a_finished_swipe_that_never_lands_still_lets_go(cx: &mut TestAppContext) {
        let (b, cx) = browser(cx);
        swipe(&b, SwipePhase::Began(None), 0.0, cx);
        swipe(&b, SwipePhase::Done, 1.0, cx);
        cx.executor().advance_clock(super::HOLD_MAX);
        cx.run_until_parked();
        assert_eq!(state(&b, cx), None);
    }
}
