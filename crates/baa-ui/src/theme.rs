//! Space tint (PLAN.md §3 "theme tints the whole window chrome") and the
//! OKLCH crossfade used when switching Spaces (§10).

use gpui::{Hsla, Rgba};

/// Every chrome colour is derived from the Space accent, so a crossfade only
/// has to interpolate one value.
#[derive(Clone, Copy)]
pub struct Theme {
    pub accent: Rgba,
    /// Window background behind the sidebar and around the page.
    pub chrome: Hsla,
    pub chrome_hover: Hsla,
    pub chrome_active: Hsla,
    pub text: Hsla,
    pub text_dim: Hsla,
    pub border: Hsla,
}

impl Theme {
    pub fn from_accent(accent: Rgba) -> Self {
        let [_, c, h] = to_oklch(accent);
        // Pastel tint like Arc: keep the hue, clamp chroma, light background.
        let tint = |l: f32, c_max: f32| Hsla::from(from_oklch([l, c.min(c_max), h]));
        Self {
            accent,
            chrome: tint(0.90, 0.06),
            chrome_hover: tint(0.85, 0.07),
            chrome_active: tint(0.97, 0.03),
            text: tint(0.25, 0.03),
            text_dim: tint(0.50, 0.03),
            border: tint(0.80, 0.05),
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

#[cfg(test)]
mod tests {
    use super::*;
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
    fn hue_takes_short_way() {
        // 350° → 10° should pass through 0°, not 180°.
        let a = from_oklch([0.6, 0.15, 350.0]);
        let b = from_oklch([0.6, 0.15, 10.0]);
        let h = to_oklch(lerp_oklch(a, b, 0.5))[2];
        assert!(!(30.0..330.0).contains(&h), "hue {h}");
    }
}
