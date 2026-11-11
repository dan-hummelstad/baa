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
}

impl gpui::Global for Updater {}

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
        let card = div()
            .debug_selector(|| "update-card".into())
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
                    .child("New baa Version Available"),
            )
            .child(div().p(px(8.)).flex().flex_col().gap(px(8.)).child(check).child(install));
        Some(drop_in(card, "update-card").into_any_element())
    }
}
