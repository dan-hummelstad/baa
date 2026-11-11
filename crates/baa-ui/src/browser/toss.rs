//! The download toss: a new download's icon is thrown from the pointer and drops
//! into the sidebar's Library button, or into a bin at the window's corner while
//! the sidebar is hidden. The motion and its numbers are Arc's (its DownloadsUI
//! module, read from the app): a projectile under gravity, drawn every frame.

use std::time::Instant;

use baa_model::SidebarSide;
use baa_web::WebHost;
use gpui::{div, prelude::*, px, AnyElement, Context, Transformation, Window};

use super::{shadow, Browser};
use crate::{
    icons::icon,
    motion::Feel,
    theme::{r, surface, Theme},
};

/// Gravity in points/s²: 9.81 × 270.
const G: f32 = 2648.7;
/// The launch speed upwards from the window's bottom edge. A launch from higher
/// up gets less (`LIFT_LOST` less at the top), so the arc stays in the window.
const LIFT: f32 = 1600.;
const LIFT_LOST: f32 = 1100.;
/// The icon sinks on through the Library button for this long, still shrinking.
const SINK: f32 = 8. / 60.;
/// The bin stays this long after its icon landed.
const BIN_HOLD: f32 = 1.;
const ICON: f32 = 30.;
const BIN: f32 = 62.;
/// The bin's centre, from the window's side and bottom edges.
const BIN_AT: (f32, f32) = (53., 40.);
/// The icon lands this far past the bin's centre and rests as far before it.
const BIN_PAST: f32 = 8.;

/// One thrown icon. Window coordinates (y down), speeds in points/s.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Toss {
    from: (f32, f32),
    /// The speed sideways, and upwards at the launch.
    speed: (f32, f32),
    /// Seconds to the landing.
    flight: f32,
    /// Lands in the bin and comes to rest at this x; `None` sinks into the Library button.
    rest: Option<f32>,
}

struct Frame {
    at: (f32, f32),
    scale: (f32, f32),
    alpha: f32,
    /// Radians, clockwise.
    turn: f32,
}

/// What is left of a spring's way `t` seconds after it starts from rest: 1 at
/// first, 0 once settled (SwiftUI's `response` and `dampingFraction`).
fn left(t: f32, response: f32, damping: f32) -> f32 {
    let w = std::f32::consts::TAU / response;
    if damping >= 1. {
        return (1. + w * t) * (-w * t).exp();
    }
    let wd = w * (1. - damping * damping).sqrt();
    (-damping * w * t).exp() * ((wd * t).cos() + damping * w / wd * (wd * t).sin())
}

impl Toss {
    /// `None` when the throw cannot reach `to` (it is too far above `from`).
    fn new(from: (f32, f32), to: (f32, f32), window_h: f32, rest: Option<f32>) -> Option<Self> {
        let up = if window_h > 0. { LIFT - LIFT_LOST * (window_h - from.1) / window_h } else { LIFT };
        // from.y − up·t + G/2·t² = to.y, the first time it is true.
        let root = (up * up - 2. * G * (from.1 - to.1)).sqrt();
        let flight = [(up - root) / G, (up + root) / G].into_iter().find(|t| *t > 0.)?;
        Some(Self { from, speed: ((to.0 - from.0) / flight, up), flight, rest })
    }

    fn ends(&self) -> f32 {
        self.flight + if self.rest.is_some() { BIN_HOLD } else { SINK }
    }

    fn at(&self, t: f32) -> Frame {
        // In the bin the fall ends at the landing; at the button it goes on.
        let fall = if self.rest.is_some() { t.min(self.flight) } else { t };
        let mut x = self.from.0 + self.speed.0 * fall;
        let y = self.from.1 - self.speed.1 * fall + G / 2. * fall * fall;
        if let Some(rest) = self.rest {
            // ponytail: the wobble starts from rest. Arc's carries the landing's speed into
            // it, by an amount its binary does not show plainly; give `left` a start speed
            // if the landing looks too soft.
            x = rest + (x - rest) * left((t - self.flight).max(0.), 0.72, 0.48);
        }
        // Stretched at the launch, largest at the top, small at the landing.
        let top = self.speed.1 / G;
        let scale = if fall < top {
            let k = fall / top;
            (0.8 + 0.8 * k, 1.2 + 0.4 * k)
        } else {
            let k = if self.flight > top { (fall - top) / (self.flight - top) } else { 0. };
            let s = (1.6 - if self.rest.is_some() { 0.9 } else { 1.3 } * k).max(0.);
            (s, s)
        };
        Frame {
            at: (x, y),
            scale,
            alpha: 1. - left(t, 0.3, 1.),
            turn: (5. - 40. * left(t, 1.2, 1.)).to_radians(),
        }
    }
}

impl<H: WebHost> Browser<H> {
    /// A download began in this window: throws its icon from the pointer.
    pub(super) fn toss_download(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cx.reduce_motion() || self.fullscreen.is_some() {
            return;
        }
        let size = window.viewport_size();
        let (w, h) = (f32::from(size.width), f32::from(size.height));
        let pointer = window.mouse_position();
        let pointer = (f32::from(pointer.x), f32::from(pointer.y));
        // A pointer outside the window did not start it: thrown from the middle then.
        let inside = (0. ..=w).contains(&pointer.0) && (0. ..=h).contains(&pointer.1);
        let from = if inside { pointer } else { (w / 2., h / 2.) };
        let button = self.library_at.get();
        let shown = self.docked() || self.reveal.value() > 0.5;
        let (to, rest) = if shown && button.size.width > px(0.) {
            let c = button.center();
            ((f32::from(c.x), f32::from(c.y)), None)
        } else {
            let (x, past) = match self.side() {
                SidebarSide::Left => (BIN_AT.0, -BIN_PAST),
                SidebarSide::Right => (w - BIN_AT.0, BIN_PAST),
            };
            ((x + past, h - BIN_AT.1), Some(x - past))
        };
        if let Some(toss) = Toss::new(from, to, h, rest) {
            self.tosses.push((toss, cx.background_executor().now()));
            cx.notify();
        }
    }

    /// Returns whether another frame is needed.
    pub(super) fn tick_tosses(&mut self, now: Instant, dt: f32, reduce: bool) -> bool {
        if reduce {
            self.tosses.clear();
        }
        let over = |(toss, born): &(Toss, Instant)| (now - *born).as_secs_f32() >= toss.ends();
        // The Library button takes what sank into it with a hop.
        if self.tosses.iter().any(|t| t.0.rest.is_none() && over(t)) {
            self.motion.from(("download-hop", 1), Feel::Glide, 1.0, 0.0);
        }
        let holding = self.tosses.iter().any(|t| t.0.rest.is_some() && !over(t));
        self.bin.set_target(holding as u8 as f32);
        let sliding = self.bin.tick(dt, reduce);
        // An icon in the bin leaves with it.
        let bin_in = self.bin.value() > 0.01;
        self.tosses.retain(|t| !over(t) || (t.0.rest.is_some() && bin_in));
        sliding || !self.tosses.is_empty()
    }

    /// The bin and the icons in the air, over everything else. Only the bin takes clicks.
    pub(super) fn render_tosses(&self, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let now = cx.background_executor().now();
        let right = self.side() == SidebarSide::Right;
        let lift = || shadow(gpui::rgba(0x18122C29).into(), 8., 24.);
        // The bin comes in from the window's side.
        let out = 1. - self.bin.value();
        let slide = out * (BIN_AT.0 + BIN) * if right { 1. } else { -1. };
        let mut els = Vec::new();
        if out < 0.99 {
            let side = BIN_AT.0 - BIN / 2. - out * (BIN_AT.0 + BIN);
            let bin = div().id("download-bin").absolute().bottom(px(BIN_AT.1 - BIN / 2.)).size(px(BIN));
            let bin = if right { bin.right(px(side)) } else { bin.left(px(side)) };
            els.push(
                bin.occlude()
                    .rounded(r(12.))
                    .bg(surface::bg())
                    .border_1()
                    .border_color(surface::border())
                    .shadow(vec![lift()])
                    .on_click(cx.listener(|this, _, _, cx| this.open_library(crate::library::Section::Downloads, cx)))
                    .into_any_element(),
            );
        }
        for (toss, born) in &self.tosses {
            let t = (now - *born).as_secs_f32();
            let f = toss.at(t);
            let landed = toss.rest.is_some() && t >= toss.flight;
            let x = f.at.0 + if landed { slide } else { 0. };
            let (w, h) = (ICON * f.scale.0, ICON * f.scale.1);
            els.push(
                div()
                    .absolute()
                    .left(px(x - ICON / 2.))
                    .top(px(f.at.1 - ICON / 2.))
                    .size(px(ICON))
                    .opacity(f.alpha)
                    // A div cannot be scaled: the disc behind the glyph is sized instead.
                    .child(
                        div()
                            .absolute()
                            .left(px((ICON - w) / 2.))
                            .top(px((ICON - h) / 2.))
                            .w(px(w))
                            .h(px(h))
                            .rounded(r(ICON))
                            .bg(surface::bg())
                            .shadow(vec![lift()]),
                    )
                    .child(
                        icon("circle-arrow-down", ICON, theme.accent).absolute().left_0().top_0().with_transformation(
                            Transformation::rotate(gpui::radians(f.turn)).with_scaling(gpui::size(f.scale.0, f.scale.1)),
                        ),
                    )
                    .into_any_element(),
            );
        }
        els
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: f32 = 900.;

    #[test]
    fn a_toss_rises_from_the_pointer_and_lands_on_the_library_button() {
        let toss = Toss::new((700., 450.), (20., 860.), H, None).unwrap();
        // Arc's numbers: from mid-window the launch is 1050 pt/s, the top 0.4 s later.
        assert!((toss.speed.1 - 1050.).abs() < 0.01);
        let start = toss.at(0.);
        assert_eq!(start.at, (700., 450.));
        assert_eq!(start.scale, (0.8, 1.2));
        assert!(start.alpha.abs() < 1e-6 && (start.turn + 35f32.to_radians()).abs() < 1e-6);
        let top = toss.at(1050. / G);
        assert!(top.at.1 < 450. - 200., "it goes up first: {:?}", top.at);
        assert!((top.scale.0 - 1.6).abs() < 1e-4 && (top.scale.1 - 1.6).abs() < 1e-4);
        let landing = toss.at(toss.flight);
        assert!((landing.at.0 - 20.).abs() < 0.1 && (landing.at.1 - 860.).abs() < 0.1, "{:?}", landing.at);
        assert!((landing.scale.0 - 0.3).abs() < 1e-4);
        assert!(landing.alpha > 0.99);
        let sunk = toss.at(toss.ends());
        assert!(sunk.at.1 > 860. && sunk.scale.0 < 0.3 && sunk.scale.0 >= 0.);
    }

    #[test]
    fn a_higher_launch_is_thrown_less_hard() {
        let low = Toss::new((400., 880.), (20., 860.), H, None).unwrap();
        let high = Toss::new((400., 60.), (20., 860.), H, None).unwrap();
        assert!(low.speed.1 > high.speed.1 && high.speed.1 > LIFT - LIFT_LOST);
        // The top of the arc stays in the window.
        let top = high.at(high.speed.1 / G).at.1;
        assert!(top > -ICON, "{top}");
    }

    #[test]
    fn in_the_bin_it_stops_and_comes_to_rest() {
        let toss = Toss::new((700., 450.), (45., 860.), H, Some(61.)).unwrap();
        let landing = toss.at(toss.flight);
        assert!((landing.at.0 - 45.).abs() < 0.1 && (landing.scale.0 - 0.7).abs() < 1e-4);
        let held = toss.at(toss.ends());
        assert!((held.at.0 - 61.).abs() < 0.5 && (held.at.1 - 860.).abs() < 0.1, "{:?}", held.at);
        assert!((held.scale.0 - 0.7).abs() < 1e-4);
    }

    #[test]
    fn a_target_out_of_reach_is_not_thrown_at() {
        assert!(Toss::new((400., 880.), (400., 0.), H, None).is_none());
    }
}
