//! The reading pane (mocks A2/B2, right; WP-I). A message's HTML reaches Chromium only as a
//! message document (PLAN-mail.md §6), made afresh for each message and again when the user
//! allows its remote images: a document's policy is fixed when it is made.

use baa_mail::{Action, Address, Attachment, Body, Message, MessageId, ThreadId};
use baa_model::{now, DownloadRecord, RemoteImages};
use baa_web::{DocumentKind, PageEvent, PageHandle, WebHost};
use gpui::{div, prelude::*, px, AnyElement, Context, Entity, SharedString, Subscription, Window};

use crate::{
    browser::Browser,
    icons::icon,
    theme::{ascii, r, surface, Theme},
};

/// The conversation on screen and its newest message's page.
pub(crate) struct Reader<H: WebHost> {
    pub(crate) thread: ThreadId,
    /// Oldest first.
    pub(crate) messages: Vec<Message>,
    /// The message the page shows: the newest, or an earlier one picked from the list under it.
    pub(crate) shown: MessageId,
    /// `None` while its content is not here yet.
    pub(crate) body: Option<Body>,
    pub(crate) page: Option<Entity<H::Page>>,
    /// The page was made with remote images allowed.
    pub(crate) remote_images: bool,
    /// Where the link under the pointer goes, as the page says: shown, never loaded.
    pub(crate) target: String,
    pub(crate) earlier_open: bool,
    /// A dangerous attachment's Save was pressed once: the next press saves it.
    pub(crate) save_anyway: Option<usize>,
    _page: Option<Subscription>,
}

impl<H: WebHost> Reader<H> {
    pub(crate) fn message(&self) -> Option<&Message> {
        self.messages.iter().find(|m| m.id == self.shown)
    }
}

fn sender(a: &Address) -> String {
    if a.name.is_empty() { a.email.clone() } else { a.name.clone() }
}

fn people(list: &[Address]) -> String {
    list.iter().map(|a| a.email.as_str()).collect::<Vec<_>>().join(", ")
}

impl<H: WebHost> Browser<H> {
    /// A row of the list was picked: its conversation, its newest message in the page, and
    /// what was unread in it is read now.
    pub(crate) fn select_thread(&mut self, thread: ThreadId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mail) = &mut self.mail else { return };
        let messages = mail.mail.conversation(thread);
        let Some(newest) = messages.last().map(|m| m.id) else {
            mail.reader = None;
            return cx.notify();
        };
        let unread: Vec<MessageId> = messages.iter().filter(|m| m.unread).map(|m| m.id).collect();
        if !unread.is_empty() {
            mail.mail.act(&unread, Action::Read(true));
        }
        mail.state.selected = Some(thread);
        mail.state.front = None;
        mail.save_view();
        mail.reader = Some(Reader {
            thread,
            messages: mail.mail.conversation(thread),
            shown: newest,
            body: None,
            page: None,
            remote_images: false,
            target: String::new(),
            earlier_open: false,
            save_anyway: None,
            _page: None,
        });
        self.show_message(newest, false, window, cx);
    }

    /// Message `id` of the conversation in the page; `remote_images` once the user allowed them.
    fn show_message(&mut self, id: MessageId, remote_images: bool, window: &mut Window, cx: &mut Context<Self>) {
        let profile = self.active_space().map(|s| s.profile);
        let setting = self.settings.mail.remote_images;
        let Some(mail) = &mut self.mail else { return };
        let Some(reader) = &mut mail.reader else { return };
        reader.shown = id;
        reader.target.clear();
        reader.save_anyway = None;
        reader.body = mail.mail.body(id);
        let from = reader.message().map(|m| m.from.email.clone()).unwrap_or_default();
        let remote_images = match setting {
            RemoteImages::Never => false,
            RemoteImages::Always => true,
            RemoteImages::Ask => remote_images || mail.mail.trusts_images(&from),
        };
        reader.remote_images = remote_images;
        reader.page = None;
        reader._page = None;
        if let (Some(body), Some(profile)) = (&reader.body, profile) {
            let page = self.host.create_document(profile, DocumentKind::Message { remote_images }, window, cx);
            page.read(cx).set_document(&body.html);
            let sub = cx.subscribe_in(&page, window, |this, _, event, window, cx| this.on_message_page_event(event, window, cx));
            if let Some(reader) = self.mail.as_mut().and_then(|m| m.reader.as_mut()) {
                reader.page = Some(page);
                reader._page = Some(sub);
            }
        }
        self.show_mail_documents(cx);
        cx.notify();
    }

    /// Mail changed: the conversation may have grown, and a body asked for may be here.
    pub(crate) fn refresh_reader(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mail) = &mut self.mail else { return };
        let Some(reader) = &mut mail.reader else { return };
        let messages = mail.mail.conversation(reader.thread);
        if messages.is_empty() {
            mail.reader = None;
            mail.state.selected = None;
            mail.save_view();
            return self.show_mail_documents(cx);
        }
        reader.messages = messages;
        if reader.body.is_none() && mail.mail.body(reader.shown).is_some() {
            let (shown, remote) = (reader.shown, reader.remote_images);
            self.show_message(shown, remote, window, cx);
        }
    }

    /// From the message's page. A link opens only through the engine's own new tab, adopted
    /// as any page's open request is: baa never loads a URL that came from mail.
    fn on_message_page_event(&mut self, event: &PageEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self.session.active_space() else { return };
        match event {
            PageEvent::OpenRequested { url, background: true, .. } => {
                self.open_background_tab(space, url);
                self.cue_new_tab(cx);
                self.sync(window, cx);
            }
            PageEvent::OpenRequested { url, .. } => self.open_supplied_in(space, url, window, cx),
            PageEvent::TargetUrl(url) => {
                if let Some(reader) = self.mail.as_mut().and_then(|m| m.reader.as_mut()) {
                    reader.target = url.clone();
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    /// "Load images": this message only, in a new document.
    pub(crate) fn load_images(&mut self, always: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(reader) = self.mail.as_ref().and_then(|m| m.reader.as_ref()) else { return };
        let shown = reader.shown;
        if always {
            if let (Some(mail), Some(from)) = (&self.mail, reader.message().map(|m| m.from.email.clone())) {
                mail.mail.trust_images(&from);
            }
        }
        self.show_message(shown, true, window, cx);
    }

    /// The attachment goes to the download folder and into the Library as a download does.
    fn save_attachment(&mut self, part: usize, cx: &mut Context<Self>) {
        let Some(mail) = &mut self.mail else { return };
        let Some(reader) = &mut mail.reader else { return };
        let Some(att) = reader.body.as_ref().and_then(|b| b.attachments.iter().find(|a| a.part == part)).cloned() else { return };
        if att.dangerous && reader.save_anyway != Some(part) {
            reader.save_anyway = Some(part);
            return cx.notify();
        }
        reader.save_anyway = None;
        let dir = self.settings.web.download_dir.clone().or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Downloads")));
        let Some(dir) = dir else { return };
        match mail.mail.save_attachment(reader.shown, part, &dir) {
            Ok(path) => {
                let record = DownloadRecord { path: path.display().to_string(), url: String::new(), bytes: att.size as i64, finished_at: now() as i64 };
                if let Some(Err(e)) = self.store().map(|s| s.record_download(&record)) {
                    log::error!("saving download: {e}");
                }
                self.saved_downloads.retain(|s| s.path != record.path);
                self.saved_downloads.insert(0, record);
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.show_toast(format!("Saved {name} to Downloads"), cx);
            }
            Err(e) => self.show_toast(e.to_string(), cx),
        }
    }

    pub(crate) fn reply_to_selected(&mut self, all: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mail) = &self.mail else { return };
        let Some(draft) = mail.reader.as_ref().and_then(|r| mail.mail.reply(r.shown, all)) else { return };
        self.open_draft(draft, window, cx);
    }

    pub(crate) fn forward_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mail) = &self.mail else { return };
        let Some(draft) = mail.reader.as_ref().and_then(|r| mail.mail.forward(r.shown)) else { return };
        self.open_draft(draft, window, cx);
    }

    /// ⌫ and the command row's Delete, Archive, Move, Flag and Read act on the whole conversation.
    pub(crate) fn act_on_selected(&mut self, action: Action, cx: &mut Context<Self>) {
        let Some(mail) = &mut self.mail else { return };
        let Some(reader) = &mail.reader else { return };
        let ids: Vec<MessageId> = reader.messages.iter().map(|m| m.id).collect();
        mail.mail.act(&ids, action);
        if matches!(action, Action::Read(_) | Action::Flag(_)) {
            let thread = reader.thread;
            if let Some(reader) = &mut mail.reader {
                reader.messages = mail.mail.conversation(thread);
            }
        } else {
            mail.reader = None;
            mail.state.selected = None;
            mail.save_view();
            self.show_mail_documents(cx);
        }
        cx.notify();
    }

    pub(crate) fn delete_selected(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.act_on_selected(Action::Delete, cx);
    }

    pub(crate) fn render_reader(&self, _theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(mail) = &self.mail else { return div().into_any_element() };
        let Some(reader) = &mail.reader else {
            let note = if ascii() { "-- no message selected --" } else { "No message selected" };
            return div().size_full().flex().items_center().justify_center().text_color(surface::muted()).child(note).into_any_element();
        };
        let Some(message) = reader.message() else { return div().into_any_element() };
        let accounts = self.mail_space().map(|(m, s)| m.accounts(s)).unwrap_or_default();
        let dot = accounts.iter().position(|a| a.id == message.account).map(crate::mail::list::account_colour);
        let date = crate::mail::list::when(message.date);
        let subject = if message.subject.is_empty() { "(no subject)".to_string() } else { message.subject.clone() };
        let count = reader.messages.len();
        let header = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(20.))
            .pt(px(16.))
            .pb(px(12.))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(if ascii() { 15. } else { 19. })).font_weight(gpui::FontWeight::SEMIBOLD).child(subject))
                    .when(count > 1, |d| d.child(div().text_size(px(11.)).text_color(surface::muted()).child(format!("{count} messages")))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .children(dot.map(|c| div().size(px(8.)).rounded_full().bg(c)))
                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(sender(&message.from)))
                    // The real address always: a display name proves nothing.
                    .child(div().debug_selector(|| "mail-sender-address".into()).text_color(surface::muted()).child(message.from.email.clone()))
                    .child(div().flex_1())
                    .child(div().text_size(px(11.5)).text_color(surface::muted()).child(date)),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(surface::muted())
                    .truncate()
                    .child(format!("to {}{}", people(&message.to), if message.cc.is_empty() { String::new() } else { format!(" · cc {}", people(&message.cc)) })),
            );
        let banner = reader.body.as_ref().filter(|b| b.has_remote_images && !reader.remote_images && self.settings.mail.remote_images == RemoteImages::Ask).map(|_| {
            let from = message.from.email.clone();
            div()
                .debug_selector(|| "mail-images-banner".into())
                .mx(px(20.))
                .mb(px(8.))
                .px(px(10.))
                .py(px(6.))
                .flex()
                .items_center()
                .gap(px(10.))
                .text_size(px(12.))
                .bg(surface::key())
                .when(ascii(), |d| d.border_1().border_color(surface::border()))
                .when(!ascii(), |d| d.rounded(r(8.)))
                .child(div().flex_1().text_color(surface::muted()).child("Remote images are off, so the sender cannot see that you opened this."))
                .child(self.mail_button("mail-load-images", 0, false, cx).child("Load images").on_click(cx.listener(|this, _, window, cx| this.load_images(false, window, cx))))
                .child(self.mail_button("mail-trust-images", 0, false, cx).child("Always for this sender").on_click(cx.listener(|this, _, window, cx| this.load_images(true, window, cx))))
                .child(div().text_size(px(11.)).text_color(surface::muted()).truncate().max_w(px(180.)).child(from))
        });
        let content: AnyElement = match &reader.page {
            Some(page) => div().flex_1().min_h_0().mx(px(20.)).child(page.update(cx, |p, cx| p.render(window, cx))).into_any_element(),
            None => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .debug_selector(|| "mail-loading".into())
                .text_color(surface::muted())
                .child(if ascii() { "loading message…" } else { "Loading message…" })
                .into_any_element(),
        };
        let attachments = reader.body.as_ref().map(|b| b.attachments.as_slice()).unwrap_or_default();
        let attachments = (!attachments.is_empty()).then(|| {
            div().px(px(20.)).pt(px(10.)).flex().flex_wrap().gap(px(8.)).children(attachments.iter().map(|a| self.render_attachment(a, reader.save_anyway == Some(a.part), cx)))
        });
        let earlier = (count > 1).then(|| self.render_earlier(reader, cx));
        let target = (!reader.target.is_empty()).then(|| {
            div()
                .debug_selector(|| "mail-link-target".into())
                .px(px(20.))
                .py(px(4.))
                .text_size(px(11.))
                .text_color(surface::muted())
                .truncate()
                .child(format!("→ {}", reader.target))
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .text_color(surface::ink())
            .child(header)
            .children(banner)
            .child(content)
            .children(target)
            .children(attachments)
            .children(earlier)
            .child(div().h(px(12.)))
            .into_any_element()
    }

    fn render_attachment(&self, a: &Attachment, armed: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let part = a.part;
        let warn = gpui::Hsla::from(surface::warn());
        div()
            .id(("mail-attachment", part as u64))
            .px(px(10.))
            .py(px(6.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_1()
            .border_color(if a.dangerous { warn } else { surface::border().into() })
            .when(!ascii(), |d| d.rounded(r(8.)))
            .child(icon(if a.dangerous { "triangle-alert" } else { "paperclip" }, 14., if a.dangerous { warn } else { surface::icon().into() }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(div().max_w(px(220.)).truncate().child(SharedString::from(a.name.clone())))
                    .child(div().text_size(px(10.5)).text_color(if a.dangerous { warn } else { surface::muted().into() }).child(if a.dangerous {
                        "This file may harm your computer".to_string()
                    } else {
                        crate::prompts::bytes(a.size as i64)
                    })),
            )
            .child(
                self.mail_button("mail-attachment-save", part as u64, false, cx).child(if armed { "Save anyway" } else { "Save" })
                    .on_click(cx.listener(move |this, _, _, cx| this.save_attachment(part, cx))),
            )
    }

    fn render_earlier(&self, reader: &Reader<H>, cx: &mut Context<Self>) -> impl IntoElement {
        let n = reader.messages.len() - 1;
        let open = reader.earlier_open;
        let label = format!("{} {n} earlier message{} in this conversation", if open { "▾" } else { "▸" }, if n == 1 { "" } else { "s" });
        let rows = open.then(|| {
            reader.messages.iter().filter(|m| m.id != reader.shown).map(|m| {
                let id = m.id;
                div()
                    .id(("mail-earlier", id.0))
                    .px(px(8.))
                    .py(px(4.))
                    .flex()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .cursor_pointer()
                    .hover(|d| d.bg(surface::key()))
                    .on_click(cx.listener(move |this, _, window, cx| this.show_message(id, false, window, cx)))
                    .child(div().w(px(160.)).truncate().child(sender(&m.from)))
                    .child(div().flex_1().truncate().text_color(surface::muted()).child(m.subject.clone()))
                    .child(div().text_color(surface::muted()).child(crate::mail::list::when(m.date)))
            })
        });
        div()
            .mx(px(20.))
            .mt(px(10.))
            .flex()
            .flex_col()
            .child(
                div()
                    .id("mail-earlier-toggle")
                    .py(px(6.))
                    .text_size(px(12.))
                    .text_color(surface::muted())
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(reader) = this.mail.as_mut().and_then(|m| m.reader.as_mut()) {
                            reader.earlier_open = !reader.earlier_open;
                        }
                        cx.notify();
                    }))
                    .child(label),
            )
            .children(rows.into_iter().flatten())
    }
}
