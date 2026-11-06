//! Privacy & Security (with the clear-data sheet) and Site Settings.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use baa_model::{Cookies, Https, Permission, PermissionState, Preload, SecureDns, Settings, SpaceId};
use baa_web::{ClearData, ProfileId, WebHost};
use gpui::{div, prelude::*, px, AnyElement, Context, SharedString};

use super::{rows::site_row, Field, Item, SettingsView};
use crate::{
    browser::{shadow, Browser},
    theme::{ascii, fade, mix, r, surface},
};

const CLOUDFLARE: &str = "https://chrome.cloudflare-dns.com/dns-query";
const QUAD9: &str = "https://dns.quad9.net/dns-query";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Dns {
    Off,
    Automatic,
    Cloudflare,
    Quad9,
    Custom,
}

fn dns_of(d: &SecureDns) -> Dns {
    match d {
        SecureDns::Off => Dns::Off,
        SecureDns::Automatic => Dns::Automatic,
        SecureDns::Custom(t) if t == CLOUDFLARE => Dns::Cloudflare,
        SecureDns::Custom(t) if t == QUAD9 => Dns::Quad9,
        SecureDns::Custom(_) => Dns::Custom,
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Range {
    Hour,
    Day,
    Week,
    Month,
    All,
}

impl Range {
    pub(crate) fn duration(self) -> Option<Duration> {
        let hours = match self {
            Range::Hour => 1,
            Range::Day => 24,
            Range::Week => 24 * 7,
            Range::Month => 24 * 28,
            Range::All => return None,
        };
        Some(Duration::from_secs(hours * 3600))
    }
}

/// What the clear-data sheet has ticked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ClearSheet {
    pub range: Range,
    pub history: bool,
    pub downloads: bool,
    pub site_data: bool,
    pub cache: bool,
    pub all_spaces: bool,
}

impl Default for ClearSheet {
    fn default() -> Self {
        Self { range: Range::Hour, history: true, downloads: false, site_data: true, cache: true, all_spaces: false }
    }
}

fn state_label(state: PermissionState) -> &'static str {
    match state {
        PermissionState::Ask => "Ask",
        PermissionState::Allow => "Allow",
        PermissionState::Block => "Block",
        PermissionState::SessionOnly => "Clear on quit",
    }
}

/// The states a user can give `kind`, `Ask` not counted for cookies.
fn states(kind: Permission) -> Vec<PermissionState> {
    [PermissionState::Ask, PermissionState::Allow, PermissionState::Block, PermissionState::SessionOnly]
        .into_iter()
        .filter(|s| kind.takes(*s) && !(kind == Permission::Cookies && *s == PermissionState::Ask))
        .collect()
}

/// "example.com" covers its subdomains too; anything with its own syntax stays as typed.
fn pattern_of(text: &str) -> String {
    let t = text.trim();
    if t.is_empty() || t.contains(['[', '*', '/']) {
        t.to_string()
    } else {
        format!("[*.]{t}")
    }
}

impl<H: WebHost> SettingsView<H> {
    pub(super) fn privacy(&self, s: &Settings, cx: &mut Context<Self>) -> Vec<Item> {
        let dns = dns_of(&s.web.secure_dns);
        let template = self.text(Field::DnsTemplate).to_string();
        let mut out = vec![
            self.section("Tracking"),
            self.row(
                "cookies",
                "Block all also stops sites you allowed below.",
                self.segments(
                    "pref-cookies",
                    vec![(Cookies::AllowAll, "Allow all".into()), (Cookies::BlockThirdParty, "Block third-party".into()), (Cookies::BlockAll, "Block all".into())],
                    s.web.cookies,
                    |v, s| s.web.cookies = v,
                    cx,
                ),
            ),
            self.row("clear_on_quit", "", self.toggle("pref-clear-quit", s.web.clear_cookies_on_quit, |s, v| s.web.clear_cookies_on_quit = v, cx)),
            self.row("dnt", "", self.toggle("pref-dnt", s.web.do_not_track, |s, v| s.web.do_not_track = v, cx)),
            self.row("gpc", "Tells sites not to sell or share your data.", self.toggle("pref-gpc", s.web.global_privacy_control, |s, v| s.web.global_privacy_control = v, cx)),
            self.section("Connections"),
            self.row(
                "https",
                "",
                self.segments(
                    "pref-https",
                    vec![(Https::Off, "Off".into()), (Https::Balanced, "Upgrade when possible".into()), (Https::Strict, "Always (warn first)".into())],
                    s.web.https,
                    |v, s| s.web.https = v,
                    cx,
                ),
            ),
            self.row(
                "preload",
                "Loads pages you are likely to open ahead of time.",
                self.segments(
                    "pref-preload",
                    vec![(Preload::Off, "Off".into()), (Preload::Standard, "Standard".into()), (Preload::Extended, "Extended".into())],
                    s.web.preload,
                    |v, s| s.web.preload = v,
                    cx,
                ),
            ),
            self.row(
                "dns",
                "",
                self.segments(
                    "pref-dns",
                    vec![
                        (Dns::Off, "Off".into()),
                        (Dns::Automatic, "Automatic".into()),
                        (Dns::Cloudflare, "Cloudflare".into()),
                        (Dns::Quad9, "Quad9".into()),
                        (Dns::Custom, "Custom".into()),
                    ],
                    dns,
                    move |v, s| {
                        s.web.secure_dns = match v {
                            Dns::Off => SecureDns::Off,
                            Dns::Automatic => SecureDns::Automatic,
                            Dns::Cloudflare => SecureDns::Custom(CLOUDFLARE.into()),
                            Dns::Quad9 => SecureDns::Custom(QUAD9.into()),
                            Dns::Custom => SecureDns::Custom(template.clone()),
                        }
                    },
                    cx,
                ),
            ),
        ];
        if dns == Dns::Custom {
            out.push(self.sub_row("dns-template".into(), "Server address".into_any_element(), self.field(Field::DnsTemplate, "dns-field", "https://…/dns-query", 260., cx)));
        }
        out.push(self.section("Data"));
        out.push(self.row(
            "clear_data",
            "History, downloads, cookies and cache, for a time range.",
            self.button("clear-open", "Clear…", false, cx)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.clear = Some(ClearSheet::default());
                    cx.notify();
                }))
                .into_any_element(),
        ));
        out
    }

    /// The small state buttons of an exception or of the add form.
    fn state_buttons(&self, kind: Permission, current: Option<PermissionState>, tag: &str, n: u64, cx: &mut Context<Self>, pick: impl Fn(&mut Self, PermissionState, &mut Context<Self>) + Clone + 'static) -> AnyElement {
        let row = site_row(kind);
        div()
            .flex()
            .gap(px(2.))
            .p(px(2.))
            .rounded(r(8.))
            .when(!ascii(), |d| d.bg(surface::key()))
            .children(states(kind).into_iter().enumerate().map(|(i, state)| {
                let selected = current == Some(state);
                let key = ("rule-state", n * 4 + i as u64);
                let h = self.motion.get(key, crate::motion::Feel::Hover);
                let pick = pick.clone();
                let el = div()
                    .id(("rule-state", n * 4 + i as u64))
                    .debug_selector(|| format!("{tag}:{row}:{}", state_label(state)))
                    .h(px(22.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(r(6.))
                    .text_size(px(11.5))
                    .when(selected, |d| d.bg(surface::raised()).shadow(vec![shadow(gpui::rgba(0x0000001F).into(), 1., 2.)]))
                    .when(!selected, |d| d.bg(fade(surface::raised().into(), 0.5 * h)))
                    .text_color(if selected { surface::ink() } else { surface::muted() })
                    .when(ascii() && selected, |d| d.text_color(self.theme.accent))
                    .on_click(cx.listener(move |this, _, _, cx| pick(this, state, cx)))
                    .child(state_label(state));
                self.feel(el, key, cx)
            }))
            .into_any_element()
    }

    pub(super) fn sites(&self, s: &Settings, cx: &mut Context<Self>) -> Vec<Item> {
        let mut out = vec![self.space_picker(cx)];
        for (n, kind) in Permission::SITE_KINDS.into_iter().enumerate() {
            let id = site_row(kind);
            let rules = self.rules.get(&kind).cloned().unwrap_or_default();
            let live = rules.iter().find(|(p, _)| p == "*").map(|(_, st)| *st);
            let default = s.web.site_defaults.iter().find(|(k, _)| *k == kind).map(|(_, st)| *st).or(live).unwrap_or(kind.chrome_default());
            let control: AnyElement = if kind == Permission::Cookies {
                div().text_size(px(12.)).text_color(surface::muted()).child("Default is set under Privacy").into_any_element()
            } else {
                let options = states(kind).into_iter().filter(|st| *st != PermissionState::SessionOnly).map(|st| (st, SharedString::from(state_label(st)))).collect();
                self.segments(
                    id,
                    options,
                    default,
                    move |v, s| {
                        s.web.site_defaults.retain(|(k, _)| *k != kind);
                        if v != kind.chrome_default() {
                            s.web.site_defaults.push((kind, v));
                        }
                    },
                    cx,
                )
            };
            out.push(self.row(id, "", control));
            for (m, (pattern, state)) in rules.iter().filter(|(p, _)| p != "*").enumerate() {
                let (p, p2) = (pattern.clone(), pattern.clone());
                let buttons = self.state_buttons(kind, Some(*state), "rule", n as u64 * 64 + m as u64, cx, move |this, st, cx| this.set_rule(kind, &p, Some(st), cx));
                let remove = div()
                    .id(("rule-remove", n as u64 * 64 + m as u64))
                    .debug_selector(|| format!("rule-remove:{id}:{p2}"))
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(r(6.))
                    .hover(|d| d.bg(surface::key()))
                    .on_click({
                        let p = pattern.clone();
                        cx.listener(move |this, _, _, cx| this.set_rule(kind, &p, None, cx))
                    })
                    .child(crate::icons::icon("x", 13., surface::icon().into()))
                    .into_any_element();
                out.push(self.sub_row(
                    format!("rule:{id}:{pattern}"),
                    SharedString::from(pattern.clone()).into_any_element(),
                    div().flex().items_center().gap(px(8.)).child(buttons).child(remove).into_any_element(),
                ));
            }
            if self.adding == Some(kind) {
                let pattern = self.text(Field::SitePattern).to_string();
                let buttons = self.state_buttons(kind, None, "add-state", n as u64 * 64 + 63, cx, move |this, st, cx| {
                    let pattern = pattern_of(this.text(Field::SitePattern));
                    if !pattern.is_empty() {
                        this.set_rule(kind, &pattern, Some(st), cx);
                        this.field_mut(Field::SitePattern).text.clear();
                        this.adding = None;
                    }
                });
                let _ = pattern;
                out.push(self.sub_row(
                    format!("add-form:{id}"),
                    self.field(Field::SitePattern, "site-pattern", "example.com", 220., cx),
                    buttons,
                ));
            } else {
                out.push(self.sub_row(
                    format!("add-rule-row:{id}"),
                    div().into_any_element(),
                    self.button(add_id(kind), "Add exception", false, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.adding = Some(kind);
                            this.active = Field::SitePattern;
                            cx.notify();
                        }))
                        .into_any_element(),
                ));
            }
        }
        out.push(self.row(
            "all_sites",
            "Sites that stored data, with how much.",
            self.button("all-sites", "Open", false, cx)
                .on_click(cx.listener(|this, _, _, cx| this.in_browser(cx, |b, w, cx| b.open_url("baa://settings/content/all", w, cx))))
                .into_any_element(),
        ));
        out
    }

    /// Writes one rule into the chosen Space's profile, or removes it.
    pub(crate) fn set_rule(&mut self, kind: Permission, pattern: &str, state: Option<PermissionState>, cx: &mut Context<Self>) {
        let Some(profile) = self.browser.read(cx).session.space(self.space).map(|s| s.profile) else { return };
        self.browser.read(cx).host.set_site_rule(profile, kind, pattern, state);
        self.rules = self.read_rules(cx);
        cx.notify();
    }

    pub(super) fn render_clear_sheet(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sheet = self.clear?;
        let space_name = self.browser.read(cx).session.space(self.space).map(|s| s.name.clone()).unwrap_or_default();
        let check = |id: &'static str, label: &'static str, on: bool, set: fn(&mut ClearSheet, bool), cx: &mut Context<Self>| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .py(px(5.))
                .child(div().text_size(px(13.)).child(label))
                .child(self.switch(id, on).on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(sheet) = &mut this.clear {
                        set(sheet, !on);
                    }
                    cx.notify();
                })))
        };
        let card = div()
            .id("clear-sheet")
            .debug_selector(|| "clear-sheet".to_string())
            .occlude()
            .w(px(440.))
            .p(px(20.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .rounded(r(14.))
            .bg(gpui::Rgba { a: 1., ..surface::bg() })
            .border_1()
            .border_color(surface::border())
            .shadow(vec![shadow(gpui::rgba(0x18122C52).into(), 30., 80.), shadow(gpui::rgba(0x18122C1F).into(), 8., 24.)])
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(div().text_size(px(16.)).font_weight(gpui::FontWeight::SEMIBOLD).child("Clear browsing data"))
            .child(self.segments_with(
                "clear-range",
                vec![(Range::Hour, "Last hour".into()), (Range::Day, "Last 24 hours".into()), (Range::Week, "Last 7 days".into()), (Range::Month, "Last 4 weeks".into()), (Range::All, "All time".into())],
                sheet.range,
                |this, v, cx| {
                    if let Some(sheet) = &mut this.clear {
                        sheet.range = v;
                    }
                    cx.notify();
                },
                cx,
            ))
            .child(check("clear-history", "Browsing history", sheet.history, |s, v| s.history = v, cx))
            .child(check("clear-downloads", "Download history", sheet.downloads, |s, v| s.downloads = v, cx))
            .child(check("clear-site-data", "Cookies and site data", sheet.site_data, |s, v| s.site_data = v, cx))
            .child(check("clear-cache", "Cached images and files", sheet.cache, |s, v| s.cache = v, cx))
            .child(self.segments_with(
                "clear-spaces",
                vec![(false, SharedString::from(format!("This Space ({space_name})"))), (true, "All Spaces".into())],
                sheet.all_spaces,
                |this, v, cx| {
                    if let Some(sheet) = &mut this.clear {
                        sheet.all_spaces = v;
                    }
                    cx.notify();
                },
                cx,
            ))
            .child(div().text_size(px(11.5)).text_color(surface::muted()).child("Incognito Spaces keep nothing, so they are left out."))
            .child(
                div()
                    .mt(px(6.))
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(self.button("clear-cancel", "Cancel", false, cx).on_click(cx.listener(|this, _, _, cx| {
                        this.clear = None;
                        cx.notify();
                    })))
                    .child(self.button("clear-confirm", "Clear", true, cx).on_click(cx.listener(|this, _, _, cx| this.run_clear(cx)))),
            );
        Some(
            div()
                .id("clear-scrim")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(mix(surface::scrim().into(), surface::scrim().into(), 1.))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.clear = None;
                    cx.notify();
                }))
                .child(card)
                .into_any_element(),
        )
    }

    pub(crate) fn run_clear(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.clear.take() else { return };
        let spaces: Vec<SpaceId> = if sheet.all_spaces {
            self.browser.read(cx).session.spaces().iter().map(|s| s.id).collect()
        } else {
            vec![self.space]
        };
        self.browser.update(cx, |b, cx| b.clear_browsing_data(&spaces, sheet, cx));
        cx.notify();
    }
}

fn add_id(kind: Permission) -> &'static str {
    match kind {
        Permission::Location => "add-rule:site.location",
        Permission::Camera => "add-rule:site.camera",
        Permission::Microphone => "add-rule:site.microphone",
        Permission::Notifications => "add-rule:site.notifications",
        Permission::Clipboard => "add-rule:site.clipboard",
        Permission::Midi => "add-rule:site.midi",
        Permission::Popups => "add-rule:site.popups",
        Permission::Sound => "add-rule:site.sound",
        Permission::JavaScript => "add-rule:site.javascript",
        Permission::Cookies => "add-rule:site.cookies",
        Permission::AutomaticDownloads => "add-rule:site.downloads",
        Permission::InsecureContent => "add-rule:site.insecure",
        Permission::Sensors => "add-rule:site.sensors",
        Permission::WindowManagement => "add-rule:site.windows",
        Permission::Other => "add-rule",
    }
}

impl<H: WebHost> Browser<H> {
    /// Clears Chrome's stores for the Spaces' profiles (incognito ones keep nothing) and baa's own records.
    pub(crate) fn clear_browsing_data(&mut self, spaces: &[SpaceId], sheet: ClearSheet, cx: &mut Context<Self>) {
        let since = sheet.range.duration().map(|d| SystemTime::now() - d);
        let what = ClearData { history: sheet.history, downloads: sheet.downloads, site_data: sheet.site_data, cache: sheet.cache };
        let profiles = self.profiles_of(spaces);
        for profile in profiles {
            self.host.clear_data(profile, since, what, cx, Box::new(|_, _| {}));
        }
        let secs = since.and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
        if let Some(store) = self.store() {
            let cleared = [(what.history, store.clear_history_since(secs)), (what.downloads, store.clear_downloads_since(secs))];
            for (_, result) in cleared.iter().filter(|(on, _)| *on) {
                if let Err(e) = result {
                    log::error!("clearing browsing data: {e}");
                }
            }
        }
        if what.downloads {
            self.saved_downloads = self.store().and_then(|s| s.downloads().ok()).unwrap_or_default();
        }
        self.show_toast("Browsing data cleared", cx);
    }

    /// The profiles of the stored (non-incognito) Spaces among `spaces`, each once.
    fn profiles_of(&self, spaces: &[SpaceId]) -> Vec<ProfileId> {
        let mut out = Vec::new();
        for space in spaces.iter().filter_map(|id| self.session.space(*id)).filter(|s| !s.ephemeral) {
            if !out.contains(&space.profile) {
                out.push(space.profile);
            }
        }
        out
    }

    /// Every setting back to its default; with `sites`, every Space's site exceptions go too.
    pub(crate) fn reset_settings(&mut self, sites: bool, cx: &mut Context<Self>) {
        let all: Vec<SpaceId> = self.session.spaces().iter().map(|s| s.id).collect();
        let changed: Vec<Permission> = self.settings.web.site_defaults.iter().map(|(k, _)| *k).collect();
        for profile in self.profiles_of(&all) {
            for kind in &changed {
                self.host.set_site_rule(profile, *kind, "*", None);
            }
            if sites {
                for kind in Permission::SITE_KINDS {
                    for (pattern, _) in self.host.site_rules(profile, kind).into_iter().filter(|(p, _)| p != "*") {
                        self.host.set_site_rule(profile, kind, &pattern, None);
                    }
                }
            }
        }
        self.settings = Settings::default();
        self.save_settings(cx);
        self.show_toast("Settings reset", cx);
    }
}
