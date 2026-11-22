//! What a page waits on the user for (permission requests, `alert`/`confirm`/
//! `prompt`, "leave site?"), and the header's popovers: downloads, the
//! site's settings and extensions. All drawn in gpui over the page, like Arc, instead of
//! Chromium's app-modal panels.

use crate::theme::r;
use std::{
    path::Path,
    time::{Duration, Instant},
};

mod banner;
mod dialog;
pub(crate) use banner::{ask_layer, page_asks};
pub(crate) use dialog::DialogAsk;
#[cfg(test)]
pub(crate) use dialog::wording;

use baa_model::TabId;
use baa_web::{DangerKind, Download, DownloadState, PageEvent, PageHandle, Permission, PermissionState, WebHost};
use gpui::{div, prelude::*, px, relative, AnyElement, Context, Div, Entity, Hsla, KeyDownEvent, SharedString, Stateful, Window};

use crate::{
    browser::{cut_left, drop_in, feel, host_of, shadow, Browser, Shown, Tip, HEADER_H, INSET},
    icons::icon,
    command_bar::Command,
    input::Key,
    motion::Feel,
    theme::{fade, mix, surface, Theme},
};

/// How long a prompt ignores OK/Allow after it appears, so a click or Enter aimed
/// at what was there before cannot land on it.
pub(crate) const ACTIVATION_DELAY: Duration = Duration::from_millis(500);

#[derive(Default)]
pub(crate) struct Asks {
    /// Oldest first; the first is shown.
    pub permissions: Vec<PermissionAsk>,
    pub dialog: Option<DialogAsk>,
}

impl Asks {
    /// The dialog or the first permission went away: the next permission shows
    /// from now on.
    fn reveal(&mut self, now: Instant) {
        if let Some(next) = self.permissions.first_mut() {
            next.shown_at = now;
        }
    }

    /// Shown somewhere new: a click there waits out [`ACTIVATION_DELAY`] again.
    pub(crate) fn moved(&mut self, now: Instant) {
        if let Some(dialog) = &mut self.dialog {
            dialog.shown_at = now;
        }
        self.reveal(now);
    }

    pub(crate) fn any(&self) -> bool {
        self.dialog.is_some() || !self.permissions.is_empty()
    }

    /// True when the dialog came or went.
    pub(crate) fn on_event(&mut self, event: &PageEvent, now: Instant) -> bool {
        match event {
            PageEvent::PermissionRequested { id, origin, kinds } => {
                self.permissions.push(PermissionAsk { id: *id, origin: origin.clone(), kinds: kinds.clone(), shown_at: now });
                false
            }
            PageEvent::Dialog(d) => {
                self.dialog = Some(DialogAsk::new(d, now));
                true
            }
            PageEvent::AskClosed(id) => {
                if self.permissions.first().is_some_and(|a| a.id == *id) {
                    self.permissions.remove(0);
                    self.reveal(now);
                }
                self.permissions.retain(|a| a.id != *id);
                let dialog = self.dialog.as_ref().is_some_and(|a| a.dialog.id == *id);
                if dialog {
                    self.dialog = None;
                    self.reveal(now);
                }
                dialog
            }
            _ => false,
        }
    }

    /// The dialog, once it may be answered: OK waits out [`ACTIVATION_DELAY`].
    pub(crate) fn take_dialog(&mut self, ok: bool, now: Instant) -> Option<DialogAsk> {
        if ok && self.dialog.as_ref().is_some_and(|a| now < a.shown_at + ACTIVATION_DELAY) {
            return None;
        }
        let ask = self.dialog.take()?;
        self.reveal(now);
        Some(ask)
    }

    /// The first permission's id, once it may be answered: Allow waits out [`ACTIVATION_DELAY`].
    pub(crate) fn take_permission(&mut self, allow: bool, now: Instant) -> Option<u64> {
        let first = self.permissions.first()?;
        if allow && now < first.shown_at + ACTIVATION_DELAY {
            return None;
        }
        let ask = self.permissions.remove(0);
        self.reveal(now);
        Some(ask.id)
    }
}

pub(crate) trait Prompter: Sized + 'static {
    type Page: PageHandle;
    fn shown_asks_mut(&mut self) -> Option<&mut Asks>;
    fn prompt_page(&self) -> Option<Entity<Self::Page>>;
    fn answered(&mut self, ask: &DialogAsk, ok: bool, window: &mut Window, cx: &mut Context<Self>);
    fn button(&self, key: (&'static str, u64), label: SharedString, primary: bool, theme: &Theme, cx: &mut Context<Self>) -> Stateful<Div>;

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog_key(event, window, cx).stop_unless_typed(cx);
        cx.notify();
    }

    fn dialog_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> Key {
        let Some(ask) = self.shown_asks_mut().and_then(|a| a.dialog.as_mut()) else { return Key::Ignored };
        let (key, answer) = ask.on_key(event, cx);
        if let Some(ok) = answer {
            self.answer(ok, window, cx);
        }
        key
    }

    fn answer(&mut self, ok: bool, window: &mut Window, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let Some(ask) = self.shown_asks_mut().and_then(|a| a.take_dialog(ok, now)) else { return };
        if let Some(page) = self.prompt_page() {
            page.read(cx).answer_dialog(ask.dialog.id, ask.answer(ok));
        }
        self.answered(&ask, ok, window, cx);
    }

    fn allow(&mut self, allow: bool, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let Some(id) = self.shown_asks_mut().and_then(|a| a.take_permission(allow, now)) else { return };
        if let Some(page) = self.prompt_page() {
            page.read(cx).answer_permission(id, allow);
        }
        cx.notify();
    }

    fn toggle_check(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.shown_asks_mut().and_then(|a| a.dialog.as_mut()) {
            d.checked = !d.checked;
            cx.notify();
        }
    }
}

pub(crate) struct PermissionAsk {
    pub id: u64,
    pub origin: String,
    pub kinds: Vec<Permission>,
    pub shown_at: Instant,
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

/// `https://a.com:8080/x?y` is `https://a.com:8080`; `None` for pages with no origin of their own
/// (about:blank, file:, data:), which have no site settings.
fn origin_of(url: &str) -> Option<String> {
    let origin = url::Url::parse(url).ok()?.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

fn permission_icon(kind: Permission) -> &'static str {
    match kind {
        Permission::Camera => "camera",
        Permission::Microphone => "mic",
        Permission::Location => "map-pin",
        Permission::Notifications => "bell",
        Permission::Clipboard => "clipboard",
        _ => "sliders-horizontal",
    }
}

fn permission_name(kind: Permission) -> &'static str {
    match kind {
        Permission::Camera => "Camera",
        Permission::Microphone => "Microphone",
        Permission::Location => "Location",
        Permission::Notifications => "Notifications",
        Permission::Clipboard => "Clipboard",
        Permission::Midi => "MIDI Devices",
        // Site settings only: no ask shows them.
        _ => "Other",
    }
}

/// "example.com wants to use your camera and microphone".
pub(crate) fn permission_sentence(kinds: &[Permission]) -> String {
    let phrase = |k: &Permission| match k {
        Permission::Camera => "use your camera",
        Permission::Microphone => "use your microphone",
        Permission::Location => "know your location",
        Permission::Notifications => "show notifications",
        Permission::Clipboard => "see what you copy",
        Permission::Midi => "use your MIDI devices",
        _ => "use more of your device",
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
pub(crate) fn button<H: WebHost>(
    key: (&'static str, u64),
    label: impl Into<SharedString>,
    primary: bool,
    theme: &Theme,
    browser: &Browser<H>,
    cx: &mut Context<Browser<H>>,
) -> gpui::Stateful<gpui::Div> {
    let (h, p) = (browser.motion.get(key, Feel::Hover), browser.motion.get(key, Feel::Press));
    feel(button_look(key, label, primary, theme, h, p), key, cx)
}

/// [`button`] at hover `h` and press `p`, without the springs that move them.
pub(crate) fn button_look(
    key: (&'static str, u64),
    label: impl Into<SharedString>,
    primary: bool,
    theme: &Theme,
    h: f32,
    p: f32,
) -> Stateful<Div> {
    let bg: Hsla = if primary {
        mix(theme.accent, gpui::black(), 0.12 * h)
    } else {
        mix(surface::key().into(), surface::line().into(), h)
    };
    div()
        .id(key)
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
        .child(label.into() as SharedString)
}

impl<H: WebHost> Browser<H> {
    pub(crate) fn on_ask(&mut self, tab: TabId, event: &PageEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let PageEvent::AudibleChanged(on) = event {
            if *on {
                self.audible.insert(tab);
            } else {
                self.audible.remove(&tab);
            }
            self.retarget_mini_player();
            return cx.notify();
        }
        let now = cx.background_executor().now();
        let changed = self.asks.entry(tab).or_default().on_event(event, now);
        let shown = self.shown() == Some(Shown::Tab(tab));
        self.asks_changed(event, changed && shown, window, cx);
    }

    pub(crate) fn asks_changed(&mut self, event: &PageEvent, dialog: bool, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(event, PageEvent::PermissionRequested { .. }) {
            self.motion.from(("ask-in", 0), Feel::Glide, 0.0, 1.0);
        }
        if dialog && self.active_dialog().is_none() {
            self.close_overlays(window, cx);
        } else if dialog && self.bar.is_none() {
            window.focus(&self.input_focus, cx);
        }
        cx.notify();
    }

    pub(crate) fn toggle_mute(&mut self, tab: TabId, cx: &mut Context<Self>) {
        // `muted` follows Chrome's PageEvent::Muted, not this guess.
        let muted = !self.muted.contains(&tab);
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

    /// The header's extensions button, Arc's: every extension, pinned or not, and the site's settings.
    pub(crate) fn render_extensions_button(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let key = ("extensions", 0);
        let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
        let open = self.popover == Some(Popover::Extensions);
        let el = div()
            .id("extensions-button")
            .size(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(r(7.))
            .bg(fade(theme.hover, h.max(open as u8 as f32)))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.toggle_popover(Popover::Extensions, window, cx)))
            .child(icon("sliders-horizontal", 15. * (1. - 0.15 * p), mix(theme.ink_muted, theme.ink, h)));
        feel(el, key, cx).into_any_element()
    }

    /// Arc's site menu: page actions, every extension as a tile (the badge pins it to the
    /// toolbar, a right click has the rest), the site's decided permissions, and its security.
    fn render_extensions_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let unpinned = &self.settings.unpinned_extensions;
        let tile = |id: (&'static str, u64), h: f32| {
            div()
                .id(id)
                .relative()
                .h(px(44.))
                .min_w_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(r(10.))
                .bg(mix(surface::key().into(), surface::line().into(), h))
        };
        let heading = |text: &'static str| {
            div().px(px(4.)).pt(px(12.)).pb(px(8.)).text_size(px(14.)).font_weight(gpui::FontWeight::BOLD).child(text)
        };
        let actions: Vec<AnyElement> = [
            (Command::CopyUrl, "Copy Link"),
            (Command::ReaderMode, "Reader Mode"),
            (Command::PictureInPicture, "Picture in Picture"),
            (Command::TranslatePage, "Translate Page"),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (c, label))| {
                let key = ("site-action", i as u64);
                let h = self.motion.get(key, Feel::Hover);
                let tip = SharedString::from(label);
                let el = tile(key, h)
                    .tooltip(move |_, cx| cx.new(|_| Tip(tip.clone())).into())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_overlays(window, cx);
                        this.run_command(c, window, cx);
                    }))
                    .child(icon(c.icon(), 20., surface::ink().into()));
                feel(el, key, cx).into_any_element()
            })
            .collect();
        let mut tiles: Vec<AnyElement> = self
            .extensions
            .iter()
            .enumerate()
            .map(|(i, ext)| {
                let key = ("ext-tile", i as u64);
                let h = self.motion.get(key, Feel::Hover);
                let pinned = !unpinned.contains(&ext.id);
                let (open, pin, menu) = (ext.clone(), ext.id.clone(), ext.id.clone());
                let name = SharedString::from(ext.name.clone());
                let badge = div()
                    .id(("ext-pin", i as u64))
                    .absolute()
                    .bottom(px(-3.))
                    .right(px(-3.))
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(surface::ink())
                    .border_2()
                    .border_color(surface::bg())
                    .opacity(if pinned { 1. } else { h })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_extension_pin(&pin, cx);
                    }))
                    .child(icon(if pinned { "pin" } else { "pin-off" }, 10., surface::bg().into()));
                let el = tile(key, h)
                    .tooltip(move |_, cx| cx.new(|_| Tip(name.clone())).into())
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
                    .child(crate::icons::extension_icon(ext, 20., surface::icon().into()))
                    .child(badge);
                feel(el, key, cx).into_any_element()
            })
            .collect();
        let key = ("ext-add", 0);
        let h = self.motion.get(key, Feel::Hover);
        let add = tile(key, h)
            .tooltip(|_, cx| cx.new(|_| Tip("Add Extensions".into())).into())
            .on_click(cx.listener(|this, _, window, cx| {
                this.close_overlays(window, cx);
                this.open_url("https://chromewebstore.google.com/", window, cx)
            }))
            .child(icon("plus", 18., surface::icon().into()));
        tiles.push(feel(add, key, cx).into_any_element());

        let page = self.active_page().map(|p| (p.read(cx).url(), p.read(cx).is_secure()));
        let web = page.filter(|(url, _)| url.starts_with("http"));
        // Arc lists only what the user decided; the lock's panel has every permission.
        let settings: Vec<AnyElement> = match (&web, self.active_space().map(|s| s.profile)) {
            (Some((url, _)), Some(profile)) => SITE_PERMISSIONS
                .iter()
                .map(|kind| (*kind, self.host.site_permission(profile, url, *kind)))
                .filter(|(_, state)| *state != PermissionState::Ask)
                .map(|(kind, state)| {
                    let allowed = state == PermissionState::Allow;
                    let url = url.clone();
                    div()
                        .id(SharedString::from(format!("site-setting-{kind:?}")))
                        .p(px(4.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .rounded(r(8.))
                        .hover(|d| d.bg(surface::key()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let next = if allowed { PermissionState::Block } else { PermissionState::Allow };
                            this.host.set_site_permission(profile, &url, kind, next);
                            cx.notify();
                        }))
                        .child(
                            div()
                                .size(px(36.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(if allowed { surface::ink() } else { surface::key() })
                                .child(icon(
                                    permission_icon(kind),
                                    16.,
                                    if allowed { surface::bg().into() } else { surface::icon().into() },
                                )),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(div().truncate().font_weight(gpui::FontWeight::SEMIBOLD).child(permission_name(kind)))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(surface::muted())
                                        .child(if allowed { "Allowed" } else { "Blocked" }),
                                ),
                        )
                        .into_any_element()
                })
                .collect(),
            _ => Vec::new(),
        };
        let security = web.map(|(_, secure)| {
            div()
                .id("extensions-panel-site")
                .h(px(32.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(r(8.))
                .bg(surface::key())
                .hover(|d| d.bg(surface::line()))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .on_click(cx.listener(|this, _, window, cx| this.toggle_popover(Popover::Site, window, cx)))
                .child(icon(
                    if secure { "lock" } else { "triangle-alert" },
                    13.,
                    if secure { surface::ink().into() } else { surface::warn().into() },
                ))
                .child(if secure { "Secure" } else { "Not Secure" })
        });
        let manage = div()
            .id("extensions-manage")
            .ml_auto()
            .size(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(surface::key())
            .hover(|d| d.bg(surface::line()))
            .tooltip(|_, cx| cx.new(|_| Tip("Manage Extensions".into())).into())
            .on_click(cx.listener(|this, _, window, cx| {
                this.open_own_page("baa://extensions", window, cx);
            }))
            .child(icon("ellipsis", 16., surface::ink().into()));
        drop_in(
            Self::popover_card("extensions-panel", 300.)
                .p(px(10.))
                .rounded(r(16.))
                .child(div().grid().grid_cols(4).gap(px(8.)).children(actions))
                .child(heading("Extensions"))
                .child(div().grid().grid_cols(4).gap(px(8.)).children(tiles))
                .when(!settings.is_empty(), |d| d.child(heading("Settings")).children(settings))
                .child(
                    div()
                        .mt(px(10.))
                        .pt(px(10.))
                        .flex()
                        .items_center()
                        .border_t_1()
                        .border_color(surface::line())
                        .children(security)
                        .child(manage),
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

    pub(crate) fn render_asks(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let account = self.render_account_ask(theme, cx);
        let tab = self.active_tab();
        let url = tab.and_then(|t| self.session.tab(t)).map(|t| t.url.clone()).unwrap_or_default();
        let asks = tab.and_then(|t| self.asks.get(&t)).map(|asks| self.render_page_asks(asks, &url, theme, cx)).unwrap_or_default();
        if account.is_none() && asks.is_empty() {
            return None;
        }
        Some(ask_layer().children(account).children(asks).into_any_element())
    }

    pub(crate) fn render_page_asks(&self, asks: &Asks, url: &str, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let extra = asks.dialog.as_ref().filter(|d| dialog::is_install(&d.dialog)).map(|d| self.render_install_target(d, theme, cx));
        page_asks(self, asks, url, theme, &self.input_focus, self.motion.get(("ask-in", 0), Feel::Glide), extra, cx)
    }

    /// The open header popover, if any, with a backdrop that closes it.
    pub(crate) fn render_popover(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let which = self.popover?;
        let card = match which {
            Popover::Downloads => self.render_downloads(theme, cx),
            Popover::Site => self.render_site_panel(cx)?,
            Popover::Extensions => self.render_extensions_panel(cx),
        };
        let placed = match which {
            Popover::Downloads => div().absolute().top(px(HEADER_H)).right(px(INSET)).child(card),
            // Under its button, which has split to its right.
            Popover::Extensions => div().absolute().top(px(HEADER_H)).right(px(INSET + 36.)).child(card),
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

    /// A download Chrome holds back until the user decides: why, and Keep or Discard.
    fn render_danger_row(&self, d: &Download, danger: DangerKind, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let id = d.id;
        let reason = match danger {
            DangerKind::Dangerous => "This file may harm your computer",
            DangerKind::Insecure => "This file was not downloaded securely",
        };
        div()
            .id(("download-danger", id))
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(icon("triangle-alert", 18., surface::warn().into()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(div().truncate().font_weight(gpui::FontWeight::MEDIUM).child(SharedString::from(download_name(d))))
                            .child(div().text_size(px(11.5)).text_color(surface::warn()).child(reason)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(6.))
                    .child(
                        button(("download-keep", id), "Keep", false, theme, self, cx)
                            .debug_selector(move || format!("download-keep-{id}"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.host.download_validate(id);
                            })),
                    )
                    .child(
                        button(("download-discard", id), "Discard", true, theme, self, cx)
                            .debug_selector(move || format!("download-discard-{id}"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.host.download_discard(id);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_downloads(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let items = self.visible_downloads(cx);
        let rows: Vec<AnyElement> = items
            .iter()
            .rev()
            .map(|d| {
                if let Some(danger) = d.danger {
                    return self.render_danger_row(d, danger, theme, cx);
                }
                let id = d.id;
                let key = ("download-row", id);
                let h = self.motion.get(key, Feel::Hover);
                let path = std::path::PathBuf::from(&d.path);
                let done = d.state == DownloadState::Complete;
                let running = d.state == DownloadState::InProgress;
                let fraction = (d.total_bytes > 0).then(|| d.received_bytes as f32 / d.total_bytes as f32);
                let open_path = path.clone();
                let trailing = if running {
                    div()
                        .id(("download-cancel", id))
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
                        .id(("download-reveal", id))
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
                    .id(("download", id))
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
                button(("downloads-all", 0), "Show All", false, theme, self, cx).on_click(cx.listener(
                    |this, _, window, cx| {
                        this.close_overlays(window, cx);
                        this.open_library(crate::library::Section::Downloads, window, cx);
                    },
                )),
            )
            .child(button(("downloads-clear", 0), "Clear", false, theme, self, cx).on_click(cx.listener(
                |this, _, window, cx| {
                    let done: Vec<u64> = this
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
        let origin = origin_of(&url);
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
                        cut_left(div())
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
                                if secure { surface::muted().into() } else { surface::warn().into() },
                            ))
                            .child(if secure { "Connection is secure" } else { "Connection is not secure" }),
                    )
                    .child(div().mx(px(8.)).h(px(1.)).bg(surface::line()))
                    .children(rows)
                    .when_some(origin, |d, origin| d.child(
                        div()
                            .id("site-settings")
                            .mx(px(8.))
                            .mt(px(4.))
                            .h(px(26.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .rounded(r(6.))
                            .text_size(px(12.))
                            .text_color(surface::muted())
                            .hover(|d| d.bg(surface::key()))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.close_overlays(window, cx);
                                this.open_site_settings(origin.clone(), cx);
                            }))
                            .child("Site settings…"),
                    )),
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
