//! Chrome's own pages in baa. `chrome://extensions` is drawn here and carried
//! out by a hidden Chrome page (baa-web), since baa's pages cannot run
//! Chrome's. The rest of `chrome://settings` is Chrome's page itself in a
//! sheet, restyled to baa's look; history and downloads go to baa's own.

use crate::theme::r;

use baa_model::SpaceId;
use baa_web::{ExtensionChange, ExtensionInfo, SiteAccess, WebHost};
use gpui::{div, prelude::*, px, AnyElement, Context, ElementId, SharedString, Window};

use crate::{
    browser::{feel, Browser},
    icons::icon,
    motion::Feel,
    theme::{fade, mix, surface, Theme},
};

/// The address of baa's extensions page.
pub(crate) const EXTENSIONS_URL: &str = "baa://extensions";

/// What the extensions page shows.
#[derive(Default)]
pub(crate) struct ExtensionsPage {
    /// `None` until read, or when reading failed.
    pub list: Option<Vec<ExtensionInfo>>,
    pub loading: bool,
    /// The Space whose extensions `list` holds.
    pub space: Option<SpaceId>,
    /// The extension whose details are open.
    pub open: Option<String>,
    /// The extension Remove was pressed on once.
    pub confirm_remove: Option<String>,
}

/// Where a Chrome page goes in baa.
#[derive(Debug, PartialEq)]
pub(crate) enum OwnPage {
    /// With an extension to open the details of.
    Extensions(Option<String>),
    /// Chrome's settings page at this address, in a sheet.
    ChromeSettings(String),
    /// baa's own settings.
    Settings,
    History,
    Downloads,
}

pub(crate) fn own_page(url: &str) -> Option<OwnPage> {
    let rest = url
        .strip_prefix("chrome://")
        .or_else(|| url.strip_prefix("baa://"))?;
    let (page, query) = rest.split_once('?').unwrap_or((rest, ""));
    let page = page.split('/').next().unwrap_or_default();
    Some(match page {
        "extensions" => OwnPage::Extensions(
            query
                .split('&')
                .find_map(|kv| kv.strip_prefix("id="))
                .filter(|id| !id.is_empty())
                .map(String::from),
        ),
        "settings" if url.starts_with("chrome://") => OwnPage::ChromeSettings(url.into()),
        "settings" => OwnPage::Settings,
        "history" => OwnPage::History,
        "downloads" => OwnPage::Downloads,
        _ => return None,
    })
}

/// Pages baa draws itself; no web page is made for them.
pub(crate) fn is_own(url: &str) -> bool {
    url.starts_with("baa://")
}

impl<H: WebHost> Browser<H> {
    /// Opens Chrome's page at `url` as baa's own; false when baa has none.
    pub(crate) fn open_own_page(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(page) = own_page(url) else { return false };
        self.close_overlays(window, cx);
        match page {
            OwnPage::Extensions(id) => {
                self.extensions_page.open = id;
                let space = self.session.active_space();
                let existing = space.and_then(|space| self.own_tab(space));
                match existing {
                    Some(tab) => {
                        self.session.activate_tab(tab, baa_model::now());
                        self.sync(window, cx);
                    }
                    None => {
                        if let Some(space) = space {
                            self.session.set_active_space(space);
                            let tab = self.session.open_tab(space, EXTENSIONS_URL, baa_model::now());
                            if let Some(tab) = self.session.tab_mut(tab) {
                                tab.title = "Extensions".into();
                            }
                            self.sync(window, cx);
                        }
                    }
                }
                self.refresh_extensions(cx);
            }
            OwnPage::ChromeSettings(url) => {
                self.chrome_sheet = Some(url);
                cx.notify();
            }
            OwnPage::Settings => self.open_settings(window, cx),
            OwnPage::History => self.open_bar(crate::command_bar::Mode::History, window, cx),
            OwnPage::Downloads => self.open_library(crate::library::Section::Downloads, cx),
        }
        true
    }

    fn own_tab(&self, space: SpaceId) -> Option<baa_model::TabId> {
        [baa_model::Zone::Pinned, baa_model::Zone::Today]
            .into_iter()
            .flat_map(|zone| self.session.tabs(space, zone))
            .find(|t| t.url.starts_with(EXTENSIONS_URL))
            .map(|t| t.id)
    }

    /// Re-reads the active Space's extensions.
    pub(crate) fn refresh_extensions(&mut self, cx: &mut Context<Self>) {
        let Some(space) = self.active_space() else { return };
        let (id, profile) = (space.id, space.profile);
        if self.extensions_page.space != Some(id) {
            self.extensions_page.list = None;
        }
        self.extensions_page.space = Some(id);
        self.extensions_page.loading = true;
        let this = cx.weak_entity();
        self.host.extension_details(
            profile,
            cx,
            // Deferred: the answer may come straight back, inside this update.
            Box::new(move |list, cx| {
                cx.defer(move |cx| {
                    this.update(cx, |this, cx| {
                        if this.extensions_page.space == Some(id) {
                            this.extensions_page.list = list;
                            this.extensions_page.loading = false;
                        }
                        if let Some(space) = this.active_space() {
                            this.extensions = this.host.extensions(space.profile);
                        }
                        cx.notify();
                    })
                    .ok();
                })
            }),
        );
        cx.notify();
    }

    /// Shown at once; then Chrome's answer replaces it.
    pub(crate) fn change_extension(&mut self, id: &str, change: ExtensionChange, cx: &mut Context<Self>) {
        let Some(profile) = self.active_space().map(|s| s.profile) else { return };
        if let Some(list) = &mut self.extensions_page.list {
            match change {
                ExtensionChange::Remove => list.retain(|e| e.id != id),
                ExtensionChange::Enable(on) => list.iter_mut().filter(|e| e.id == id).for_each(|e| e.enabled = on),
                ExtensionChange::SiteAccess(a) => {
                    list.iter_mut().filter(|e| e.id == id).for_each(|e| e.site_access = Some(a))
                }
            }
        }
        if change == ExtensionChange::Remove {
            self.extensions.retain(|e| e.id != id);
        }
        let this = cx.weak_entity();
        self.host.change_extension(
            profile,
            id,
            change,
            cx,
            Box::new(move |_, cx| {
                cx.defer(move |cx| {
                    this.update(cx, |this, cx| this.refresh_extensions(cx)).ok();
                })
            }),
        );
        cx.notify();
    }

    /// Whether the toolbar shows `id`: not when Chrome has it switched off.
    pub(crate) fn extension_enabled(&self, id: &str) -> bool {
        self.extensions_page
            .list
            .as_ref()
            .is_none_or(|list| list.iter().find(|e| e.id == id).is_none_or(|e| e.enabled))
    }

    pub(crate) fn render_extensions_page(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let page = &self.extensions_page;
        let body: Vec<AnyElement> = match &page.list {
            Some(list) if list.is_empty() => vec![note("No extensions yet. Find some in the Chrome Web Store.")],
            Some(list) => list.iter().enumerate().map(|(i, e)| self.render_extension(i, e, theme, cx)).collect(),
            None if page.loading => vec![note("Loading…")],
            None => vec![note("Couldn't read your extensions.")],
        };
        div()
            .id("extensions-page")
            .size_full()
            .overflow_y_scroll()
            .bg(surface::bg())
            .text_color(surface::ink())
            .child(
                div()
                    .mx_auto()
                    .max_w(px(720.))
                    .px(px(32.))
                    .py(px(36.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .mb(px(8.))
                            .flex()
                            .items_center()
                            .child(div().flex_1().text_size(px(22.)).font_weight(gpui::FontWeight::SEMIBOLD).child("Extensions"))
                            .child(self.pill(("ext-store", 0), "Chrome Web Store".into(), false, theme, cx).on_click(cx.listener(
                                |this, _, window, cx| this.open_url("https://chromewebstore.google.com/", window, cx),
                            ))),
                    )
                    .children(body),
            )
            .into_any_element()
    }

    fn render_extension(&self, i: usize, e: &ExtensionInfo, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let page = &self.extensions_page;
        let open = page.open.as_deref() == Some(e.id.as_str());
        let confirming = page.confirm_remove.as_deref() == Some(e.id.as_str());
        let pinned = !self.settings.unpinned_extensions.contains(&e.id);
        let n = i as u64;
        let icon_path = self.extensions.iter().find(|x| x.id == e.id).and_then(|x| x.icon.clone());
        let (id, on) = (e.id.clone(), e.enabled);
        let switch = self.switch(("ext-on", n), on, theme).on_click(cx.listener(move |this, _, _, cx| {
            this.change_extension(&id, ExtensionChange::Enable(!on), cx)
        }));
        let mut actions: Vec<AnyElement> = Vec::new();
        let id = e.id.clone();
        actions.push(
            self.pill(("ext-details", n), if open { "Hide Details" } else { "Details" }.into(), false, theme, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let page = &mut this.extensions_page;
                    page.open = (page.open.as_deref() != Some(id.as_str())).then(|| id.clone());
                    cx.notify();
                }))
                .into_any_element(),
        );
        if let Some(options) = e.options.clone().filter(|_| e.enabled) {
            actions.push(
                self.pill(("ext-options", n), "Options".into(), false, theme, cx)
                    .on_click(cx.listener(move |this, _, window, cx| this.open_url(&options, window, cx)))
                    .into_any_element(),
            );
        }
        let id = e.id.clone();
        actions.push(
            self.pill(("ext-pin", n), if pinned { "Unpin from Toolbar" } else { "Pin to Toolbar" }.into(), false, theme, cx)
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_extension_pin(&id, cx)))
                .into_any_element(),
        );
        actions.push(div().flex_1().into_any_element());
        let id = e.id.clone();
        if confirming {
            actions.push(
                self.pill(("ext-keep", n), "Cancel".into(), false, theme, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.extensions_page.confirm_remove = None;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        actions.push(
            self.pill(("ext-remove", n), if confirming { "Remove for Good" } else { "Remove" }.into(), confirming, theme, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.extensions_page.confirm_remove.as_deref() == Some(id.as_str()) {
                        this.extensions_page.confirm_remove = None;
                        this.change_extension(&id, ExtensionChange::Remove, cx);
                    } else {
                        this.extensions_page.confirm_remove = Some(id.clone());
                        cx.notify();
                    }
                }))
                .into_any_element(),
        );
        let details = open.then(|| self.render_details(n, e, theme, cx));
        div()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .rounded(r(12.))
            .border_1()
            .border_color(surface::line())
            .child(
                div()
                    .flex()
                    .gap(px(14.))
                    .child(
                        div()
                            .size(px(36.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .opacity(if e.enabled { 1. } else { 0.45 })
                            .child(match icon_path {
                                Some(path) => gpui::img(path).size(px(30.)).into_any_element(),
                                None => icon("puzzle", 24., surface::icon().into()).into_any_element(),
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap(px(8.))
                                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(SharedString::from(e.name.clone())))
                                    .child(div().text_size(px(12.)).text_color(surface::muted()).child(SharedString::from(e.version.clone()))),
                            )
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .text_color(surface::muted())
                                    .line_clamp(2)
                                    .child(SharedString::from(e.description.clone())),
                            ),
                    )
                    .child(switch),
            )
            .child(div().flex().flex_wrap().gap(px(6.)).children(actions))
            .children(details)
            .into_any_element()
    }

    fn render_details(&self, n: u64, e: &ExtensionInfo, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let heading = |text: &'static str| {
            div().text_size(px(12.)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(surface::muted()).child(text)
        };
        let permissions: Vec<AnyElement> = if e.permissions.is_empty() {
            vec![div().text_size(px(13.)).child("No special permissions").into_any_element()]
        } else {
            e.permissions
                .iter()
                .map(|p| div().text_size(px(13.)).child(SharedString::from(format!("•  {p}"))).into_any_element())
                .collect()
        };
        let access = e.site_access.map(|current| {
            let mut options = vec![(SiteAccess::OnClick, "On Click"), (SiteAccess::OnAllSites, "On All Sites")];
            // Chosen sites are set in Chrome's page; shown only when already in use.
            if current == SiteAccess::OnSpecificSites {
                options.insert(1, (SiteAccess::OnSpecificSites, "On Specific Sites"));
            }
            let segments = options.into_iter().enumerate().map(|(k, (value, label))| {
                let id = e.id.clone();
                self.pill(("ext-access", n * 4 + k as u64), label.into(), value == current, theme, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change_extension(&id, ExtensionChange::SiteAccess(value), cx)
                    }))
            });
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(heading("Site Access"))
                .child(div().flex().gap(px(6.)).children(segments))
        });
        div()
            .pt(px(12.))
            .border_t_1()
            .border_color(surface::line())
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(div().flex().flex_col().gap(px(6.)).child(heading("Permissions")).children(permissions))
            .children(access)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(heading("ID"))
                    .child(div().text_size(px(12.)).font_family(crate::theme::MONO).child(SharedString::from(e.id.clone()))),
            )
            .into_any_element()
    }

    /// A small button; `strong` fills it with the accent.
    fn pill(
        &self,
        id: (&'static str, u64),
        label: SharedString,
        strong: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let (h, p) = (self.motion.get(id, Feel::Hover), self.motion.get(id, Feel::Press));
        let el = div()
            .id(ElementId::NamedInteger(id.0.into(), id.1))
            .h(px(26.))
            .px(px(10.))
            .mx(px(1.5 * p))
            .flex()
            .items_center()
            .rounded(r(7.))
            .text_size(px(12.5))
            .font_weight(gpui::FontWeight::MEDIUM)
            .bg(if strong {
                mix(theme.accent, gpui::black(), 0.12 * h)
            } else {
                mix(surface::key().into(), surface::line().into(), h)
            })
            .text_color(if strong { gpui::white() } else { surface::ink().into() })
            .child(label);
        feel(el, id, cx)
    }

    fn switch(&self, id: (&'static str, u64), on: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
        let base = div().id(ElementId::NamedInteger(id.0.into(), id.1)).flex_none();
        if crate::theme::ascii() {
            return base
                .text_color(if on { theme.accent } else { surface::icon().into() })
                .child(if on { "[x]" } else { "[ ]" });
        }
        base.w(px(36.))
            .h(px(22.))
            .p(px(2.))
            .rounded(r(11.))
            .bg(if on { theme.accent } else { fade(surface::line().into(), 1.) })
            .child(
                div()
                    .size(px(18.))
                    .when(on, |d| d.ml(px(14.)))
                    .rounded(r(9.))
                    .bg(gpui::white())
                    .shadow(vec![crate::browser::shadow(gpui::rgba(0x00000033).into(), 1., 3.)]),
            )
    }
}

/// The Chrome settings sheet: a scrim, a card with baa's header, and Chrome's
/// page put over the card's body (it is a window of its own, so it is placed,
/// not drawn).
impl<H: WebHost> Browser<H> {
    pub(crate) fn render_chrome_sheet(&self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let profile = self.active_space().map(|s| s.profile);
        let Some(url) = self.chrome_sheet.clone() else {
            if let Some(profile) = profile {
                self.host.show_chrome_page(profile, None, window);
            }
            return None;
        };
        let profile = profile?;
        let view = window.viewport_size();
        let (vw, vh) = (f32::from(view.width), f32::from(view.height));
        let top = crate::browser::HEADER_H;
        let (w, h) = ((vw - 96.).min(1040.), (vh - top - 40.).min(780.));
        let (x, y) = ((vw - w) / 2., top + ((vh - top - h) / 2.).max(8.));
        const BAR: f32 = 44.;
        let inset = 6.;
        let body = gpui::Bounds::new(
            gpui::point(px(x + inset), px(y + BAR)),
            gpui::size(px(w - inset * 2.), px(h - BAR - inset)),
        );
        let sheet = baa_web::ChromeSheet {
            url,
            bounds: body,
            corner_radius: f32::from(r(10.)),
            css: chrome_skin(theme),
            dark: crate::theme::ascii(),
        };
        if !self.host.show_chrome_page(profile, Some(sheet), window) {
            // Not up yet: look again shortly.
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                this.update(cx, |_, cx| cx.notify()).ok();
            })
            .detach();
        }
        let ascii = crate::theme::ascii();
        let key = ("chrome-sheet-close", 0);
        let ch = self.motion.get(key, Feel::Hover);
        let close = feel(
            div()
                .id("chrome-sheet-close")
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(r(7.))
                .bg(fade(surface::line().into(), ch))
                .on_click(cx.listener(|this, _, window, cx| this.close_overlays(window, cx)))
                .child(icon("x", 14., surface::icon().into())),
            key,
            cx,
        );
        let card = div()
            .id("chrome-sheet")
            .occlude()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(w))
            .h(px(h))
            .rounded(r(14.))
            .bg(surface::bg())
            .when(!ascii, |d| d.border_1().border_color(surface::border()))
            .shadow(vec![crate::browser::shadow(gpui::rgba(0x0A08143D).into(), 16., 48.)])
            .when(ascii, |d| d.child(crate::theme::ascii::outline(14.)))
            .child(
                div()
                    .h(px(BAR))
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(icon("settings", 15., surface::icon().into()))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(13.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(surface::ink())
                            .child(if ascii { "" } else { "Chrome Settings" }),
                    )
                    .child(close),
            )
            .when(ascii, |d| d.child(crate::theme::ascii::box_title("CHROME SETTINGS", theme.accent)));
        Some(
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .id("chrome-sheet-scrim")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .bg(surface::scrim())
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, window, cx| this.close_overlays(window, cx))),
                )
                .child(card)
                .into_any_element(),
        )
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
    css
}

fn note(text: &'static str) -> AnyElement {
    div().py(px(24.)).text_color(surface::muted()).child(text).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_pages_map_to_baas_own() {
        assert_eq!(own_page("chrome://extensions/"), Some(OwnPage::Extensions(None)));
        assert_eq!(own_page("chrome://extensions/?id=abc"), Some(OwnPage::Extensions(Some("abc".into()))));
        assert_eq!(own_page("chrome://extensions/shortcuts"), Some(OwnPage::Extensions(None)));
        assert_eq!(own_page("baa://extensions"), Some(OwnPage::Extensions(None)));
        assert_eq!(
            own_page("chrome://settings/content/cookies"),
            Some(OwnPage::ChromeSettings("chrome://settings/content/cookies".into()))
        );
        assert_eq!(own_page("baa://settings"), Some(OwnPage::Settings));
        assert_eq!(own_page("chrome://history"), Some(OwnPage::History));
        assert_eq!(own_page("chrome://downloads/"), Some(OwnPage::Downloads));
        assert_eq!(own_page("chrome://flags"), None);
        assert_eq!(own_page("https://example.com/extensions"), None);
    }
}
