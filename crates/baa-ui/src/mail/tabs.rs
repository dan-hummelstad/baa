//! The tab strip over the mail view: Inbox, then one tab a draft (mocks A2/B2, A3/B3; WP-J).

use baa_mail::{Filter, View};
use baa_web::WebHost;
use gpui::{div, prelude::*, px, AnyElement, Context, Hsla, Window};

use crate::{
    browser::Browser,
    icons::icon,
    theme::{ascii, r, surface, Theme},
};

impl<H: WebHost> Browser<H> {
    pub(crate) fn render_mail_tabs(&self, _theme: &Theme, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(ui) = &self.mail else { return div().into_any_element() };
        let unread = ui.space.map_or(0, |space| {
            ui.mail.count(&View { space, folder: None, account: None, filter: Filter::Unread, search: String::new() })
        });
        let dim: Hsla = surface::muted().into();
        let tab = |lit: bool| {
            let el = div().flex().flex_none().items_center().h(px(28.)).px(px(10.)).gap(px(6.)).text_size(px(12.));
            if ascii() {
                el.border_1().border_color(if lit { surface::ink() } else { surface::border() })
            } else {
                el.rounded(r(8.)).when(lit, |d| d.bg(surface::key()).font_weight(gpui::FontWeight::SEMIBOLD))
            }
        };
        let inbox = tab(ui.state.front.is_none())
            .id("mail-tab-inbox")
            .debug_selector(|| "mail-tab-inbox".into())
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| this.show_front(None, cx)))
            .child("Inbox")
            .when(unread > 0, |d| d.child(div().text_color(dim).child(unread.to_string())));
        let drafts = ui.drafts.iter().map(|draft| {
            let id = draft.id;
            let name = match draft.subject.text.trim() {
                "" => "New message".to_owned(),
                subject => subject.to_owned(),
            };
            tab(ui.state.front == Some(id))
                .debug_selector(move || format!("mail-draft-tab-{}", id.0))
                .max_w(px(220.))
                .child(
                    div()
                        .id(("mail-draft-tab", id.0))
                        .cursor_pointer()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .on_click(cx.listener(move |this, _, _, cx| this.show_front(Some(id), cx)))
                        .child(name),
                )
                .when(draft.dirty, |d| d.child(div().text_color(dim).child("●")))
                .child(
                    div()
                        .id(("mail-draft-tab-x", id.0))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| this.request_close_draft(id, cx)))
                        .child(icon("x", 11., dim)),
                )
        });
        let plus = div()
            .id("mail-new-message")
            .cursor_pointer()
            .px(px(8.))
            .on_click(cx.listener(|this, _, window, cx| this.new_message(window, cx)))
            .child(icon("plus", 13., dim));
        let count = ui.drafts.len();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .px(px(10.))
            .py(px(6.))
            .border_b_1()
            .border_color(surface::line())
            .child(inbox)
            .children(drafts)
            .child(plus)
            .child(div().flex_1())
            .when(count > 0, |d| d.child(div().text_size(px(12.)).text_color(dim).child(format!("{count} drafts open"))))
            .into_any_element()
    }
}
