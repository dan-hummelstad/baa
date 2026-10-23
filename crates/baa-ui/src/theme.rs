//! Space theme (PLAN.md §3 "theme tints the whole window chrome"): a two-stop
//! gradient per Space, and every chrome colour derived from it, so a Space
//! switch crossfades two colours (§10). Values follow the pen.dev mocks.

use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{hsla, px, Hsla, Pixels, Rgba};

// ponytail: one process-wide style, since every window shares the setting.
static ASCII: AtomicBool = AtomicBool::new(false);

/// ASCII Chrome (pen.dev "1. ASCII Chrome"): monospace, navy, box rules.
pub fn ascii() -> bool {
    ASCII.load(Ordering::Relaxed)
}

pub fn set_ascii(on: bool) {
    ASCII.store(on, Ordering::Relaxed);
}

/// A corner radius; ASCII Chrome only rounds as much as a `╭` does.
pub fn r(v: f32) -> Pixels {
    px(if ascii() { v.min(3.) } else { v })
}

pub const MONO: &str = "JetBrains Mono";

/// ASCII Chrome's fixed colours (pen.dev "1. ASCII Chrome").
pub mod ascii {
    use gpui::{rgb, Rgba};
    /// Box-drawing frames around the page and overlays.
    pub fn frame() -> Rgba {
        rgb(0x2C3950)
    }
    /// Section rules and favourite boxes.
    pub fn rule() -> Rgba {
        rgb(0x1B2433)
    }
    /// "── LABEL ─────────" to the container's width.
    pub fn rule_label(label: &'static str, ink: gpui::Hsla) -> gpui::Div {
        use gpui::{div, prelude::*};
        // flex_none: in a scrolling column it would shrink and clip its own text.
        div()
            .flex()
            .flex_none()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .child(div().flex_none().text_color(ink).child(format!("── {label} ")))
            .child(div().flex_1().overflow_hidden().text_color(rule()).child("─".repeat(160)))
    }
    /// A box's outline. gpui paints a div's own border over its children, so a
    /// box with a title draws its outline as its first child instead.
    pub fn outline(radius: f32) -> gpui::Div {
        use gpui::{div, prelude::*};
        div().absolute().inset_0().border_1().border_color(frame()).rounded(super::r(radius))
    }
    /// A box's label sitting on its top edge: "╭─ LABEL ───╮". Add it last.
    pub fn box_title(label: &'static str, accent: gpui::Hsla) -> gpui::Div {
        use gpui::{div, prelude::*, px};
        div()
            .absolute()
            .top(px(-9.))
            .left(px(14.))
            .px(px(5.))
            .bg(super::surface::bg())
            .text_size(px(12.5))
            .text_color(accent)
            .child(label)
    }
}

#[derive(Clone, Copy)]
pub struct Theme {
    /// Window gradient, top and bottom.
    pub bg_top: Rgba,
    pub bg_bottom: Rgba,
    /// Text and icons on the gradient, and its muted and faint variants.
    pub ink: Hsla,
    pub ink_muted: Hsla,
    pub ink_faint: Hsla,
    /// Translucent white fills: resting (URL bar, hover) and selected (active tab).
    pub glass: Hsla,
    pub glass_strong: Hsla,
    /// Hovered rows and buttons on the gradient.
    pub hover: Hsla,
    pub divider: Hsla,
    /// Saturated Space colour: command bar selection, focus rings.
    pub accent: Hsla,
}

/// Light surfaces that sit on any Space: command bar, peek, menus.
pub mod surface {
    use gpui::{rgb, rgba, Rgba};
    pub fn bg() -> Rgba {
        if super::ascii() { rgb(0x090C12) } else { rgba(0xFFFFFFF7) }
    }
    pub fn ink() -> Rgba {
        if super::ascii() { rgb(0xC7D2E4) } else { rgb(0x17151D) }
    }
    pub fn muted() -> Rgba {
        if super::ascii() { rgb(0x7F8DA6) } else { rgb(0x8A8796) }
    }
    pub fn icon() -> Rgba {
        if super::ascii() { rgb(0x4C5A70) } else { rgb(0x6E6A7A) }
    }
    pub fn line() -> Rgba {
        if super::ascii() { rgb(0x1B2433) } else { rgb(0xEFEDF3) }
    }
    pub fn border() -> Rgba {
        if super::ascii() { rgb(0x2C3950) } else { rgba(0x18122C29) }
    }
    pub fn key() -> Rgba {
        if super::ascii() { rgb(0x102535) } else { rgb(0xF3F2F6) }
    }
    /// A raised segment or knob.
    pub fn raised() -> Rgba {
        if super::ascii() { rgb(0x16334C) } else { rgb(0xFFFFFF) }
    }
    pub fn scrim() -> Rgba {
        if super::ascii() { rgba(0x04070CCC) } else { rgba(0x1814243D) }
    }
}

impl Theme {
    pub fn from_gradient(top: Rgba, bottom: Rgba) -> Self {
        if ascii() {
            return Self::ascii(bottom);
        }
        let [l, c, h] = to_oklch(bottom);
        let dark = l < 0.45;
        let with_alpha = |c: Hsla, a: f32| Hsla { a, ..c };
        let ink = if dark {
            Hsla::from(from_oklch([0.96, 0.01, h]))
        } else {
            Hsla::from(from_oklch([0.22, c.min(0.04), h]))
        };
        let white = hsla(0., 0., 1., 1.);
        Self {
            bg_top: top,
            bg_bottom: bottom,
            ink,
            ink_muted: with_alpha(ink, 0.56),
            ink_faint: with_alpha(ink, 0.30),
            glass: with_alpha(white, if dark { 0.08 } else { 0.24 }),
            glass_strong: with_alpha(white, if dark { 0.15 } else { 0.91 }),
            hover: with_alpha(white, if dark { 0.10 } else { 0.45 }),
            divider: with_alpha(if dark { white } else { ink }, 0.12),
            accent: Hsla::from(from_oklch([if dark { 0.65 } else { 0.52 }, c.clamp(0.12, 0.2), h])),
        }
    }
}

impl Theme {
    /// The Space's hue survives as the accent and a tint at the top of the navy.
    fn ascii(space: Rgba) -> Self {
        let [_, c, h] = to_oklch(space);
        let rgb = |hex: u32| Hsla::from(gpui::rgb(hex));
        Self {
            bg_top: from_oklch([0.31, 0.055, h]),
            bg_bottom: gpui::rgb(0x06080D),
            ink: rgb(0xC7D2E4),
            ink_muted: rgb(0x7F8DA6),
            ink_faint: rgb(0x4C5A70),
            glass: rgb(0x0E1D2B),
            glass_strong: rgb(0x102535),
            hover: rgb(0x0E1D2B),
            divider: rgb(0x1B2433),
            accent: Hsla::from(from_oklch([0.85, c.clamp(0.10, 0.15), h])),
        }
    }
}

/// A Space's gradient: stored stops, or for a single colour, a light-to-mid
/// ramp of its hue.
pub fn space_gradient(accent: u32, accent_to: Option<u32>) -> (Rgba, Rgba) {
    let from = gpui::rgb(accent);
    match accent_to {
        Some(to) => (from, gpui::rgb(to)),
        None => {
            let [_, c, h] = to_oklch(from);
            (from_oklch([0.88, (c * 0.5).min(0.06), h]), from_oklch([0.72, c.min(0.13), h]))
        }
    }
}

pub fn lerp_oklch(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let (a_lch, b_lch) = (to_oklch(a), to_oklch(b));
    // Shortest way round the hue circle, so red→magenta does not pass green.
    let mut dh = b_lch[2] - a_lch[2];
    if dh > 180.0 {
        dh -= 360.0;
    } else if dh < -180.0 {
        dh += 360.0;
    }
    let mut out = from_oklch([
        a_lch[0] + (b_lch[0] - a_lch[0]) * t,
        a_lch[1] + (b_lch[1] - a_lch[1]) * t,
        (a_lch[2] + dh * t).rem_euclid(360.0),
    ]);
    out.a = a.a + (b.a - a.a) * t;
    out
}

/// sRGB → [L, C, h°] (Björn Ottosson's OKLab).
// Ottosson's published constants, kept verbatim so they can be checked against the source.
#[allow(clippy::excessive_precision)]
pub fn to_oklch(c: Rgba) -> [f32; 3] {
    let lin = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(c.r), lin(c.g), lin(c.b));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let ok_l = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
    let ok_a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
    let ok_b = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
    [
        ok_l,
        ok_a.hypot(ok_b),
        ok_b.atan2(ok_a).to_degrees().rem_euclid(360.0),
    ]
}

#[allow(clippy::excessive_precision)]
pub fn from_oklch([ok_l, c, h]: [f32; 3]) -> Rgba {
    let (ok_a, ok_b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l = (ok_l + 0.3963377774 * ok_a + 0.2158037573 * ok_b).powi(3);
    let m = (ok_l - 0.1055613458 * ok_a - 0.0638541728 * ok_b).powi(3);
    let s = (ok_l - 0.0894841775 * ok_a - 1.2914855480 * ok_b).powi(3);
    let r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
    let g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
    let b = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;
    let gamma = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    };
    Rgba {
        r: gamma(r),
        g: gamma(g),
        b: gamma(b),
        a: 1.0,
    }
}

/// `c` with its alpha scaled by `t` (fading a hover tint in).
pub fn fade(c: Hsla, t: f32) -> Hsla {
    Hsla {
        a: c.a * t.clamp(0.0, 1.0),
        ..c
    }
}

/// Straight blend in RGB; a transparent end takes the other's colour so the
/// blend never passes through black.
pub fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let t = t.clamp(0.0, 1.0);
    let (mut x, mut y) = (a.to_rgb(), b.to_rgb());
    if x.a == 0.0 {
        x = Rgba { a: 0.0, ..y };
    }
    if y.a == 0.0 {
        y = Rgba { a: 0.0, ..x };
    }
    let l = |p: f32, q: f32| p + (q - p) * t;
    Rgba {
        r: l(x.r, y.r),
        g: l(x.g, y.g),
        b: l(x.b, y.b),
        a: l(x.a, y.a),
    }
    .into()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_with_transparent_keeps_the_colour() {
        let white: Hsla = gpui::white();
        let m = mix(gpui::black(), white, 0.5).to_rgb();
        assert!((m.r - 0.5).abs() < 1e-3);
        let m = mix(gpui::transparent_black(), white, 0.5).to_rgb();
        assert!((m.r - 1.0).abs() < 1e-3, "transparent end takes the other's colour");
        assert_eq!(fade(white, 0.5).a, 0.5);
    }
    use gpui::rgb;

    fn close(a: Rgba, b: Rgba) -> bool {
        (a.r - b.r).abs() < 0.002 && (a.g - b.g).abs() < 0.002 && (a.b - b.b).abs() < 0.002
    }

    #[test]
    fn oklch_round_trips() {
        for hex in [0xff0000, 0x00ff00, 0x0000ff, 0x6c9cff, 0x808080, 0xffffff] {
            let c = rgb(hex);
            assert!(close(from_oklch(to_oklch(c)), c), "{hex:06x}");
        }
    }

    #[test]
    fn lerp_endpoints_and_midpoint_lightness() {
        let (a, b) = (rgb(0xff0000), rgb(0x0000ff));
        assert!(close(lerp_oklch(a, b, 0.0), a));
        assert!(close(lerp_oklch(a, b, 1.0), b));
        let mid = to_oklch(lerp_oklch(a, b, 0.5))[0];
        let expect = (to_oklch(a)[0] + to_oklch(b)[0]) / 2.0;
        assert!(
            (mid - expect).abs() < 0.02,
            "perceptual midpoint, not RGB mud"
        );
    }

    #[test]
    fn light_spaces_get_dark_ink_and_dark_spaces_light_ink() {
        let light = Theme::from_gradient(rgb(0xDCCFFB), rgb(0xA592EC));
        assert!(light.ink.l < 0.3);
        let night = Theme::from_gradient(rgb(0x34304A), rgb(0x16151E));
        assert!(night.ink.l > 0.9);
    }

    #[test]
    fn ascii_keeps_the_space_hue_on_navy() {
        let t = Theme::ascii(rgb(0x6C9CFF));
        assert!(t.ink.l > 0.8 && to_oklch(t.bg_bottom)[0] < 0.2);
        let (accent, space) = (to_oklch(t.accent.to_rgb()), to_oklch(rgb(0x6C9CFF)));
        assert!((accent[2] - space[2]).abs() < 15. && accent[0] > 0.75, "gamut clipping may shift it a little: {accent:?}");
    }

    #[test]
    fn hue_takes_short_way() {
        // 350° → 10° should pass through 0°, not 180°.
        let a = from_oklch([0.6, 0.15, 350.0]);
        let b = from_oklch([0.6, 0.15, 10.0]);
        let h = to_oklch(lerp_oklch(a, b, 0.5))[2];
        assert!(!(30.0..330.0).contains(&h), "hue {h}");
    }
}
