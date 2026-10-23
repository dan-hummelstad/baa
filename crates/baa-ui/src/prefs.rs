//! Settings (⌘,): a card over the window, in the command bar's style. Every
//! change saves at once; there is no Apply.

use crate::theme::r;
use baa_model::{import, ChromeStyle, SearchEngine, SidebarSide, SEARCH_ENGINES};
use baa_web::WebHost;
use gpui::{div, prelude::*, px, relative, AnyElement, Context, SharedString, Window};

use crate::{
    browser::{drop_in, feel, shadow, Browser},
    icons::icon,
    motion::Feel,
    theme::{ascii, fade, mix, surface, Theme},
};

const PASSWORDS_APP: &str = "/System/Applications/Passwords.app";

/// Archive choices, in hours (0 = never).
const ARCHIVE_AFTER: [(u32, &str); 4] = [(12, "12 hours"), (24, "A day"), (24 * 7, "A week"), (0, "Never")];

impl<H: WebHost> Browser<H> {
    pub(crate) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlays(window, cx);
        self.settings_open = true;
        // Toggles start where they are, then spring when flipped.
        for (key, on) in [
            ("pref-suggest", self.settings.search_suggest),
            ("pref-motion", self.settings.reduce_motion),
        ] {
            let v = on as u8 as f32;
            self.motion.from((key, 0), Feel::Glide, v, v);
        }
        self.motion.from(("settings-in", 0), Feel::Glide, 0.0, 1.0);
        cx.notify();
    }

    fn update_settings(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut baa_model::Settings)) {
        f(&mut self.settings);
        self.save_settings(cx);
    }

    /// A segmented control; `on` picks the highlighted segment.
    fn segments<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        options: Vec<(T, SharedString)>,
        on: T,
        set: fn(&mut baa_model::Settings, T),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .p(px(2.))
            .flex()
            .flex_wrap()
            .gap(px(2.))
            .rounded(r(8.))
            .when(!ascii(), |d| d.bg(surface::key()))
            .children(options.into_iter().enumerate().map(|(i, (value, label))| {
                let selected = value == on;
                let key = (id, i as u64);
                let h = self.motion.get(key, Feel::Hover);
                let el = div()
                    .id((id, i))
                    .h(px(24.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(r(6.))
                    .text_size(px(12.))
                    .when(selected, |d| d.bg(surface::raised()).shadow(vec![shadow(gpui::rgba(0x0000001F).into(), 1., 2.)]))
                    .when(!selected, |d| d.bg(fade(surface::raised().into(), 0.5 * h)))
                    .text_color(if selected { surface::ink() } else { surface::muted() })
                    .when(ascii() && selected, |d| d.bg(surface::key()).text_color(self.theme().accent))
                    .on_click(cx.listener(move |this, _, _, cx| this.update_settings(cx, |s| set(s, value))))
                    .child(if ascii() {
                        SharedString::from(format!("[{} {label} ]", if selected { "◉" } else { " " }))
                    } else {
                        label
                    });
                feel(el, key, cx)
            }))
    }

    fn toggle(
        &self,
        key: &'static str,
        on: bool,
        flip: fn(&mut baa_model::Settings),
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = self.motion.get((key, 0), Feel::Glide);
        if ascii() {
            return div()
                .id(key)
                .text_color(if on { theme.accent } else { surface::icon().into() })
                .on_click(cx.listener(move |this, _, _, cx| this.update_settings(cx, flip)))
                .child(if on { "[x]" } else { "[ ]" })
                .into_any_element();
        }
        div()
            .id(key)
            .w(px(36.))
            .h(px(22.))
            .p(px(2.))
            .rounded(r(11.))
            .bg(mix(surface::line().into(), theme.accent, t.clamp(0., 1.)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.update_settings(cx, flip);
                this.motion.to((key, 0), Feel::Glide, !on as u8 as f32);
            }))
            .child(
                div()
                    .size(px(18.))
                    .ml(px(14. * t))
                    .rounded(r(9.))
                    .bg(gpui::white())
                    .shadow(vec![shadow(gpui::rgba(0x00000033).into(), 1., 3.)]),
            )
            .into_any_element()
    }

    pub(crate) fn render_settings(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.settings_open {
            return None;
        }
        let s = &self.settings;
        let row = |label: SharedString, detail: Option<&'static str>, control: AnyElement| {
            div()
                .py(px(9.))
                .flex()
                .items_center()
                .gap(px(16.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().text_size(px(13.)).child(label))
                        .children(detail.map(|d| div().text_size(px(11.5)).text_color(surface::muted()).child(d))),
                )
                .child(div().flex_none().max_w(px(330.)).flex().justify_end().child(control))
        };
        let section = |title: &'static str| {
            if ascii() {
                return crate::theme::ascii::rule_label(title, surface::icon().into()).pt(px(16.)).pb(px(2.));
            }
            div()
                .pt(px(16.))
                .pb(px(2.))
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(surface::muted())
                .child(title.to_uppercase())
        };
        let engines: Vec<(usize, SharedString)> =
            SEARCH_ENGINES.iter().enumerate().map(|(i, (name, _))| (i, SharedString::from(*name))).collect();
        let engine = SEARCH_ENGINES.iter().position(|(_, url)| *url == s.search_engine.url).unwrap_or(usize::MAX);
        let action_button = |key: crate::motion::Key, label: SharedString, cx: &mut Context<Self>| {
            let h = self.motion.get(key, Feel::Hover);
            feel(
                div()
                    .id(key)
                    .h(px(26.))
                    .px(px(11.))
                    .flex()
                    .items_center()
                    .rounded(r(7.))
                    .bg(mix(surface::key().into(), surface::line().into(), h))
                    .text_size(px(12.5))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(label),
                key,
                cx,
            )
        };
        let history_sources = import::sources();
        let arc = import::arc_sidebar_path().is_some();

        let body = div()
            .id("settings-body")
            .px(px(22.))
            .pb(px(18.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .child(section("Appearance"))
            .child(row(
                "Chrome".into(),
                Some("ASCII draws the window in a monospace terminal frame."),
                self.segments(
                    "pref-chrome",
                    vec![(ChromeStyle::Arc, "Arc".into()), (ChromeStyle::Ascii, "ASCII".into())],
                    s.chrome,
                    |s, c| s.chrome = c,
                    cx,
                )
                .into_any_element(),
            ))
            .child(section("Search"))
            .child(row(
                "Search engine".into(),
                None,
                self.segments(
                    "pref-engine",
                    engines,
                    engine,
                    |s, i| {
                        let (name, url) = SEARCH_ENGINES[i];
                        s.search_engine = SearchEngine { name: name.into(), url: url.into() };
                    },
                    cx,
                )
                .into_any_element(),
            ))
            .child(row(
                "Suggestions while typing".into(),
                Some("Sends what you type in the command bar to the search engine."),
                self.toggle("pref-suggest", s.search_suggest, |s| s.search_suggest = !s.search_suggest, theme, cx)
                    .into_any_element(),
            ))
            .child(section("Sidebar"))
            .child(row(
                "Side".into(),
                None,
                self.segments(
                    "pref-side",
                    vec![(SidebarSide::Left, "Left".into()), (SidebarSide::Right, "Right".into())],
                    s.sidebar_side,
                    |s, side| s.sidebar_side = side,
                    cx,
                )
                .into_any_element(),
            ))
            .child(row(
                "Archive Today tabs after".into(),
                Some("Pinned tabs and favourites are never archived."),
                self.segments(
                    "pref-archive",
                    ARCHIVE_AFTER.iter().map(|(h, l)| (*h, SharedString::from(*l))).collect(),
                    s.archive_after_hours,
                    |s, h| s.archive_after_hours = h,
                    cx,
                )
                .into_any_element(),
            ))
            .child(row(
                "Reduce motion".into(),
                Some("Also follows the system setting."),
                self.toggle("pref-motion", s.reduce_motion, |s| s.reduce_motion = !s.reduce_motion, theme, cx)
                    .into_any_element(),
            ))
            .child(section("Passwords & Autofill"))
            .child(row(
                "Password managers".into(),
                Some("1Password, Bitwarden and others fill logins and passkeys through their Chrome extension; baa connects it to their Mac app."),
                action_button(("pref-extensions", 0), "Get Extension".into(), cx)
                    .on_click(cx.listener(|this, _, window, cx| this.set_up_passwords(window, cx)))
                    .into_any_element(),
            ))
            .child(row(
                "Apple Passwords".into(),
                Some("macOS lets only Chrome and Edge use Apple's iCloud Passwords extension. Logins are a click away in the Passwords app."),
                action_button(("pref-passwords", 0), "Open Passwords".into(), cx)
                    .on_click(cx.listener(|_, _, _, cx| cx.open_with_system(std::path::Path::new(PASSWORDS_APP))))
                    .into_any_element(),
            ))
            .child(section("History"))
            .child(row(
                "Browsing history".into(),
                Some("Kept on this Mac only. Incognito Spaces keep none."),
                action_button(("pref-clear-history", 0), "Clear…".into(), cx)
                    .on_click(cx.listener(|this, _, _, cx| this.clear_history(cx)))
                    .into_any_element(),
            ))
            .child(section("Import"))
            .when(arc, |d| {
                d.child(row(
                    "Spaces from Arc".into(),
                    Some("Pinned tabs, folders and favourites, each Space as a new one."),
                    action_button(("pref-import-arc", 0), "Import".into(), cx)
                        .on_click(cx.listener(|this, _, window, cx| this.import_arc(window, cx)))
                        .into_any_element(),
                ))
            })
            .children(history_sources.into_iter().enumerate().map(|(i, source)| {
                row(
                    format!("History from {}", source.name).into(),
                    None,
                    action_button(("pref-history", i as u64), "Import".into(), cx)
                        .on_click(cx.listener(move |this, _, _, cx| this.import_history(source.clone(), cx)))
                        .into_any_element(),
                )
                .into_any_element()
            }))
            .child(section("Downloads"))
            .child(row("Save files to".into(), Some("~/Downloads"), div().into_any_element()));

        let t = self.motion.get(("settings-in", 0), Feel::Glide);
        let card = div()
            .id("settings")
            .occlude()
            .w(px(560.))
            .max_h(relative(0.8))
            .mt(px(-14. * (1. - t)))
            .opacity(t.clamp(0., 1.))
            .flex()
            .flex_col()
            .rounded(r(14.))
            .bg(surface::bg())
            .border_1()
            .border_color(surface::border())
            .shadow(vec![
                shadow(gpui::rgba(0x18122C52).into(), 30., 80.),
                shadow(gpui::rgba(0x18122C1F).into(), 8., 24.),
            ])
            .text_color(surface::ink())
            .when(ascii(), |d| d.relative().border_color(gpui::transparent_black()).child(crate::theme::ascii::outline(14.)))
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .h(px(52.))
                    .px(px(22.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(surface::line())
                    .child(div().text_size(px(16.)).font_weight(gpui::FontWeight::SEMIBOLD).when(!ascii(), |d| d.child("Settings")))
                    .child(
                        div()
                            .id("settings-close")
                            .size(px(26.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(r(7.))
                            .hover(|s| s.bg(surface::key()))
                            .on_click(cx.listener(|this, _, window, cx| this.close_overlays(window, cx)))
                            .child(icon("x", 14., surface::icon().into())),
                    ),
            )
            .child(body)
            .when(ascii(), |d| d.child(crate::theme::ascii::box_title("SETTINGS", theme.accent)));
        Some(
            div()
                .id("settings-backdrop")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .justify_center()
                .items_start()
                .pt(relative(0.1))
                .bg(fade(surface::scrim().into(), t.clamp(0., 1.)))
                .on_click(cx.listener(|this, _, window, cx| this.close_overlays(window, cx)))
                .child(drop_in(card, "settings"))
                .into_any_element(),
        )
    }
}
