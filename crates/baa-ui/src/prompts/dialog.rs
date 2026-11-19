//! The page dialog sheet: JS dialogs, "leave site?", and Chrome's own (HTTP auth,
//! external protocol, unresponsive page, extension install).

use crate::theme::r;

use baa_model::SpaceId;
use baa_web::{Dialog, DialogAnswer, DialogKind, WebHost};
use gpui::{div, prelude::*, px, relative, AnyElement, App, Context, Div, Entity, FocusHandle, Hsla, KeyDownEvent, SharedString, Stateful, Window};

use super::{button, site_name, Asks, Prompter};
use crate::{
    browser::{cut_left, drop_in, shadow, Browser, InstallTarget, Shown},
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
pub(super) fn is_install(d: &Dialog) -> bool {
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

    pub(crate) fn field(&mut self) -> Option<&mut TextField> {
        match self.dialog.kind {
            DialogKind::Auth if self.on_password => Some(&mut self.password),
            DialogKind::Auth | DialogKind::Prompt => Some(&mut self.field),
            _ => None,
        }
    }

    /// The key as the dialog took it, and the answer it gives (OK or Cancel), if any.
    pub(crate) fn on_key(&mut self, event: &KeyDownEvent, cx: &App) -> (Key, Option<bool>) {
        let kind = self.dialog.kind;
        let auth = kind == DialogKind::Auth;
        if event.keystroke.key == "space" && !event.is_held && kind != DialogKind::Prompt && !auth && !self.dialog.check_label.is_empty() {
            self.checked = !self.checked;
            return (Key::Ignored, None);
        }
        // Tab (not → at the end, which the field also reports as Complete) moves between auth fields.
        if auth && (event.keystroke.key == "tab" || matches!(event.keystroke.key.as_str(), "up" | "down")) {
            self.on_password = !self.on_password;
            return (Key::Ignored, None);
        }
        let key = match self.field() {
            Some(field) => field.handle(&event.keystroke, cx),
            None => match event.keystroke.key.as_str() {
                "enter" => Key::Submit,
                "escape" => Key::Cancel,
                _ => Key::Ignored,
            },
        };
        // A held key repeats into whatever appears next, and a risky dialog (open an
        // app, install an extension) is never accepted from the keyboard.
        let risky = matches!(kind, DialogKind::ExternalProtocol | DialogKind::ExtensionInstall);
        let answer = match key {
            Key::Submit | Key::Cancel if event.is_held => None,
            Key::Submit if risky => None,
            Key::Submit => Some(true),
            Key::Cancel => Some(false),
            _ => None,
        };
        (key, answer)
    }

    pub(crate) fn answer(&self, ok: bool) -> DialogAnswer {
        DialogAnswer { ok, text: self.field.text.clone(), password: self.password.text.clone(), checked: self.checked }
    }
}

impl<H: WebHost> Browser<H> {
    fn shown_asks(&self) -> Option<&Asks> {
        match self.shown()? {
            Shown::Peek => self.peek.as_ref().map(|p| &p.asks),
            Shown::Tab(tab) => self.asks.get(&tab),
        }
    }

    pub(crate) fn shown_page(&self) -> Option<(Entity<H::Page>, SpaceId)> {
        match self.shown()? {
            Shown::Peek => self.peek.as_ref().map(|p| (p.page.clone(), p.space)),
            Shown::Tab(tab) => Some((self.pages.get(&tab)?.0.clone(), self.session.tab(tab)?.space)),
        }
    }

    pub(crate) fn active_dialog(&self) -> Option<&DialogAsk> {
        self.shown_asks()?.dialog.as_ref()
    }

    pub(crate) fn dialog_field(&mut self) -> Option<&mut TextField> {
        self.shown_asks_mut()?.dialog.as_mut()?.field()
    }

    pub(crate) fn set_install_target(&mut self, target: InstallTarget, cx: &mut Context<Self>) {
        if let Some(d) = self.shown_asks_mut().and_then(|a| a.dialog.as_mut()) {
            d.target = target;
            cx.notify();
        }
    }

    /// "Add to": this Space, another one, or all of them.
    pub(super) fn render_install_target(&self, ask: &DialogAsk, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let here = self.shown_page().map(|(_, space)| space);
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
                .bg(if on { Hsla { a: 0.12, ..theme.accent } } else { surface::raised().into() })
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
}

impl<H: WebHost> Prompter for Browser<H> {
    type Page = H::Page;

    fn shown_asks_mut(&mut self) -> Option<&mut Asks> {
        match self.shown()? {
            Shown::Peek => self.peek.as_mut().map(|p| &mut p.asks),
            Shown::Tab(tab) => self.asks.get_mut(&tab),
        }
    }

    fn prompt_page(&self) -> Option<Entity<H::Page>> {
        self.shown_page().map(|(page, _)| page)
    }

    fn answered(&mut self, ask: &DialogAsk, ok: bool, window: &mut Window, cx: &mut Context<Self>) {
        if ok && is_install(&ask.dialog) {
            if let Some((_, space)) = self.shown_page() {
                self.install_accepted(space, ask.target, &ask.dialog.title, cx);
            }
        }
        // Chrome closes a tab opened only for an app link once the app opens; cancelled,
        // its Peek has nothing to show.
        if !ok && ask.dialog.kind == DialogKind::ExternalProtocol && self.peek.as_ref().is_some_and(|p| p.app_link) {
            self.close_peek(window, cx);
        }
        self.close_overlays(window, cx);
    }

    // The sheet shares the keyboard's focus with the command bar, find and rename,
    // which take the keys first while open.
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.on_input_key(event, window, cx)
    }

    fn button(&self, key: (&'static str, u64), label: SharedString, primary: bool, theme: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        button(key, label, primary, theme, self, cx)
    }
}

pub(super) fn render_dialog<V: Prompter>(
    view: &V,
    ask: &DialogAsk,
    url: &str,
    theme: &Theme,
    focus: &FocusHandle,
    extra: Option<AnyElement>,
    cx: &mut Context<V>,
) -> [AnyElement; 2] {
    let d = &ask.dialog;
    let (title, message, ok, cancel) = wording(d, url);
    let field_box = |f: &TextField, focused: bool, placeholder: &'static str| {
        div()
            .h(px(32.))
            .flex_shrink_0()
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(r(7.))
            .border_1()
            .border_color(if focused { theme.accent } else { surface::border().into() })
            .bg(surface::raised())
            .text_size(px(13.))
            .child(f.render(focused, placeholder, surface::ink().into(), surface::muted().into(), Hsla { a: 0.25, ..theme.accent }))
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
                    .bg(if on { theme.accent } else { surface::raised().into() })
                    .when(on, |d| d.child(icon("check", 11., gpui::white()))),
            )
            .child(SharedString::from(d.check_label.clone()))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_check(cx)))
    });
    let card = div()
        .id("page-dialog")
        .debug_selector(|| "page-dialog".into())
        .occlude()
        .mt(px(16.))
        .w(px(420.))
        .max_w(relative(0.9))
        // A small window shrinks the message, which scrolls, before the sheet runs off it.
        .max_h(relative(0.9))
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
        .track_focus(focus)
        .on_key_down(cx.listener(V::key))
        .child(
            cut_left(div())
                .flex_shrink_0()
                .text_size(px(14.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(surface::ink())
                .child(SharedString::from(title)),
        )
        .when(!message.is_empty(), |d| {
            d.child(
                div()
                    .id("page-dialog-message")
                    .min_h_0()
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
        .children(extra)
        .child(
            div()
                .flex_shrink_0()
                .flex()
                .justify_end()
                .gap(px(8.))
                .children(cancel.map(|label| {
                    view.button(("dialog-cancel", d.id), label.into(), false, theme, cx)
                        .debug_selector(|| "dialog-cancel".into())
                        .on_click(cx.listener(|this, _, window, cx| this.answer(false, window, cx)))
                }))
                .child(
                    view.button(("dialog-ok", d.id), ok.into(), true, theme, cx)
                        .debug_selector(|| "dialog-ok".into())
                        .on_click(cx.listener(|this, _, window, cx| this.answer(true, window, cx))),
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
