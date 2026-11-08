//! The page dialog sheet: JS dialogs, "leave site?", and Chrome's own (HTTP auth,
//! external protocol, unresponsive page, extension install).

use crate::theme::r;

use baa_web::{Dialog, DialogAnswer, DialogKind, PageHandle, WebHost};
use gpui::{div, prelude::*, px, relative, AnyElement, Context, Hsla, KeyDownEvent, SharedString, Window};

use super::{button, site_name, ACTIVATION_DELAY};
use crate::{
    browser::{cut_left, drop_in, shadow, Browser, InstallTarget},
    icons::icon,
    input::{Key, TextField},
    theme::{surface, Theme},
};

pub(crate) struct DialogAsk {
    pub dialog: Dialog,
    pub checked: bool,
    /// The prompt's answer or the username; unused (but still taking Enter/Escape) otherwise.
    pub field: TextField,
    /// Auth only.
    pub password: TextField,
    /// Auth only: Tab moves the keyboard to the password.
    pub on_password: bool,
    /// Extension installs only: where the extension goes.
    pub target: InstallTarget,
    pub shown_at: std::time::Instant,
}

/// An extension install prompt, not a re-enable or a permission increase (those keep
/// the extension where it is).
// ponytail: told apart by Chrome's button label ("Add extension"); a translated label
// hides the Space picker. Pass the prompt type through the ABI if that matters.
fn is_install(d: &Dialog) -> bool {
    d.kind == DialogKind::ExtensionInstall && (d.ok_label.is_empty() || d.ok_label.starts_with("Add"))
}

/// Title, message, OK and Cancel. Chrome's own wording when it sent any (its titles
/// name the frame's origin, "An embedded page at X says"); ours otherwise, where a
/// JS dialog names the tab's site only because Chrome gave no title.
pub(crate) fn wording(d: &Dialog, url: &str) -> (String, String, String, Option<String>) {
    let pick = |chrome: &str, ours: &str| if chrome.is_empty() { ours.to_string() } else { chrome.to_string() };
    let says = || pick(&d.title, &format!("{} says", site_name(url)));
    let chrome = |title, ok, cancel| (pick(&d.title, title), d.message.clone(), pick(&d.ok_label, ok), Some(pick(&d.cancel_label, cancel)));
    match d.kind {
        DialogKind::BeforeUnload { is_reload } => (
            if is_reload { "Reload site?" } else { "Leave site?" }.into(),
            "Changes you made may not be saved.".into(),
            if is_reload { "Reload" } else { "Leave" }.into(),
            Some("Stay".into()),
        ),
        DialogKind::Alert => (says(), d.message.clone(), "OK".into(), None),
        DialogKind::Confirm | DialogKind::Prompt => (says(), d.message.clone(), "OK".into(), Some("Cancel".into())),
        DialogKind::Auth => chrome("Sign in", "Sign In", "Cancel"),
        DialogKind::ExternalProtocol => chrome("Open external app?", "Open", "Cancel"),
        DialogKind::Unresponsive => chrome("Page unresponsive", "Exit pages", "Wait"),
        DialogKind::ExtensionInstall => chrome("Add extension?", "Add extension", "Cancel"),
    }
}

impl DialogAsk {
    pub(crate) fn new(d: &Dialog, shown_at: std::time::Instant) -> Self {
        let field = if d.kind == DialogKind::Prompt { TextField::selected(d.default_text.clone()) } else { TextField::default() };
        Self {
            dialog: d.clone(),
            checked: false,
            field,
            password: TextField { masked: true, ..Default::default() },
            on_password: false,
            target: InstallTarget::Here,
            shown_at,
        }
    }
}

impl<H: WebHost> Browser<H> {
    pub(crate) fn active_dialog(&self) -> Option<&DialogAsk> {
        self.asks.get(&self.active_tab()?)?.dialog.as_ref()
    }

    pub(crate) fn on_dialog_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ask) = self
            .active_tab()
            .and_then(|id| self.asks.get_mut(&id))
            .and_then(|a| a.dialog.as_mut())
        else {
            return;
        };
        let kind = ask.dialog.kind;
        let auth = kind == DialogKind::Auth;
        if event.keystroke.key == "space" && !event.is_held && kind != DialogKind::Prompt && !auth && !ask.dialog.check_label.is_empty() {
            ask.checked = !ask.checked;
            cx.notify();
            return;
        }
        // Tab (not → at the end, which the field also reports as Complete) moves between auth fields.
        if auth && (event.keystroke.key == "tab" || matches!(event.keystroke.key.as_str(), "up" | "down")) {
            ask.on_password = !ask.on_password;
            cx.notify();
            return;
        }
        let mut scratch = TextField::default();
        let field = match kind {
            DialogKind::Auth if ask.on_password => &mut ask.password,
            DialogKind::Auth | DialogKind::Prompt => &mut ask.field,
            _ => &mut scratch,
        };
        // A held key repeats into whatever appears next, and a risky dialog (open an
        // app, install an extension) is never accepted from the keyboard.
        let risky = matches!(kind, DialogKind::ExternalProtocol | DialogKind::ExtensionInstall);
        match field.handle(&event.keystroke, cx) {
            Key::Submit | Key::Cancel if event.is_held => {}
            Key::Submit if risky => {}
            Key::Submit => self.answer_dialog(true, window, cx),
            Key::Cancel => self.answer_dialog(false, window, cx),
            Key::Edited => cx.notify(),
            _ => {}
        }
    }

    pub(crate) fn toggle_dialog_check(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.active_tab().and_then(|t| self.asks.get_mut(&t)).and_then(|a| a.dialog.as_mut()) {
            d.checked = !d.checked;
            cx.notify();
        }
    }

    pub(crate) fn answer_dialog(&mut self, ok: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab() else { return };
        let now = cx.background_executor().now();
        let Some(asks) = self.asks.get_mut(&tab) else { return };
        if ok && asks.dialog.as_ref().is_some_and(|a| now < a.shown_at + ACTIVATION_DELAY) {
            return;
        }
        let Some(ask) = asks.dialog.take() else { return };
        asks.reveal(now);
        if ok && is_install(&ask.dialog) {
            if let Some(space) = self.session.tab(tab).map(|t| t.space) {
                self.install_accepted(space, ask.target, cx);
            }
        }
        if let Some((page, _)) = self.pages.get(&tab) {
            let answer = DialogAnswer { ok, text: ask.field.text, password: ask.password.text, checked: ask.checked };
            page.read(cx).answer_dialog(ask.dialog.id, answer);
        }
        self.close_overlays(window, cx);
    }

    pub(crate) fn set_install_target(&mut self, target: InstallTarget, cx: &mut Context<Self>) {
        if let Some(d) = self.active_tab().and_then(|t| self.asks.get_mut(&t)).and_then(|a| a.dialog.as_mut()) {
            d.target = target;
            cx.notify();
        }
    }

    /// "Add to": this Space, another one, or all of them.
    fn render_install_target(&self, ask: &DialogAsk, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let here = self.active_space().map(|s| s.id);
        let mut choices: Vec<(InstallTarget, String)> = self
            .session
            .spaces()
            .iter()
            .filter(|s| !s.ephemeral)
            .map(|s| {
                let target = if Some(s.id) == here { InstallTarget::Here } else { InstallTarget::Space(s.id) };
                (target, format!("{} {}", s.icon, s.name))
            })
            .collect();
        choices.push((InstallTarget::All, "All Spaces".into()));
        let chips = choices.into_iter().enumerate().map(|(i, (target, label))| {
            let on = ask.target == target;
            div()
                .id(("install-target", i))
                .debug_selector(move || format!("install-target-{i}"))
                .px(px(10.))
                .py(px(4.))
                .rounded(r(7.))
                .border_1()
                .border_color(if on { theme.accent } else { surface::border().into() })
                .bg(if on { Hsla { a: 0.12, ..theme.accent } } else { gpui::white() })
                .text_size(px(12.5))
                .text_color(surface::ink())
                .child(SharedString::from(label))
                .on_click(cx.listener(move |this, _, _, cx| this.set_install_target(target, cx)))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(div().text_size(px(12.)).text_color(surface::muted()).child("Add to"))
            .child(div().flex().flex_wrap().gap(px(6.)).children(chips))
            .into_any_element()
    }

    /// The scrim and the sheet over the page.
    pub(super) fn render_dialog(&self, ask: &DialogAsk, url: &str, theme: &Theme, cx: &mut Context<Self>) -> [AnyElement; 2] {
        let d = &ask.dialog;
        let (title, message, ok, cancel) = wording(d, url);
        let field_box = |f: &TextField, focused: bool, placeholder: &'static str| {
            div()
                .h(px(32.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(r(7.))
                .border_1()
                .border_color(if focused { theme.accent } else { surface::border().into() })
                .bg(gpui::white())
                .text_size(px(13.))
                .child(f.render(placeholder, surface::ink().into(), surface::muted().into(), Hsla { a: 0.25, ..theme.accent }))
        };
        let fields = match d.kind {
            DialogKind::Prompt => vec![field_box(&ask.field, true, "")],
            DialogKind::Auth => vec![
                field_box(&ask.field, !ask.on_password, "Username"),
                field_box(&ask.password, ask.on_password, "Password"),
            ],
            _ => vec![],
        };
        let list = d.kind == DialogKind::ExtensionInstall;
        let checkbox = (!d.check_label.is_empty()).then(|| {
            let on = ask.checked;
            div()
                .id(("dialog-check", d.id))
                .debug_selector(|| "dialog-check".into())
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(13.))
                .text_color(surface::ink())
                .child(
                    div()
                        .size(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(r(4.))
                        .border_1()
                        .border_color(if on { theme.accent } else { surface::border().into() })
                        .bg(if on { theme.accent } else { gpui::white() })
                        .when(on, |d| d.child(icon("check", 11., gpui::white()))),
                )
                .child(SharedString::from(d.check_label.clone()))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_dialog_check(cx)))
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
                cut_left(div())
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
                        .when(!list, |d| d.child(SharedString::from(message.clone())))
                        .when(list, |d| {
                            d.flex().flex_col().gap(px(4.)).children(message.lines().map(|l| {
                                div().flex().gap(px(6.)).child("\u{2022}").child(SharedString::from(l.to_string()))
                            }))
                        }),
                )
            })
            .children(fields)
            .children(checkbox)
            .children(is_install(d).then(|| self.render_install_target(ask, theme, cx)))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .children(cancel.map(|label| {
                        button(("dialog-cancel", d.id), label, false, theme, self, cx)
                            .on_click(cx.listener(|this, _, window, cx| this.answer_dialog(false, window, cx)))
                    }))
                    .child(
                        button(("dialog-ok", d.id), ok, true, theme, self, cx)
                            .debug_selector(|| "dialog-ok".into())
                            .on_click(cx.listener(|this, _, window, cx| this.answer_dialog(true, window, cx))),
                    ),
            );
        [
            div()
                .id("page-dialog-scrim")
                .absolute()
                .inset_0()
                .occlude()
                .rounded(r(crate::browser::PAGE_RADIUS))
                .bg(surface::scrim())
                .into_any_element(),
            drop_in(card, "page-dialog").into_any_element(),
        ]
    }
}
