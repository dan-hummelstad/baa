//! The Archive's Mailbox section: the door into mail (mocks A1/B1; WP-H).

use baa_mail::{Filter, View};
use baa_web::WebHost;
use gpui::{div, prelude::*, px, AnyElement, Context, Hsla, SharedString, Window};

use super::{list::account_colour, nav::inbox_unread};
use crate::{
    browser::Browser,
    icons::icon,
    theme::{ascii, r, Theme},
};

fn label(theme: &Theme, text: &'static str) -> AnyElement {
    if ascii() {
        crate::theme::ascii::rule_label(text, theme.ink_faint).h(px(22.)).into_any_element()
    } else {
        div().flex_none().pt(px(10.)).pb(px(4.)).text_size(px(10.)).text_color(theme.ink_faint).child(text).into_any_element()
    }
}

impl<H: WebHost> Browser<H> {
    pub(crate) fn render_door(&self, theme: &Theme, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let col = div().size_full().flex().flex_col().p(px(12.)).gap(px(4.)).text_size(px(12.5)).text_color(theme.ink);
        let Some((mail, space)) = self.mail_space() else { return col.into_any_element() };
        let accounts = mail.accounts(space);
        let folders = mail.folders(space);
        let boxed = |d: gpui::Div| {
            d.flex_none().p(px(10.)).rounded(r(12.)).when(ascii(), |d| d.border_1().border_color(crate::theme::ascii::rule())).when(!ascii(), |d| d.bg(theme.glass_strong))
        };
        let mut col = col.child(label(theme, "MAILBOX · THIS SPACE"));

        if accounts.is_empty() {
            col = col.child(
                boxed(div().debug_selector(|| "mail-add-account".into()).flex().flex_col().gap(px(8.)))
                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Add an account"))
                    .child(div().text_size(px(11.)).text_color(theme.ink_muted).child("Read and send mail here, next to your tabs. Accounts are kept in this Space, or in all of them."))
                    .child(
                        div()
                            .id("mail-add-account-button")
                            .h(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(r(8.))
                            .bg(theme.accent)
                            .text_color(if ascii() { crate::theme::surface::bg().into() } else { gpui::white() })
                            .on_click(cx.listener(|this, _, _, cx| this.open_mail_settings(cx)))
                            .child("Add an account"),
                    ),
            );
        } else {
            let total = inbox_unread(&folders, None);
            let mut card = boxed(div().debug_selector(|| "mail-door-card".into()).flex().flex_col().gap(px(5.))).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().flex_1().font_weight(gpui::FontWeight::SEMIBOLD).child("All inboxes"))
                    .child(div().px(px(7.)).rounded(r(8.)).text_size(px(11.)).bg(theme.accent).text_color(if ascii() { crate::theme::surface::bg().into() } else { gpui::white() }).child(total.to_string())),
            );
            for (i, a) in accounts.iter().enumerate() {
                card = card.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(11.))
                        .text_color(theme.ink_muted)
                        .child(div().flex_none().size(px(6.)).rounded_full().bg(account_colour(i)))
                        .child(div().flex_1().min_w_0().truncate().child(a.address.clone()))
                        .child(inbox_unread(&folders, Some(a.id)).to_string()),
                );
            }
            col = col.child(card.child(div().text_size(px(10.)).text_color(theme.ink_faint).child(format!("{} mailboxes, combined", accounts.len()))));
            col = col.child(
                div()
                    .id("open-mailbox")
                    .flex_none()
                    .mt(px(6.))
                    .h(px(34.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .rounded(r(10.))
                    .bg(theme.accent)
                    .text_color(if ascii() { crate::theme::surface::bg().into() } else { gpui::white() })
                    .on_click(cx.listener(|this, _, window, cx| this.open_mail_from_door(window, cx)))
                    .when(!ascii(), |d| d.child(icon("mail-open", 14., gpui::white())))
                    .child(if ascii() { "OPEN MAILBOX" } else { "Open Mailbox" }),
            );
            col = col.child(div().flex_none().text_size(px(10.)).text_color(theme.ink_faint).child("Switches baa to the Mail section"));

            col = col.child(label(theme, "WAITING FOR YOU"));
            for (n, (filter, name, glyph)) in [(Filter::Unread, "Unread", "circle-dot"), (Filter::Flagged, "Flagged", "flag")].into_iter().enumerate() {
                let count = mail.count(&View { space, folder: None, account: None, filter, search: String::new() });
                col = col.child(self.door_row(theme, ("door-filter", n as u64), glyph, name.into(), count, cx, move |this, window, cx| {
                    this.open_list(None, filter, cx);
                    this.open_mail_from_door(window, cx);
                }));
            }
        }

        let others = self.mail_other_spaces();
        if !others.is_empty() {
            col = col.child(label(theme, "OTHER SPACES"));
            for (id, name, unread) in others {
                col = col.child(self.door_row(theme, ("door-space", id.0), "mail", name, unread as usize, cx, move |this, window, cx| {
                    this.close_library(window, cx);
                    this.show_mail_of(id, window, cx);
                }));
            }
        }
        col.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn door_row(&self, theme: &Theme, key: (&'static str, u64), glyph: &'static str, name: SharedString, count: usize, cx: &mut Context<Self>, go: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> AnyElement {
        let h = self.motion.get(key, crate::motion::Feel::Hover);
        let tone: Hsla = theme.ink;
        let el = div()
            .id(key)
            .flex_none()
            .h(px(28.))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(r(8.))
            .text_color(tone)
            .bg(crate::theme::fade(theme.hover, h))
            .on_click(cx.listener(move |this, _, window, cx| go(this, window, cx)))
            .child(if ascii() { div().child("▸").into_any_element() } else { icon(glyph, 14., theme.ink_muted).into_any_element() })
            .child(div().flex_1().min_w_0().truncate().child(name))
            .when(count > 0, |d| d.child(div().text_size(px(11.)).text_color(theme.ink_muted).child(count.to_string())));
        crate::browser::feel(el, key, cx).into_any_element()
    }
}
