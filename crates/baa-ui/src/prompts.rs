//! What a page waits on the user for (permission requests, `alert`/`confirm`/
//! `prompt`, "leave site?"), and the header's popovers: downloads, the
//! site's settings and extensions. All drawn in gpui over the page, like Arc, instead of
//! Chromium's app-modal panels.

use crate::theme::r;
use std::path::Path;

use baa_model::TabId;
use baa_web::{DialogKind, Download, DownloadState, PageEvent, PageHandle, Permission, PermissionState, WebHost};
use gpui::{div, prelude::*, px, relative, AnyElement, Context, Hsla, KeyDownEvent, SharedString, Window};

use crate::{
    browser::{drop_in, feel, host_of, shadow, Browser, HEADER_H, INSET},
    icons::icon,
    input::{Key, TextField},
    motion::Feel,
    theme::{fade, mix, surface, Theme},
};

#[derive(Default)]
pub(crate) struct Asks {
    /// Oldest first; the first is shown.
    pub permissions: Vec<PermissionAsk>,
    pub dialog: Option<DialogAsk>,
}

pub(crate) struct PermissionAsk {
    pub id: u64,
    pub origin: String,
    pub kinds: Vec<Permission>,
}

pub(crate) struct DialogAsk {
    pub kind: DialogKind,
    pub message: String,
    /// The prompt's answer; unused (but still taking Enter/Escape) otherwise.
    pub field: TextField,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Popover {
    Downloads,
    Site,
    Extensions,
}

/// The site permissions the site panel lists.
const SITE_PERMISSIONS: [Permission; 4] = [
    Permission::Camera,
    Permission::Microphone,
    Permission::Location,
    Permission::Notifications,
];

fn permission_icon(kind: Permission) -> &'static str {
    match kind {
        Permission::Camera => "camera",
        Permission::Microphone => "mic",
        Permission::ScreenShare => "maximize-2",
        Permission::Location => "map-pin",
        Permission::Notifications => "bell",
        Permission::Clipboard => "clipboard",
        Permission::Midi | Permission::Other => "sliders-horizontal",
    }
}

fn permission_name(kind: Permission) -> &'static str {
    match kind {
        Permission::Camera => "Camera",
        Permission::Microphone => "Microphone",
        Permission::ScreenShare => "Screen Sharing",
        Permission::Location => "Location",
        Permission::Notifications => "Notifications",
        Permission::Clipboard => "Clipboard",
        Permission::Midi => "MIDI Devices",
        Permission::Other => "Other",
    }
}

/// "example.com wants to use your camera and microphone".
pub(crate) fn permission_sentence(kinds: &[Permission]) -> String {
    let phrase = |k: &Permission| match k {
        Permission::Camera => "use your camera",
        Permission::Microphone => "use your microphone",
        Permission::ScreenShare => "share your screen",
        Permission::Location => "know your location",
        Permission::Notifications => "show notifications",
        Permission::Clipboard => "see what you copy",
        Permission::Midi => "use your MIDI devices",
        Permission::Other => "use more of your device",
    };
    // "use your camera and use your microphone" reads as "use your camera and microphone".
    if kinds == [Permission::Camera, Permission::Microphone] {
        return "use your camera and microphone".into();
    }
    let parts: Vec<&str> = kinds.iter().map(phrase).collect();
    match parts.as_slice() {
        [] => phrase(&Permission::Other).into(),
        [one] => (*one).into(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The host as a page's dialogs name it; files and data URLs have none.
fn site_name(url: &str) -> String {
    Some(host_of(url)).filter(|h| !h.is_empty()).unwrap_or_else(|| "This page".into())
}

/// "1.4 MB".
pub(crate) fn bytes(n: i64) -> String {
    let n = n.max(0) as f64;
    match n {
        n if n >= 1e9 => format!("{:.1} GB", n / 1e9),
        n if n >= 1e6 => format!("{:.1} MB", n / 1e6),
        n if n >= 1e3 => format!("{:.0} KB", n / 1e3),
        n => format!("{n:.0} bytes"),
    }
}

fn download_name(d: &Download) -> String {
    let from_path = Path::new(&d.path).file_name().map(|n| n.to_string_lossy().into_owned());
    from_path
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| d.url.rsplit('/').next().unwrap_or(&d.url).to_string())
}

fn download_status(d: &Download) -> String {
    match d.state {
        DownloadState::Complete => format!("{} · Done", bytes(d.total_bytes.max(d.received_bytes))),
        DownloadState::Canceled => "Cancelled".into(),
        DownloadState::InProgress if d.total_bytes > 0 => {
            format!("{} of {}", bytes(d.received_bytes), bytes(d.total_bytes))
        }
        DownloadState::InProgress => bytes(d.received_bytes),
    }
}

/// A plain or accent button on a light surface.
fn button<H: WebHost>(
    id: &'static str,
    label: &'static str,
    primary: bool,
    theme: &Theme,
    browser: &Browser<H>,
    cx: &mut Context<Browser<H>>,
) -> gpui::Stateful<gpui::Div> {
    let key = (id, 0);
    let (h, p) = (browser.motion.get(key, Feel::Hover), browser.motion.get(key, Feel::Press));
    let bg: Hsla = if primary {
        mix(theme.accent, gpui::black(), 0.12 * h)
    } else {
        mix(surface::key().into(), surface::line().into(), h)
    };
    let el = div()
        .id(id)
        .h(px(28.))
        .px(px(12.))
        .mx(px(1.5 * p))
        .flex()
        .items_center()
        .rounded(r(7.))
        .bg(bg)
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(if primary { gpui::white() } else { surface::ink().into() })
        .child(label);
    feel(el, key, cx)
}

impl<H: WebHost> Browser<H> {
    pub(crate) fn on_ask(&mut self, tab: TabId, event: &PageEvent, window: &mut Window, cx: &mut Context<Self>) {
        let asks = self.asks.entry(tab).or_default();
        match event {
            PageEvent::PermissionRequested { id, origin, kinds } => {
                asks.permissions.push(PermissionAsk {
                    id: *id,
                    origin: origin.clone(),
                    kinds: kinds.clone(),
                });
                self.motion.from(("ask-in", 0), Feel::Glide, 0.0, 1.0);
            }
            PageEvent::PermissionDismissed(id) => asks.permissions.retain(|a| a.id != *id),
            PageEvent::Dialog { kind, message, default_text } => {
                asks.dialog = Some(DialogAsk {
                    kind: *kind,
                    message: message.clone(),
                    field: TextField::selected(default_text.clone()),
                });
                if Some(tab) == self.active_tab() && self.bar.is_none() {
                    window.focus(&self.input_focus, cx);
                }
            }
            PageEvent::DialogClosed => {
                asks.dialog = None;
                if Some(tab) == self.active_tab() {
                    self.close_overlays(window, cx);
                }
            }
            PageEvent::AudibleChanged(on) => {
                if *on {
                    self.audible.insert(tab);
                } else {
                    self.audible.remove(&tab);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    pub(crate) fn active_dialog(&self) -> Option<&DialogAsk> {
        self.asks.get(&self.active_tab()?)?.dialog.as_ref()
    }

    pub(crate) fn on_dialog_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self
            .active_tab()
            .and_then(|id| self.asks.get_mut(&id))
            .and_then(|a| a.dialog.as_mut())
        else {
            return;
        };
        let prompt = dialog.kind == DialogKind::Prompt;
        let mut scratch = TextField::default();
        let field = if prompt { &mut dialog.field } else { &mut scratch };
        match field.handle(&event.keystroke, cx) {
            Key::Submit => self.answer_dialog(true, window, cx),
            Key::Cancel => self.answer_dialog(false, window, cx),
            _ => {}
        }
    }

    pub(crate) fn answer_dialog(&mut self, ok: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab() else { return };
        let Some(dialog) = self.asks.get_mut(&tab).and_then(|a| a.dialog.take()) else {
            return;
        };
        if let Some((page, _)) = self.pages.get(&tab) {
            page.read(cx).answer_dialog(ok, &dialog.field.text);
        }
        self.close_overlays(window, cx);
    }

    pub(crate) fn answer_permission(&mut self, allow: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab() else { return };
        let Some(asks) = self.asks.get_mut(&tab) else { return };
        if asks.permissions.is_empty() {
            return;
        }
        let ask = asks.permissions.remove(0);
        if let Some((page, _)) = self.pages.get(&tab) {
            page.read(cx).answer_permission(ask.id, allow);
        }
        cx.notify();
    }

    pub(crate) fn toggle_mute(&mut self, tab: TabId, cx: &mut Context<Self>) {
        let muted = !self.muted.contains(&tab);
        if muted {
            self.muted.insert(tab);
        } else {
            self.muted.remove(&tab);
        }
        if let Some((page, _)) = self.pages.get(&tab) {
            page.read(cx).set_muted(muted);
        }
        self.motion.from(("mute-pop", tab.0), Feel::Glide, 0.6, 1.0);
        cx.notify();
    }

    /// A new download makes the header's button hop.
    pub(crate) fn downloads_changed(&mut self, count: usize, cx: &mut Context<Self>) {
        if count > self.downloads_seen {
            self.motion.from(("download-hop", 0), Feel::Glide, 1.0, 0.0);
        }
        self.downloads_seen = count;
        cx.notify();
    }

    pub(crate) fn toggle_popover(&mut self, which: Popover, window: &mut Window, cx: &mut Context<Self>) {
        let open = self.popover != Some(which);
        self.close_overlays(window, cx);
        if open {
            self.popover = Some(which);
        }
        cx.notify();
    }

    fn visible_downloads(&self, cx: &gpui::App) -> Vec<Download> {
        let list = self.host.downloads();
        let items = list.read(cx).items.clone();
        items.into_iter().filter(|d| !self.cleared_downloads.contains(&d.id)).collect()
    }

    /// The header's downloads button, once something downloaded this session.
    pub(crate) fn render_download_button(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let items = self.visible_downloads(cx);
        if items.is_empty() {
            return None;
        }
        let running: Vec<&Download> = items.iter().filter(|d| d.state == DownloadState::InProgress).collect();
        let (got, total) = running
            .iter()
            .fold((0i64, 0i64), |(g, t), d| (g + d.received_bytes, t + d.total_bytes.max(0)));
        let progress = (!running.is_empty()).then(|| if total > 0 { got as f32 / total as f32 } else { 0.5 });
        let key = ("downloads", 0);
        let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
        let hop = self.motion.get(("download-hop", 0), Feel::Glide);
        let open = self.popover == Some(Popover::Downloads);
        let el = div()
            .id("downloads")
            .relative()
            .size(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(r(6.))
            .bg(fade(theme.hover, h.max(open as u8 as f32)))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.toggle_popover(Popover::Downloads, window, cx)))
            .child(
                icon("arrow-down-to-line", 15., mix(theme.ink_muted, theme.ink, h)).with_transformation(
                    gpui::Transformation::scale(gpui::size(1. - 0.15 * p, 1. - 0.15 * p))
                        .with_translation(gpui::point(px(0.), px(-5. * hop))),
                ),
            )
            .children(progress.map(|f| {
                div()
                    .absolute()
                    .bottom(px(3.))
                    .left(px(6.))
                    .w(px(14.))
                    .h(px(2.))
                    .rounded(r(1.))
                    .bg(theme.ink_faint)
                    .child(div().h_full().w(relative(f.clamp(0.05, 1.))).rounded(r(1.)).bg(theme.accent))
            }));
        Some(feel(el, key, cx).into_any_element())
    }

    /// The header's extensions button, Arc's: every extension, pinned or not.
    pub(crate) fn render_extensions_button(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let key = ("extensions", 0);
        let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
        let open = self.popover == Some(Popover::Extensions);
        let el = div()
            .id("extensions-button")
            .size(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(r(6.))
            .bg(fade(theme.hover, h.max(open as u8 as f32)))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.toggle_popover(Popover::Extensions, window, cx)))
            .child(icon("puzzle", 15. * (1. - 0.15 * p), mix(theme.ink_muted, theme.ink, h)));
        feel(el, key, cx).into_any_element()
    }

    /// Every extension as a tile: click opens it, the badge pins it to the
    /// toolbar, a right click has the rest.
    fn render_extensions_panel(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let unpinned = &self.settings.unpinned_extensions;
        let tile = |id: (&'static str, u64), h: f32| {
            div()
                .id(id)
                .relative()
                .h(px(60.))
                .min_w_0()
                .px(px(4.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .rounded(r(9.))
                .bg(mix(surface::key().into(), surface::line().into(), h))
        };
        let mut tiles: Vec<AnyElement> = self
            .extensions
            .iter()
            .enumerate()
            .map(|(i, ext)| {
                let key = ("ext-tile", i as u64);
                let h = self.motion.get(key, Feel::Hover);
                let pinned = !unpinned.contains(&ext.id);
                let (open, pin, menu) = (ext.clone(), ext.id.clone(), ext.id.clone());
                let badge = div()
                    .id(("ext-pin", i as u64))
                    .absolute()
                    .top(px(4.))
                    .right(px(4.))
                    .size(px(18.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(r(9.))
                    .when(pinned, |d| d.bg(surface::raised()).shadow(vec![shadow(gpui::rgba(0x0000002E).into(), 1., 3.)]))
                    .opacity(if pinned { 1. } else { h })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_extension_pin(&pin, cx);
                    }))
                    .child(icon(if pinned { "pin" } else { "pin-off" }, 10., if pinned { theme.accent } else { surface::icon().into() }));
                let el = tile(key, h)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_overlays(window, cx);
                        this.open_extension(&open, window, cx);
                    }))
                    .on_mouse_down(
                        gpui::MouseButton::Right,
                        cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                            this.open_menu(crate::browser::MenuKind::Extension(menu.clone()), e.position, window, cx)
                        }),
                    )
                    .child(match &ext.icon {
                        Some(path) => gpui::img(path.clone()).size(px(20.)).into_any_element(),
                        None => icon("puzzle", 18., surface::icon().into()).into_any_element(),
                    })
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_center()
                            .text_size(px(10.5))
                            .text_color(surface::muted())
                            .child(SharedString::from(ext.name.clone())),
                    )
                    .child(badge);
                feel(el, key, cx).into_any_element()
            })
            .collect();
        let key = ("ext-add", 0);
        let h = self.motion.get(key, Feel::Hover);
        let add = tile(key, h)
            .on_click(cx.listener(|this, _, window, cx| {
                this.close_overlays(window, cx);
                this.open_url("https://chromewebstore.google.com/", window, cx)
            }))
            .child(icon("plus", 18., surface::icon().into()))
            .child(div().text_size(px(10.5)).text_color(surface::muted()).child("Add"));
        tiles.push(feel(add, key, cx).into_any_element());
        drop_in(
            Self::popover_card("extensions-panel", 300.)
                .child(div().px(px(8.)).py(px(6.)).font_weight(gpui::FontWeight::SEMIBOLD).child("Extensions"))
                .child(div().px(px(2.)).grid().grid_cols(4).gap(px(6.)).children(tiles))
                .child(
                    div()
                        .mt(px(8.))
                        .pt(px(6.))
                        .px(px(4.))
                        .flex()
                        .border_t_1()
                        .border_color(surface::line())
                        .child(button("extensions-manage", "Manage Extensions", false, theme, self, cx).on_click(
                            cx.listener(|this, _, window, cx| this.open_extensions("chrome://extensions/", window, cx)),
                        )),
                ),
            "extensions-panel",
        )
        .into_any_element()
    }

    /// Lock (secure) or warning, at the right end of the address pill; opens
    /// the site's settings.
    pub(crate) fn render_site_button(&self, theme: &Theme, ink: Hsla, cx: &mut Context<Self>) -> Option<AnyElement> {
        let page = self.active_page()?;
        let url = page.read(cx).url();
        if !url.starts_with("http") {
            return None;
        }
        let secure = page.read(cx).is_secure();
        let key = ("site", 0);
        let h = self.motion.get(key, Feel::Hover);
        let el = div()
            .id("site-button")
            .size(px(18.))
            .ml(px(4.))
            .mr(px(-4.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(r(4.))
            .bg(fade(theme.hover, h))
            .on_click(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                this.toggle_popover(Popover::Site, window, cx)
            }))
            .child(icon(if secure { "lock" } else { "triangle-alert" }, 12., ink));
        Some(feel(el, key, cx).into_any_element())
    }

    /// Over the page: the first permission request as a banner under the top
    /// edge, and the page's dialog as a sheet with the page dimmed behind it.
    pub(crate) fn render_asks(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tab = self.active_tab()?;
        let asks = self.asks.get(&tab)?;
        let url = self.session.tab(tab).map(|t| t.url.clone()).unwrap_or_default();
        let mut layer = div().absolute().inset_0().flex().flex_col().items_center();
        let mut any = false;
        if let Some(dialog) = &asks.dialog {
            any = true;
            let (title, message, ok, cancel): (String, String, &'static str, Option<&'static str>) = match dialog.kind {
                DialogKind::BeforeUnload { is_reload } => (
                    if is_reload { "Reload site?" } else { "Leave site?" }.into(),
                    "Changes you made may not be saved.".into(),
                    if is_reload { "Reload" } else { "Leave" },
                    Some("Stay"),
                ),
                DialogKind::Alert => (format!("{} says", site_name(&url)), dialog.message.clone(), "OK", None),
                DialogKind::Confirm | DialogKind::Prompt => {
                    (format!("{} says", site_name(&url)), dialog.message.clone(), "OK", Some("Cancel"))
                }
            };
            let field = (dialog.kind == DialogKind::Prompt).then(|| {
                div()
                    .h(px(32.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(r(7.))
                    .border_1()
                    .border_color(theme.accent)
                    .bg(gpui::white())
                    .text_size(px(13.))
                    .child(dialog.field.render(
                        "",
                        surface::ink().into(),
                        surface::muted().into(),
                        Hsla { a: 0.25, ..theme.accent },
                    ))
            });
            let card = div()
                .id("page-dialog")
                .occlude()
                .mt(px(16.))
                .w(px(420.))
                .max_w(relative(0.9))
                .p(px(18.))
                .flex()
                .flex_col()
                .gap(px(12.))
                .rounded(r(12.))
                .bg(surface::bg())
                .border_1()
                .border_color(surface::border())
                .shadow(vec![
                    shadow(gpui::rgba(0x0A08143D).into(), 16., 40.),
                    shadow(gpui::rgba(0x0A08141F).into(), 2., 8.),
                ])
                .track_focus(&self.input_focus)
                .on_key_down(cx.listener(Self::on_input_key))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(surface::ink())
                        .child(SharedString::from(title)),
                )
                .when(!message.is_empty(), |d| {
                    d.child(
                        div()
                            .id("page-dialog-message")
                            .max_h(px(240.))
                            .overflow_y_scroll()
                            .text_size(px(13.))
                            .text_color(surface::ink())
                            .child(SharedString::from(message)),
                    )
                })
                .children(field)
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .children(cancel.map(|label| {
                            button("dialog-cancel", label, false, theme, self, cx)
                                .on_click(cx.listener(|this, _, window, cx| this.answer_dialog(false, window, cx)))
                        }))
                        .child(
                            button("dialog-ok", ok, true, theme, self, cx)
                                .on_click(cx.listener(|this, _, window, cx| this.answer_dialog(true, window, cx))),
                        ),
                );
            layer = layer
                .child(
                    div()
                        .id("page-dialog-scrim")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .rounded(r(crate::browser::PAGE_RADIUS))
                        .bg(surface::scrim()),
                )
                .child(drop_in(card, "page-dialog"));
        }
        if let Some(ask) = asks.permissions.first().filter(|_| asks.dialog.is_none()) {
            any = true;
            let t = self.motion.get(("ask-in", 0), Feel::Glide);
            let host = site_name(&ask.origin);
            let icons = ask.kinds.iter().map(|k| icon(permission_icon(*k), 16., theme.accent));
            let banner = div()
                .id("permission")
                .occlude()
                .mt(px(10. - 14. * (1. - t)))
                .opacity(t.clamp(0., 1.))
                .max_w(relative(0.94))
                .pl(px(14.))
                .pr(px(8.))
                .py(px(8.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(r(12.))
                .bg(surface::bg())
                .border_1()
                .border_color(surface::border())
                .shadow(vec![shadow(gpui::rgba(0x0A08142E).into(), 8., 24.)])
                .child(div().flex().gap(px(4.)).children(icons))
                .child(
                    div()
                        .min_w_0()
                        .text_size(px(13.))
                        .text_color(surface::ink())
                        .child(
                            div()
                                .flex()
                                .gap(px(4.))
                                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(SharedString::from(host)))
                                .child(SharedString::from(format!("wants to {}", permission_sentence(&ask.kinds)))),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(
                            button("perm-deny", "Don't Allow", false, theme, self, cx)
                                .on_click(cx.listener(|this, _, _, cx| this.answer_permission(false, cx))),
                        )
                        .child(
                            button("perm-allow", "Allow", true, theme, self, cx)
                                .on_click(cx.listener(|this, _, _, cx| this.answer_permission(true, cx))),
                        ),
                );
            layer = layer.child(banner);
        }
        any.then(|| layer.into_any_element())
    }

    /// The open header popover, if any, with a backdrop that closes it.
    pub(crate) fn render_popover(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let which = self.popover?;
        let card = match which {
            Popover::Downloads => self.render_downloads(theme, cx),
            Popover::Site => self.render_site_panel(cx)?,
            Popover::Extensions => self.render_extensions_panel(theme, cx),
        };
        let placed = match which {
            Popover::Downloads | Popover::Extensions => div().absolute().top(px(HEADER_H)).right(px(INSET)).child(card),
            Popover::Site => div()
                .absolute()
                .top(px(HEADER_H))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(card),
        };
        Some(
            gpui::deferred(
                div()
                    .absolute()
                    .inset_0()
                    .child(
                        div()
                            .id("popover-backdrop")
                            .absolute()
                            .inset_0()
                            .occlude()
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(|this, _, window, cx| this.close_overlays(window, cx)),
                            ),
                    )
                    .child(placed),
            )
            .into_any_element(),
        )
    }

    fn popover_card(id: &'static str, width: f32) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .occlude()
            .w(px(width))
            .p(px(6.))
            .flex()
            .flex_col()
            .rounded(r(12.))
            .bg(surface::bg())
            .border_1()
            .border_color(surface::border())
            .shadow(vec![shadow(gpui::rgba(0x18122C29).into(), 8., 24.)])
            .text_size(px(13.))
            .text_color(surface::ink())
    }

    fn render_downloads(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let items = self.visible_downloads(cx);
        let rows: Vec<AnyElement> = items
            .iter()
            .rev()
            .map(|d| {
                let id = d.id;
                let key = ("download-row", id as u64);
                let h = self.motion.get(key, Feel::Hover);
                let path = std::path::PathBuf::from(&d.path);
                let done = d.state == DownloadState::Complete;
                let running = d.state == DownloadState::InProgress;
                let fraction = (d.total_bytes > 0).then(|| d.received_bytes as f32 / d.total_bytes as f32);
                let open_path = path.clone();
                let trailing = if running {
                    div()
                        .id(("download-cancel", id as u64))
                        .size(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(r(6.))
                        .hover(|s| s.bg(surface::line()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.host.cancel_download(id);
                        }))
                        .child(icon("x", 13., surface::icon().into()))
                        .into_any_element()
                } else if done {
                    let reveal = path.clone();
                    div()
                        .id(("download-reveal", id as u64))
                        .size(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(r(6.))
                        .hover(|s| s.bg(surface::line()))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            cx.reveal_path(&reveal);
                        }))
                        .child(icon("folder-open", 13., surface::icon().into()))
                        .into_any_element()
                } else {
                    div().into_any_element()
                };
                let row = div()
                    .id(("download", id as u64))
                    .h(px(46.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(r(8.))
                    .bg(fade(surface::key().into(), h))
                    .when(done, |d| {
                        d.on_click(cx.listener(move |_, _, _, cx| cx.open_with_system(&open_path)))
                    })
                    .child(icon("file", 18., if done { theme.accent } else { surface::icon().into() }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(div().truncate().font_weight(gpui::FontWeight::MEDIUM).child(SharedString::from(download_name(d))))
                            .child(match (running, fraction) {
                                (true, Some(f)) => div()
                                    .h(px(4.))
                                    .rounded(r(2.))
                                    .bg(surface::line())
                                    .child(div().h_full().w(relative(f.clamp(0.02, 1.))).rounded(r(2.)).bg(theme.accent))
                                    .into_any_element(),
                                _ => div()
                                    .text_size(px(11.5))
                                    .text_color(surface::muted())
                                    .child(SharedString::from(download_status(d)))
                                    .into_any_element(),
                            }),
                    )
                    .child(trailing);
                feel(row, key, cx).into_any_element()
            })
            .collect();
        let empty = rows.is_empty();
        let footer = div()
            .mt(px(4.))
            .pt(px(6.))
            .px(px(4.))
            .flex()
            .justify_between()
            .border_t_1()
            .border_color(surface::line())
            .child(
                button("downloads-all", "Show All", false, theme, self, cx).on_click(cx.listener(
                    |this, _, window, cx| {
                        this.close_overlays(window, cx);
                        this.open_library(crate::library::Section::Downloads, cx);
                    },
                )),
            )
            .child(button("downloads-clear", "Clear", false, theme, self, cx).on_click(cx.listener(
                |this, _, window, cx| {
                    let done: Vec<u32> = this
                        .visible_downloads(cx)
                        .iter()
                        .filter(|d| d.state != DownloadState::InProgress)
                        .map(|d| d.id)
                        .collect();
                    this.cleared_downloads.extend(done);
                    if this.visible_downloads(cx).is_empty() {
                        this.close_overlays(window, cx);
                    }
                    cx.notify();
                },
            )));
        drop_in(
            Self::popover_card("downloads-list", 320.)
                .child(div().px(px(8.)).py(px(6.)).font_weight(gpui::FontWeight::SEMIBOLD).child("Downloads"))
                .children(rows)
                .when(empty, |d| d.child(div().px(px(8.)).py(px(10.)).text_color(surface::muted()).child("Nothing yet")))
                .child(footer),
            "downloads-list",
        )
        .into_any_element()
    }

    fn render_site_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let page = self.active_page()?;
        let url = page.read(cx).url();
        let secure = page.read(cx).is_secure();
        let profile = self.active_space()?.profile;
        let rows: Vec<AnyElement> = SITE_PERMISSIONS
            .iter()
            .map(|kind| {
                let kind = *kind;
                let state = self.host.site_permission(profile, &url, kind);
                let options = [
                    (PermissionState::Ask, "Ask"),
                    (PermissionState::Allow, "Allow"),
                    (PermissionState::Block, "Block"),
                ];
                let segments = options.into_iter().map(|(value, label)| {
                    let on = value == state;
                    let url = url.clone();
                    div()
                        .id(SharedString::from(format!("site-{kind:?}-{label}")))
                        .h(px(22.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .rounded(r(5.))
                        .text_size(px(11.5))
                        .when(on, |d| d.bg(crate::theme::surface::raised()).shadow(vec![shadow(gpui::rgba(0x0000001F).into(), 1., 2.)]))
                        .text_color(if on { surface::ink() } else { surface::muted() })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.host.set_site_permission(profile, &url, kind, value);
                            cx.notify();
                        }))
                        .child(label)
                });
                div()
                    .h(px(36.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(icon(permission_icon(kind), 15., surface::icon().into()))
                    .child(div().flex_1().child(permission_name(kind)))
                    .child(div().p(px(2.)).flex().rounded(r(7.)).bg(surface::key()).children(segments))
                    .into_any_element()
            })
            .collect();
        Some(
            drop_in(
                Self::popover_card("site-panel", 340.)
                    .child(
                        div()
                            .px(px(8.))
                            .pt(px(6.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_size(px(14.))
                            .child(SharedString::from(host_of(&url))),
                    )
                    .child(
                        div()
                            .px(px(8.))
                            .pt(px(4.))
                            .pb(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(12.))
                            .text_color(surface::muted())
                            .child(icon(
                                if secure { "lock" } else { "triangle-alert" },
                                12.,
                                if secure { surface::muted().into() } else { gpui::rgb(0xC2410C).into() },
                            ))
                            .child(if secure { "Connection is secure" } else { "Connection is not secure" }),
                    )
                    .child(div().mx(px(8.)).h(px(1.)).bg(surface::line()))
                    .children(rows),
                "site-panel",
            )
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_requests_read_as_a_sentence() {
        use Permission::*;
        assert_eq!(permission_sentence(&[Camera, Microphone]), "use your camera and microphone");
        assert_eq!(permission_sentence(&[Location]), "know your location");
        assert_eq!(
            permission_sentence(&[Location, Notifications, Clipboard]),
            "know your location, show notifications and see what you copy"
        );
    }

    #[test]
    fn sizes_read_like_finder() {
        assert_eq!(bytes(512), "512 bytes");
        assert_eq!(bytes(1_400_000), "1.4 MB");
        assert_eq!(bytes(-1), "0 bytes");
    }
}
