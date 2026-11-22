//! The tab strip over the mail view: the list's tab, always first; the preview tab in a narrow
//! view; then one tab a draft (mocks A2/B2, A3/B3; WP-J). Which is in front is
//! `ViewState::tab()`, the one model every part of the mail view reads.

use baa_mail::{Preview, Tab};
use baa_web::WebHost;
use gpui::{div, prelude::*, px, AnyElement, Context, Hsla, Window};

use crate::{
    browser::Browser,
    icons::icon,
    theme::{ascii, r, surface, Theme},
};

impl<H: WebHost> Browser<H> {
    pub(crate) fn show_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        let Some(ui) = self.mail.as_mut() else { return };
        ui.state.set_tab(tab);
        ui.save_view();
        self.show_mail_documents(cx);
        cx.notify();
    }

    /// The back control, ⌘[ and Esc on the preview tab: the list, scrolled back to where the
    /// conversation was picked, with it still selected and its tab kept. False elsewhere.
    pub(crate) fn back_to_list(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(ui) = self.mail.as_mut() else { return false };
        if ui.state.tab() != Tab::Preview {
            return false;
        }
        ui.list.scroll_to_item(ui.state.scroll, gpui::ScrollStrategy::Top);
        window.focus(&ui.focus, cx);
        self.show_tab(Tab::List, cx);
        true
    }

    /// ⌘W in mail: the tab in front closes, except the list's.
    pub(crate) fn close_mail_tab(&mut self, cx: &mut Context<Self>) {
        match self.mail.as_ref().map(|m| m.state.tab()) {
            Some(Tab::Preview) => self.close_preview(cx),
            Some(Tab::Draft(id)) => self.request_close_draft(id, cx),
            _ => {}
        }
    }

    pub(crate) fn close_preview(&mut self, cx: &mut Context<Self>) {
        let Some(ui) = self.mail.as_mut() else { return };
        ui.drop_reader();
        ui.save_view();
        self.show_mail_documents(cx);
        cx.notify();
    }

    pub(crate) fn render_mail_tabs(&self, _theme: &Theme, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(ui) = &self.mail else { return div().into_any_element() };
        let front = ui.state.tab();
        let dim: Hsla = surface::muted().into();
        let tab = |lit: bool| {
            let el = div().flex().min_w_0().items_center().h(px(28.)).px(px(10.)).gap(px(6.)).text_size(px(12.));
            if ascii() {
                el.border_1().border_color(if lit { surface::ink() } else { surface::border() })
            } else {
                el.rounded(r(8.)).when(lit, |d| d.bg(surface::key()).font_weight(gpui::FontWeight::SEMIBOLD))
            }
        };
        let label = |text: String| div().min_w_0().overflow_hidden().whitespace_nowrap().child(text);
        let close = |id: (&'static str, u64), cx: &mut Context<Self>, run: fn(&mut Self, u64, &mut Context<Self>)| {
            div()
                .id(id)
                .flex_none()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    run(this, id.1, cx)
                }))
                .child(icon("x", 11., dim))
        };
        let (title, count, _) = self.list_title();
        let list = tab(front == Tab::List)
            .id("mail-tab-list")
            .debug_selector(|| "mail-tab-list".into())
            .flex_none()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| this.show_tab(Tab::List, cx)))
            .child(title)
            .when(count > 0, |d| d.child(div().text_color(dim).child(count.to_string())));
        let preview = ui.reader.as_ref().filter(|_| ui.state.preview != Preview::Closed).map(|reader| {
            let subject = reader.message().map(|m| m.subject.trim()).filter(|s| !s.is_empty()).unwrap_or("(no subject)").to_owned();
            // Italic, as an editor marks its preview tab: the next conversation opened replaces it.
            tab(front == Tab::Preview)
                .id("mail-tab-preview")
                .debug_selector(|| "mail-tab-preview".into())
                .max_w(px(220.))
                .italic()
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.show_tab(Tab::Preview, cx)))
                .child(label(subject))
                .child(close(("mail-tab-preview-x", 0), cx, |this, _, cx| this.close_preview(cx)))
        });
        let drafts = ui.drafts.iter().map(|draft| {
            let id = draft.id;
            let name = match draft.subject.text.trim() {
                "" => "New message".to_owned(),
                subject => subject.to_owned(),
            };
            tab(front == Tab::Draft(id))
                .id(("mail-draft-tab", id.0))
                .debug_selector(move || format!("mail-draft-tab-{}", id.0))
                .max_w(px(220.))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.show_tab(Tab::Draft(id), cx)))
                .child(label(name))
                .when(draft.dirty, |d| d.child(div().flex_none().text_color(dim).child("●")))
                .child(close(("mail-draft-tab-x", id.0), cx, |this, id, cx| this.request_close_draft(baa_mail::DraftId(id), cx)))
        }).collect::<Vec<_>>();
        let plus = div()
            .id("mail-new-message")
            .flex_none()
            .cursor_pointer()
            .px(px(8.))
            .on_click(cx.listener(|this, _, window, cx| this.new_message(window, cx)))
            .child(icon("plus", 13., dim));
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .px(px(10.))
            .py(px(6.))
            .overflow_hidden()
            .border_b_1()
            .border_color(surface::line())
            .child(list)
            .children(preview)
            .children(drafts)
            .child(plus)
            .into_any_element()
    }
}
