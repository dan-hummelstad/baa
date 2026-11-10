//! baa's look over Chrome's own pages (`baa://flags`, settings subpages …), as
//! one stylesheet the engine injects into every WebUI page of a Space's profile.

use baa_web::WebHost;

use crate::{
    browser::Browser,
    theme::{surface, Theme},
};

impl<H: WebHost> Browser<H> {
    /// Tells the engine the skin for the active Space's Chrome pages when it changed.
    pub(crate) fn push_webui_skin(&mut self) {
        let Some(profile) = self.active_space().map(|s| s.profile) else { return };
        // Dark surfaces, so Chrome's own light/dark rules must be the dark ones.
        let ((top, _, bottom), dark) = (self.gradient_to, crate::theme::ascii() || self.dark);
        let key = [top.r, top.g, top.b, bottom.r, bottom.g, bottom.b].iter().fold(crate::theme::ascii() as u64 + 2 * self.dark as u64, |h, c| h.rotate_left(7) ^ u64::from(c.to_bits()));
        if self.skin_sent == Some((profile, key)) {
            return;
        }
        self.skin_sent = Some((profile, key));
        let css = chrome_skin(&Theme::from_gradient(top, bottom));
        self.host.set_webui_skin(profile, css, dark);
    }
}

/// baa's look over Chrome's own pages, through the variables Chrome's pages
/// take their colours, type and shape from. Variables, not element rules, so
/// Chrome's updates keep working.
pub(crate) fn chrome_skin(theme: &Theme) -> String {
    let ascii = crate::theme::ascii();
    let hex = |c: gpui::Rgba| {
        let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
        format!("#{:02x}{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b), byte(c.a))
    };
    let accent = hex(theme.accent.into());
    let rgb = |c: gpui::Rgba| format!("{}, {}, {}", (c.r * 255.).round(), (c.g * 255.).round(), (c.b * 255.).round());
    let (bg, panel, key, line, border, ink, muted, on_accent, font, radius, scheme) = if ascii {
        (
            surface::bg(),
            gpui::rgb(0x0D121B),
            surface::key(),
            surface::line(),
            surface::border(),
            surface::ink(),
            surface::muted(),
            gpui::rgb(0x04131B),
            "\"JetBrains Mono\", \"SF Mono\", Menlo, monospace",
            "3px",
            "dark",
        )
    } else if crate::theme::dark() {
        (
            surface::bg(),
            gpui::rgb(0x24212C),
            surface::key(),
            surface::line(),
            surface::border(),
            surface::ink(),
            surface::muted(),
            gpui::rgb(0xFFFFFF),
            "Inter, -apple-system, system-ui, sans-serif",
            "12px",
            "dark",
        )
    } else {
        (
            surface::bg(),
            gpui::rgb(0xF7F6FA),
            gpui::rgb(0xEFEDF3),
            surface::line(),
            surface::border(),
            surface::ink(),
            surface::muted(),
            gpui::rgb(0xFFFFFF),
            "Inter, -apple-system, system-ui, sans-serif",
            "12px",
            "light",
        )
    };
    let wash = format!("{}2e", &accent[..7]);
    let (bg, panel, key, line, border, ink_rgb, ink, muted, on_accent) = (
        hex(bg), hex(panel), hex(key), hex(line), hex(border), rgb(ink), hex(ink), hex(muted), hex(on_accent),
    );
    let vars = [
        ("--cr-fallback-color-primary", &accent),
        ("--cr-fallback-color-on-primary", &on_accent),
        ("--cr-fallback-color-primary-container", &wash),
        ("--cr-fallback-color-on-primary-container", &ink),
        ("--cr-fallback-color-surface", &bg),
        ("--cr-fallback-color-surface1", &panel),
        ("--cr-fallback-color-surface2", &panel),
        ("--cr-fallback-color-surface3", &key),
        ("--cr-fallback-color-surface4", &key),
        ("--cr-fallback-color-surface5", &key),
        ("--cr-fallback-color-surface-variant", &key),
        ("--cr-fallback-color-base-container", &panel),
        ("--cr-fallback-color-neutral-container", &key),
        ("--cr-fallback-color-tonal-container", &wash),
        ("--cr-fallback-color-on-tonal-container", &ink),
        ("--cr-fallback-color-secondary-container", &wash),
        ("--cr-fallback-color-on-secondary-container", &ink),
        ("--cr-fallback-color-inverse-surface", &ink),
        ("--cr-fallback-color-on-surface", &ink),
        ("--cr-fallback-color-on-surface-rgb", &ink_rgb),
        ("--cr-fallback-color-on-surface-variant", &muted),
        ("--cr-fallback-color-on-surface-subtle", &muted),
        ("--cr-fallback-color-outline", &border),
        ("--cr-fallback-color-neutral-outline", &border),
        ("--cr-fallback-color-tonal-outline", &border),
        ("--cr-fallback-color-divider", &line),
        ("--cr-primary-text-color", &ink),
        ("--cr-secondary-text-color", &muted),
        ("--cr-form-field-label-color", &muted),
        ("--cr-link-color", &accent),
        ("--cr-checked-color", &accent),
        ("--cr-focus-outline-color", &accent),
        ("--cr-separator-color", &line),
        ("--cr-scrollable-border-color", &line),
        ("--cr-menu-background-color", &panel),
        ("--cr-card-background-color", &bg),
    ];
    let mut css = String::from(":root, html {\n");
    for (name, value) in vars {
        css += &format!("  {name}: {value} !important;\n");
    }
    // Chrome's components pick their blues (selection, links, focus) from
    // the Google palette.
    let accent_rgb = rgb(theme.accent.into());
    for shade in [50, 100, 200, 300, 400, 500, 600, 700, 800, 900] {
        let colour = if shade < 200 { &wash } else { &accent };
        css += &format!("  --google-blue-{shade}: {colour} !important;\n  --google-blue-{shade}-rgb: {accent_rgb} !important;\n");
    }
    css += &format!(
        "  --cr-toolbar-search-field-border-radius: {radius} !important;\n  --cr-toolbar-search-field-background: {key} !important;\n"
    );
    css += &format!(
        "  --cr-card-border-radius: {radius} !important;\n  --cr-card-shadow: none !important;\n  --cr-hover-background-color: {line} !important;\n  color-scheme: {scheme} !important;\n}}\nhtml, body {{ background: {bg} !important; color: {ink} !important; font-family: {font} !important; }}\n"
    );
    // Pages that use no cr variables (net-internals, gpu, version, flags tables): generic elements only,
    // never per-page rules, so Chrome's updates keep working.
    css += &format!(
        "table, th, td {{ border-color: {line} !important; }}\nth {{ background: {key} !important; color: {ink} !important; }}\ninput, select, textarea, button {{ background: {key} !important; color: {ink} !important; border: 1px solid {border} !important; border-radius: {radius}; font-family: {font}; }}\na {{ color: {accent} !important; }}\ncode, pre {{ background: {key} !important; color: {ink} !important; font-family: \"JetBrains Mono\", Menlo, monospace; }}\n"
    );
    css
}

