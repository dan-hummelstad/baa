//! A page's asks over it: the dialog sheet, else the first permission as a banner.

use gpui::{div, prelude::*, px, relative, AnyElement, Context, Div, FocusHandle, SharedString};

use super::{dialog, permission_icon, permission_sentence, site_name, Asks, Prompter};
use crate::{
    browser::{cut_left, shadow},
    icons::icon,
    theme::{r, surface, Theme},
};

/// Where a page's asks are drawn: over the page, centred.
pub(crate) fn ask_layer() -> Div {
    div().absolute().inset_0().flex().flex_col().items_center()
}

/// The dialog as a sheet with the page dimmed behind it, else the first permission
/// as a banner under the top edge; `extra` goes in the sheet.
#[allow(clippy::too_many_arguments)]
pub(crate) fn page_asks<V: Prompter>(
    view: &V,
    asks: &Asks,
    url: &str,
    theme: &Theme,
    focus: &FocusHandle,
    banner_in: f32,
    extra: Option<AnyElement>,
    cx: &mut Context<V>,
) -> Vec<AnyElement> {
    if let Some(dialog) = &asks.dialog {
        return dialog::render_dialog(view, dialog, url, theme, focus, extra, cx).into();
    }
    let Some(ask) = asks.permissions.first() else { return Vec::new() };
    let host = site_name(&ask.origin);
    let icons = ask.kinds.iter().map(|k| icon(permission_icon(*k), 16., theme.accent));
    let banner = div()
        .id("permission")
        .occlude()
        .mt(px(10. - 14. * (1. - banner_in)))
        .opacity(banner_in.clamp(0., 1.))
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
                        .child(cut_left(div().min_w_0()).font_weight(gpui::FontWeight::SEMIBOLD).child(SharedString::from(host)))
                        .child(SharedString::from(format!("wants to {}", permission_sentence(&ask.kinds)))),
                ),
        )
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(
                    view.button(("perm-deny", ask.id), "Don't Allow".into(), false, theme, cx)
                        .debug_selector(|| "perm-deny".into())
                        .on_click(cx.listener(|this, _, _, cx| this.allow(false, cx))),
                )
                .child(
                    view.button(("perm-allow", ask.id), "Allow".into(), true, theme, cx)
                        .debug_selector(|| "perm-allow".into())
                        .on_click(cx.listener(|this, _, _, cx| this.allow(true, cx))),
                ),
        );
    vec![banner.into_any_element()]
}
