//! A draft in the reading pane (mocks A3/B3; WP-J).

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use baa_mail::{parse_addresses, Account, AccountId, Address, Draft, DraftId, MessageId};
use baa_web::{DocumentKind, PageHandle, WebHost, EDITOR_DOCUMENT};
use gpui::{div, prelude::*, px, AnyElement, Context, Div, ElementId, Entity, Hsla, PathPromptOptions, Window};

use super::Field;
use crate::{
    browser::{menus::MenuEntry, Browser, MenuKind},
    icons::icon,
    input::{Key, TextField},
    theme::{ascii, r, surface, Theme},
};

const AUTOSAVE: Duration = Duration::from_secs(2);

/// One open draft: a tab of the strip.
pub(crate) struct Composer<H: WebHost> {
    pub(crate) id: DraftId,
    /// The body's editor document.
    pub(crate) page: Option<Entity<H::Page>>,
    pub(crate) to: TextField,
    pub(crate) cc: TextField,
    pub(crate) bcc: TextField,
    pub(crate) subject: TextField,
    pub(crate) from: AccountId,
    /// The committed recipients of To, Cc and Bcc.
    chips: [Vec<Address>; 3],
    /// Each file with its size when it was attached.
    attachments: Vec<(PathBuf, u64)>,
    in_reply_to: Option<MessageId>,
    /// The body as last read from the editor.
    html: String,
    text: String,
    /// Changed since the last save: the tab's dot.
    pub(crate) dirty: bool,
    saved_at: Option<Instant>,
    /// Counts edits, so only the last one's timer saves.
    edits: u64,
    show_cc: bool,
    ask_close: bool,
    pub(crate) error: Option<&'static str>,
}

impl<H: WebHost> Composer<H> {
    fn new(id: DraftId, draft: &Draft, page: Entity<H::Page>) -> Self {
        let field = |text: &str| TextField { text: text.to_owned(), ..Default::default() };
        Composer {
            id,
            page: Some(page),
            to: field(""),
            cc: field(""),
            bcc: field(""),
            subject: field(&draft.subject),
            from: draft.from,
            chips: [draft.to.clone(), draft.cc.clone(), draft.bcc.clone()],
            attachments: draft.attachments.iter().map(|p| (p.clone(), std::fs::metadata(p).map_or(0, |m| m.len()))).collect(),
            in_reply_to: draft.in_reply_to,
            html: draft.html.clone(),
            text: draft.text.clone(),
            dirty: false,
            saved_at: None,
            edits: 0,
            show_cc: false,
            ask_close: false,
            error: None,
        }
    }

    pub(crate) fn field(&self, field: Field) -> Option<&TextField> {
        match field {
            Field::To => Some(&self.to),
            Field::Cc => Some(&self.cc),
            Field::Bcc => Some(&self.bcc),
            Field::Subject => Some(&self.subject),
            Field::Search => None,
        }
    }

    pub(crate) fn field_mut(&mut self, field: Field) -> Option<&mut TextField> {
        match field {
            Field::To => Some(&mut self.to),
            Field::Cc => Some(&mut self.cc),
            Field::Bcc => Some(&mut self.bcc),
            Field::Subject => Some(&mut self.subject),
            Field::Search => None,
        }
    }

    pub(crate) fn chips(&self, field: Field) -> &[Address] {
        match field {
            Field::To => &self.chips[0],
            Field::Cc => &self.chips[1],
            Field::Bcc => &self.chips[2],
            _ => &[],
        }
    }

    fn chips_mut(&mut self, field: Field) -> Option<&mut Vec<Address>> {
        match field {
            Field::To => Some(&mut self.chips[0]),
            Field::Cc => Some(&mut self.chips[1]),
            Field::Bcc => Some(&mut self.chips[2]),
            _ => None,
        }
    }

    /// The recipients of `field`: the chips and what its text parses to.
    fn recipients(&self, field: Field) -> Vec<Address> {
        let mut all = self.chips(field).to_vec();
        all.extend(self.field(field).map(|f| split_addresses(&f.text).0).unwrap_or_default());
        all
    }

    fn cc_open(&self) -> bool {
        self.show_cc || !self.cc.text.is_empty() || !self.chips[1].is_empty() || !self.bcc.text.is_empty() || !self.chips[2].is_empty()
    }

    fn fields(&self) -> Vec<Field> {
        let mut fields = vec![Field::To];
        if self.cc_open() {
            fields.extend([Field::Cc, Field::Bcc]);
        }
        fields.push(Field::Subject);
        fields
    }

    fn draft(&self) -> Draft {
        Draft {
            id: Some(self.id),
            from: self.from,
            to: self.recipients(Field::To),
            cc: self.recipients(Field::Cc),
            bcc: self.recipients(Field::Bcc),
            subject: self.subject.text.clone(),
            html: self.html.clone(),
            text: self.text.clone(),
            attachments: self.attachments.iter().map(|(p, _)| p.clone()).collect(),
            in_reply_to: self.in_reply_to,
        }
    }
}

/// The addresses in `text`, and what is left of it. Entries are cut at `,` and `;`; text
/// with a quoted name is parsed whole, since a comma may be inside the quotes.
fn split_addresses(text: &str) -> (Vec<Address>, String) {
    if text.contains('"') {
        let found = parse_addresses(text);
        let rest = if found.is_empty() { text.to_owned() } else { String::new() };
        return (found, rest);
    }
    let (mut found, mut rest) = (Vec::new(), Vec::new());
    for piece in text.split([',', ';']) {
        let parsed = parse_addresses(piece);
        if !parsed.is_empty() {
            found.extend(parsed);
        } else if !piece.trim().is_empty() {
            rest.push(piece.trim());
        }
    }
    (found, rest.join(", "))
}

fn account_label(account: &Account) -> String {
    if account.name.is_empty() { account.address.clone() } else { format!("{} <{}>", account.name, account.address) }
}

fn saved_label(at: Option<Instant>) -> String {
    match at.map(|t| t.elapsed().as_secs()) {
        None => "saved".into(),
        Some(s @ 0..60) => format!("saved {s}s ago"),
        Some(s) => format!("saved {} min ago", s / 60),
    }
}

/// The script for Link: the selection becomes a link to its own text when that is an
/// http(s) URL, else nothing happens.
fn link_script() -> String {
    format!(
        "(() => {{ const d = {EDITOR_DOCUMENT}; try {{ const u = new URL(d.getSelection().toString().trim()); \
         if (u.protocol === 'http:' || u.protocol === 'https:') d.execCommand('createLink', false, u.href); }} catch (e) {{}} }})()"
    )
}

const BODY_SCRIPT: &str = "(() => { const b = frames[0].document.body; return { html: b.innerHTML, text: b.innerText }; })()";

impl<H: WebHost> Browser<H> {
    fn composer(&self, id: DraftId) -> Option<&Composer<H>> {
        self.mail.as_ref()?.drafts.iter().find(|d| d.id == id)
    }

    fn composer_mut(&mut self, id: DraftId) -> Option<&mut Composer<H>> {
        self.mail.as_mut()?.drafts.iter_mut().find(|d| d.id == id)
    }

    fn make_composer(&self, id: DraftId, draft: &Draft, window: &mut Window, cx: &mut Context<Self>) -> Option<Composer<H>> {
        let profile = self.active_space()?.profile;
        let page = self.host.create_document(profile, DocumentKind::Editor, window, cx);
        page.read(cx).set_document(&draft.html);
        Some(Composer::new(id, draft, page))
    }

    /// ⌘N: a new draft from the Space's first account.
    pub(crate) fn new_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((mail, space)) = self.mail_space() else { return };
        let Some(from) = mail.accounts(space).first().map(|a| a.id) else {
            return self.open_mail_settings(cx);
        };
        let draft = Draft {
            id: None,
            from,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: String::new(),
            html: String::new(),
            text: String::new(),
            attachments: Vec::new(),
            in_reply_to: None,
        };
        self.open_draft(draft, window, cx);
        self.focus_mail_field(Field::To, window, cx);
    }

    /// Opens `draft` (a new one, a reply, a forward) as a tab in front.
    pub(crate) fn open_draft(&mut self, draft: Draft, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mail) = self.mail.as_ref().map(|m| m.mail.clone()) else { return };
        let id = match draft.id {
            Some(id) => id,
            None => match mail.save_draft(&draft) {
                Ok(id) => id,
                Err(_) => return self.show_toast("Could not start the draft.", cx),
            },
        };
        if self.composer(id).is_none() {
            let Some(composer) = self.make_composer(id, &draft, window, cx) else { return };
            let Some(ui) = self.mail.as_mut() else { return };
            ui.drafts.push(composer);
            if !ui.state.drafts.contains(&id) {
                ui.state.drafts.push(id);
            }
        }
        self.show_front(Some(id), cx);
    }

    /// `front`: a draft's tab, or `None` for Inbox.
    pub(crate) fn show_front(&mut self, front: Option<DraftId>, cx: &mut Context<Self>) {
        let Some(ui) = self.mail.as_mut() else { return };
        ui.state.front = front;
        ui.save_view();
        self.show_mail_documents(cx);
        cx.notify();
    }

    /// The view state's open drafts, after a Space switch or at launch.
    pub(crate) fn restore_drafts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ui) = &self.mail else { return };
        let (mail, ids) = (ui.mail.clone(), ui.state.drafts.clone());
        let (mut open, mut gone) = (Vec::new(), Vec::new());
        for id in ids {
            match mail.draft(id).and_then(|draft| self.make_composer(id, &draft, window, cx)) {
                Some(composer) => open.push(composer),
                None => gone.push(id),
            }
        }
        let Some(ui) = self.mail.as_mut() else { return };
        ui.drafts = open;
        if !gone.is_empty() {
            ui.state.drafts.retain(|id| !gone.contains(id));
            if ui.state.front.is_some_and(|f| gone.contains(&f)) {
                ui.state.front = None;
            }
            ui.save_view();
        }
    }

    /// Removes the tab and its page; the draft stays where it was saved.
    fn close_composer(&mut self, id: DraftId, cx: &mut Context<Self>) {
        let Some(ui) = self.mail.as_mut() else { return };
        if let Some(i) = ui.drafts.iter().position(|d| d.id == id) {
            if let Some(page) = &ui.drafts[i].page {
                page.read(cx).set_visible(false);
            }
            ui.drafts.remove(i);
        }
        ui.state.drafts.retain(|d| *d != id);
        if ui.state.front == Some(id) {
            ui.state.front = None;
        }
        ui.save_view();
        self.show_mail_documents(cx);
        cx.notify();
    }

    /// A tab's × or the composer's close: asks first when there are unsaved changes.
    pub(crate) fn request_close_draft(&mut self, id: DraftId, cx: &mut Context<Self>) {
        match self.composer_mut(id) {
            Some(c) if c.dirty => {
                c.ask_close = true;
                self.show_front(Some(id), cx);
            }
            Some(_) => self.close_composer(id, cx),
            None => {}
        }
    }

    fn discard(&mut self, id: DraftId, cx: &mut Context<Self>) {
        if let Some(ui) = &self.mail {
            ui.mail.discard_draft(id);
        }
        self.close_composer(id, cx);
    }

    /// Reads the editor's body into the composer, then runs `then`. Skipped when the
    /// composer is gone by then.
    fn read_body(&mut self, id: DraftId, cx: &mut Context<Self>, then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static) {
        let Some(page) = self.composer(id).and_then(|c| c.page.clone()) else { return then(self, cx) };
        let this = cx.entity().downgrade();
        page.read(cx).eval_value(
            BODY_SCRIPT,
            Box::new(move |body, cx| {
                let _ = this.update(cx, |this, cx| {
                    let Some(c) = this.composer_mut(id) else { return };
                    if let Ok(body) = body {
                        if let (Some(html), Some(text)) = (body["html"].as_str(), body["text"].as_str()) {
                            c.html = html.to_owned();
                            c.text = text.to_owned();
                        }
                    }
                    then(this, cx);
                });
            }),
        );
    }

    fn store_draft(&mut self, id: DraftId, cx: &mut Context<Self>) -> bool {
        let Some(mail) = self.mail.as_ref().map(|m| m.mail.clone()) else { return false };
        let Some(draft) = self.composer(id).map(|c| c.draft()) else { return false };
        if mail.save_draft(&draft).is_err() {
            self.show_toast("Could not save the draft.", cx);
            return false;
        }
        if let Some(c) = self.composer_mut(id) {
            c.dirty = false;
            c.saved_at = Some(Instant::now());
        }
        cx.notify();
        true
    }

    fn save_now(&mut self, id: DraftId, cx: &mut Context<Self>) {
        self.read_body(id, cx, move |this, cx| {
            this.store_draft(id, cx);
        });
    }

    // ponytail: body edits are saved on the next field edit, on Save draft, on send and on
    // close, until a page-to-host channel exists: the editor's own typing is not reported to the shell.
    fn mark_unsaved(&mut self, id: DraftId, cx: &mut Context<Self>) {
        let Some(c) = self.composer_mut(id) else { return };
        c.dirty = true;
        c.edits += 1;
        let n = c.edits;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AUTOSAVE).await;
            let _ = this.update(cx, |this, cx| {
                if this.composer(id).is_some_and(|c| c.edits == n && c.dirty) {
                    this.save_now(id, cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn send_front_draft(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.mail.as_ref().and_then(|m| m.state.front) {
            self.send_draft(id, cx);
        }
    }

    fn send_draft(&mut self, id: DraftId, cx: &mut Context<Self>) {
        let Some(c) = self.composer_mut(id) else { return };
        if ![Field::To, Field::Cc, Field::Bcc].iter().any(|f| !c.recipients(*f).is_empty()) {
            c.error = Some("Add a recipient to send this message.");
            cx.notify();
            return;
        }
        self.read_body(id, cx, move |this, cx| {
            if !this.store_draft(id, cx) {
                return;
            }
            if let Some(ui) = &this.mail {
                ui.mail.send(id);
            }
            this.close_composer(id, cx);
            this.show_toast("Sending…", cx);
        });
    }

    pub(crate) fn on_sent(&mut self, draft: DraftId, cx: &mut Context<Self>) {
        self.close_composer(draft, cx);
        self.show_toast("Sent", cx);
    }

    /// Moves what parses in `field`'s text to its chips; true when something moved.
    fn commit_field(&mut self, id: DraftId, field: Field) -> bool {
        let Some(c) = self.composer_mut(id) else { return false };
        let Some(text) = c.field(field).map(|f| f.text.clone()) else { return false };
        let (found, rest) = split_addresses(&text);
        if found.is_empty() {
            return false;
        }
        let Some(chips) = c.chips_mut(field) else { return false };
        for a in found {
            if !chips.iter().any(|c| c.email.eq_ignore_ascii_case(&a.email)) {
                chips.push(a);
            }
        }
        if let Some(f) = c.field_mut(field) {
            f.text = rest;
        }
        true
    }

    fn commit_all(&mut self, id: DraftId) -> bool {
        [Field::To, Field::Cc, Field::Bcc].into_iter().fold(false, |any, f| self.commit_field(id, f) | any)
    }

    fn remove_chip(&mut self, id: DraftId, field: Field, i: usize, cx: &mut Context<Self>) {
        if let Some(chips) = self.composer_mut(id).and_then(|c| c.chips_mut(field)) {
            if i < chips.len() {
                chips.remove(i);
                self.mark_unsaved(id, cx);
            }
        }
    }

    pub(crate) fn on_compose_key(&mut self, field: Field, key: Key, pressed: &str, before: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.mail.as_ref().and_then(|m| m.state.front) else { return };
        let recipients = field != Field::Subject;
        match key {
            Key::Edited => {
                let Some(c) = self.composer_mut(id) else { return };
                c.error = None;
                let text = c.field(field).map(|f| f.text.clone()).unwrap_or_default();
                if recipients && pressed == "backspace" && before.is_empty() {
                    if let Some(chips) = c.chips_mut(field) {
                        chips.pop();
                    }
                } else if recipients && text.contains([',', ';']) {
                    self.commit_field(id, field);
                }
                self.mark_unsaved(id, cx);
            }
            Key::Submit | Key::Complete if recipients => {
                if self.commit_field(id, field) {
                    self.mark_unsaved(id, cx);
                }
                let empty = self.composer(id).and_then(|c| c.field(field)).is_none_or(|f| f.text.is_empty());
                if key == Key::Complete && empty {
                    let next = self.composer(id).and_then(|c| {
                        let fields = c.fields();
                        fields.iter().position(|f| *f == field).and_then(|i| fields.get(i + 1).copied())
                    });
                    if let Some(next) = next {
                        self.focus_mail_field(next, window, cx);
                    }
                }
            }
            Key::Submit | Key::Complete => self.leave_mail_field(window, cx),
            Key::Cancel => {
                if self.commit_all(id) {
                    self.mark_unsaved(id, cx);
                }
                self.leave_mail_field(window, cx);
            }
            _ => {}
        }
    }

    pub(crate) fn open_from_menu(&mut self, id: DraftId, window: &mut Window, cx: &mut Context<Self>) {
        self.open_menu(MenuKind::MailFrom(id), window.mouse_position(), window, cx);
    }

    /// From: the Space's own accounts and those of every Space, never another Space's.
    pub(crate) fn mail_from_menu(&self, id: DraftId) -> Vec<MenuEntry<H>> {
        let (Some((mail, space)), Some(current)) = (self.mail_space(), self.composer(id).map(|c| c.from)) else { return Vec::new() };
        mail.accounts(space)
            .iter()
            .map(|a| {
                let from = a.id;
                MenuEntry::Item {
                    label: account_label(a),
                    checked: from == current,
                    enabled: true,
                    run: Box::new(move |this: &mut Browser<H>, _: &mut Window, cx: &mut Context<Browser<H>>| {
                        if let Some(c) = this.composer_mut(id) {
                            c.from = from;
                        }
                        this.mark_unsaved(id, cx);
                    }),
                }
            })
            .collect()
    }

    fn attach_files(&mut self, id: DraftId, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Attach".into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                let _ = this.update(cx, |this, cx| this.add_attachments(id, paths, cx));
            }
        })
        .detach();
    }

    pub(crate) fn add_attachments(&mut self, id: DraftId, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(c) = self.composer_mut(id) else { return };
        for path in paths {
            if let Some(meta) = std::fs::metadata(&path).ok().filter(|m| m.is_file()) {
                if !c.attachments.iter().any(|(p, _)| *p == path) {
                    c.attachments.push((path, meta.len()));
                }
            }
        }
        self.mark_unsaved(id, cx);
    }

    fn format(&mut self, id: DraftId, script: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.composer(id).and_then(|c| c.page.clone()) else { return };
        page.read(cx).eval(&script);
        if self.active_dialog().is_none() {
            window.focus(&page.read(cx).focus_handle(cx), cx);
        }
        self.mark_unsaved(id, cx);
    }


    fn chip(&self, id: impl Into<ElementId>, label: String, dim: Hsla, on_remove: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(5.))
            .h(px(22.))
            .px(px(8.))
            .text_size(px(12.))
            .max_w(px(280.))
            .overflow_hidden()
            .whitespace_nowrap()
            .when(ascii(), |d| d.border_1().border_color(surface::border()))
            .when(!ascii(), |d| d.rounded(r(11.)).bg(surface::key()))
            .child(label)
            .child(div().id(id).cursor_pointer().text_color(dim).on_click(cx.listener(move |this, _, window, cx| on_remove(this, window, cx))).child("×"))
    }

    pub(crate) fn render_composer(&self, _theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(c) = self.mail.as_ref().and_then(|m| m.front_draft()) else { return div().into_any_element() };
        let id = c.id;
        let (ink, dim): (Hsla, Hsla) = (surface::ink().into(), surface::muted().into());
        let accounts = self.mail_space().map(|(m, s)| m.accounts(s)).unwrap_or_default();
        let from = accounts.iter().find(|a| a.id == c.from).map_or_else(|| "—".to_owned(), account_label);
        let space_name = self.active_space().map(|s| s.name.clone()).unwrap_or_default();
        let subject = if c.subject.text.trim().is_empty() { "New message".to_owned() } else { c.subject.text.trim().to_owned() };
        let label = |text: &'static str| {
            div().w(px(52.)).flex_none().text_size(px(12.)).text_color(dim).child(if ascii() { text.to_uppercase() } else { text.to_owned() })
        };
        let row = || div().flex().items_start().gap(px(8.)).px(px(16.)).py(px(5.)).border_b_1().border_color(surface::line());

        let title = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(16.))
            .py(px(10.))
            .border_b_1()
            .border_color(surface::line())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(15.)).font_weight(gpui::FontWeight::SEMIBOLD).overflow_hidden().whitespace_nowrap().child(subject))
                    .child(div().text_size(px(11.)).text_color(dim).child(format!("draft · {space_name}"))),
            )
            .child(
                self.mail_button("mail-compose-close", 0, false, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.request_close_draft(id, cx)))
                    .child(icon("x", 12., surface::icon().into())),
            );

        let mut format = div().flex().items_center().gap(px(4.)).px(px(16.)).py(px(6.)).border_b_1().border_color(surface::line());
        let commands = [("B", "bold"), ("I", "italic"), ("U", "underline"), ("•", "insertUnorderedList"), ("1.", "insertOrderedList")];
        for (n, (text, command)) in commands.into_iter().enumerate() {
            let script = format!("{EDITOR_DOCUMENT}.execCommand('{command}')");
            format = format.child(
                self.mail_button("mail-format", n as u64, false, cx)
                    .on_click(cx.listener(move |this, _, window, cx| this.format(id, script.clone(), window, cx)))
                    .child(text),
            );
        }
        // ponytail: the toolbar does not show the selection's state — the ceiling is a page-to-host channel, which ABI 17 could carry
        format = format
            .child(
                self.mail_button("mail-format", 5, false, cx)
                    .on_click(cx.listener(move |this, _, window, cx| this.format(id, link_script(), window, cx)))
                    .child(if ascii() { "↗Link".into_any_element() } else { icon("link", 12., surface::icon().into()).into_any_element() })
                    .when(!ascii(), |d| d.child("Link")),
            )
            .child(div().flex_1())
            .child(
                self.mail_button("mail-attach", 0, false, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.attach_files(id, cx)))
                    .child(icon("paperclip", 12., surface::icon().into())),
            );
        let format = if ascii() {
            div().child(div().px(px(16.)).pt(px(6.)).child(ascii::rule_label("MESSAGE", dim))).child(format)
        } else {
            div().child(format)
        };

        let from_row = row().child(label("From")).child(
            div()
                .id("mail-from")
                .flex_1()
                .min_w_0()
                .cursor_pointer()
                .text_size(px(13.))
                .overflow_hidden()
                .whitespace_nowrap()
                .on_click(cx.listener(move |this, _, window, cx| this.open_from_menu(id, window, cx)))
                .child(format!("{from}  ▾")),
        );

        let recipient_row = |field: Field, name: &'static str, cx: &mut Context<Self>| {
            let chips = c.chips(field).iter().enumerate().map(|(i, a)| {
                let key = ((field as u64) << 8) | i as u64;
                self.chip(("mail-chip-x", key), a.email.clone(), dim, move |this, _, cx| this.remove_chip(id, field, i, cx), cx)
            }).collect::<Vec<_>>();
            let more = (field == Field::To && !c.cc_open()).then(|| {
                div()
                    .id("mail-cc-toggle")
                    .flex_none()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(dim)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(c) = this.composer_mut(id) {
                            c.show_cc = true;
                        }
                        cx.notify();
                    }))
                    .child("Cc Bcc")
            });
            row()
                .capture_any_mouse_down(cx.listener(move |this, _, _, cx| {
                    let others = [Field::To, Field::Cc, Field::Bcc].into_iter().filter(|f| *f != field);
                    if others.fold(false, |any, f| this.commit_field(id, f) | any) {
                        this.mark_unsaved(id, cx);
                    }
                }))
                .child(label(name))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(4.))
                        .children(chips)
                        .child(div().flex().flex_1().min_w(px(140.)).child(self.render_mail_field(field, "Recipients", ink, dim, window, cx))),
                )
                .children(more)
        };
        let to_row = recipient_row(Field::To, "To", cx);
        let cc_rows = c.cc_open().then(|| div().child(recipient_row(Field::Cc, "Cc", cx)).child(recipient_row(Field::Bcc, "Bcc", cx)));
        let subject_row = row().child(label("Subject")).child(self.render_mail_field(Field::Subject, "Subject", ink, dim, window, cx));

        let attachments = (!c.attachments.is_empty()).then(|| {
            let chips = c.attachments.iter().enumerate().map(|(i, (path, size))| {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.chip(
                    ("mail-attachment-x", i as u64),
                    format!("{name}  {}", crate::prompts::bytes(*size as i64)),
                    dim,
                    move |this, _, cx| {
                        if let Some(c) = this.composer_mut(id) {
                            if i < c.attachments.len() {
                                c.attachments.remove(i);
                            }
                        }
                        this.mark_unsaved(id, cx);
                    },
                    cx,
                )
            }).collect::<Vec<_>>();
            row().child(label("Attach")).child(div().flex_1().flex().flex_wrap().gap(px(4.)).children(chips))
        });

        let body = div().flex_1().min_h_0().children(c.page.as_ref().map(|page| page.update(cx, |p, cx| p.render(window, cx))));

        let notice = c.error.map(|text| div().px(px(16.)).py(px(6.)).text_size(px(12.)).text_color(surface::warn()).child(text));
        let ask = c.ask_close.then(|| {
            div()
                .id("mail-close-ask")
                .debug_selector(|| "mail-close-ask".into())
                .occlude()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(16.))
                .py(px(8.))
                .border_t_1()
                .border_color(surface::line())
                .bg(surface::key())
                .child(div().flex_1().text_size(px(12.)).child("Save changes to this draft?"))
                .child(self.mail_button("mail-ask-save", 0, true, cx).on_click(cx.listener(move |this, _, _, cx| {
                    this.read_body(id, cx, move |this, cx| {
                        if this.store_draft(id, cx) {
                            this.close_composer(id, cx);
                        }
                    });
                })).child("Save"))
                .child(self.mail_button("mail-ask-drop", 0, false, cx).on_click(cx.listener(move |this, _, _, cx| this.discard(id, cx))).child("Don't save"))
                .child(self.mail_button("mail-ask-cancel", 0, false, cx).on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(c) = this.composer_mut(id) {
                        c.ask_close = false;
                    }
                    cx.notify();
                })).child("Cancel"))
        });

        let status = if c.dirty { "unsaved".to_owned() } else { saved_label(c.saved_at) };
        let footer = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(16.))
            .py(px(10.))
            .border_t_1()
            .border_color(surface::line())
            .child(
                self.mail_button("mail-send", 0, true, cx)
                    .debug_selector(|| "mail-send".into())
                    .on_click(cx.listener(move |this, _, _, cx| this.send_draft(id, cx)))
                    .child("Send")
                    .child(div().text_color(dim).child("⌘↩")),
            )
            .child(self.mail_button("mail-save", 0, false, cx).on_click(cx.listener(move |this, _, _, cx| this.save_now(id, cx))).child("Save draft"))
            .child(self.mail_button("mail-discard", 0, false, cx).on_click(cx.listener(move |this, _, _, cx| this.discard(id, cx))).child("Discard"))
            .child(div().flex_1())
            .child(div().text_size(px(12.)).text_color(dim).child(status));

        div()
            .id(("mail-composer", id.0))
            .debug_selector(|| "mail-composer".into())
            .size_full()
            .flex()
            .flex_col()
            .bg(surface::bg())
            .text_color(ink)
            .child(title)
            .child(format)
            .child(from_row)
            .child(to_row)
            .children(cc_rows)
            .child(subject_row)
            .children(attachments)
            .child(body)
            .children(notice)
            .children(ask)
            .child(footer)
            .into_any_element()
    }
}
