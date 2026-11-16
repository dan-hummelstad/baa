//! "New baa Version Available" at the foot of the sidebar (Arc's card), once the app
//! shell's updater holds a downloaded update.

use baa_web::WebHost;
use gpui::{div, prelude::*, px, AnyElement, Context};

use crate::{
    browser::{drop_in, Browser},
    icons::icon,
    theme::{r, surface, Theme},
};

/// The app shell's updater. Absent in a build that cannot update itself (a dev engine).
pub struct Updater {
    /// Whether baa looks for updates by itself. The updater keeps the choice.
    pub automatic: Box<dyn Fn() -> bool>,
    pub set_automatic: Box<dyn Fn(bool)>,
    /// Installs the update that is ready and starts baa again.
    pub install: Box<dyn Fn()>,
    /// Looks for an update now. The answer comes back as [`UpdateCheck`].
    pub check: Box<dyn Fn()>,
}

impl gpui::Global for Updater {}

/// What the last check found; the shell sets it from the updater's answers. A found update
/// downloads by itself and then shows the sidebar card.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum UpdateCheck {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Found,
    Failed(String),
}

impl gpui::Global for UpdateCheck {}

impl<H: WebHost> Browser<H> {
    /// An update is downloaded. It installs when baa quits, or now from the card.
    pub fn update_ready(&mut self, cx: &mut Context<Self>) {
        self.update_ready = true;
        cx.notify();
    }

    pub(crate) fn render_update_card(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.update_ready {
            return None;
        }
        let automatic = cx.try_global::<Updater>().is_some_and(|u| (u.automatic)());
        let check = div()
            .id("update-automatic")
            .debug_selector(|| "update-automatic".into())
            .flex()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .cursor_pointer()
            .text_size(px(12.5))
            .text_color(surface::muted())
            .child(
                div()
                    .size(px(15.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(r(4.))
                    .border_1()
                    .border_color(if automatic { theme.accent } else { surface::icon().into() })
                    .bg(if automatic { theme.accent } else { surface::raised().into() })
                    .when(automatic, |d| d.child(icon("check", 11., gpui::white()))),
            )
            .child("Automatic Updates")
            .on_click(cx.listener(move |_, _, _, cx| {
                if let Some(updater) = cx.try_global::<Updater>() {
                    (updater.set_automatic)(!automatic);
                }
                cx.notify();
            }));
        let install = crate::prompts::button(("update-install", 0), "Restart and Update", true, theme, self, cx)
            .debug_selector(|| "update-install".into())
            .justify_center()
            .on_click(cx.listener(|_, _, _, cx| {
                if let Some(updater) = cx.try_global::<Updater>() {
                    (updater.install)();
                }
            }));
        Some(sidebar_card("update-card", "New baa Version Available", [check.into_any_element(), install.into_any_element()]))
    }

    /// The saved session or settings could not be read, so this run saves nothing; shown until
    /// dismissed, as the log is not where anyone looks.
    pub(crate) fn render_data_notice(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.data_notice {
            return None;
        }
        let text = div()
            .text_size(px(12.5))
            .text_color(surface::muted())
            .child("baa could not read its saved session or settings. Changes in this session will not be saved.");
        let ok = crate::prompts::button(("data-notice-ok", 0), "OK", true, theme, self, cx)
            .debug_selector(|| "data-notice-ok".into())
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| {
                this.data_notice = false;
                cx.notify();
            }));
        Some(sidebar_card("data-notice", "Saved Data Unreadable", [text.into_any_element(), ok.into_any_element()]))
    }
}

fn sidebar_card(id: &'static str, title: &'static str, body: impl IntoIterator<Item = AnyElement>) -> AnyElement {
    let card = div()
        .debug_selector(move || id.into())
        .flex_none()
        .mb(px(6.))
        .flex()
        .flex_col()
        .rounded(r(12.))
        .overflow_hidden()
        .bg(surface::bg())
        .border_1()
        .border_color(surface::border())
        .text_color(surface::ink())
        .child(
            div()
                .h(px(30.))
                // gpui clips children to the box, not to its rounded corners.
                .rounded_t(r(11.))
                .flex()
                .items_center()
                .justify_center()
                .bg(surface::key())
                .text_size(px(12.5))
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(title),
        )
        .child(div().p(px(8.)).flex().flex_col().gap(px(8.)).children(body));
    drop_in(card, id).into_any_element()
}
