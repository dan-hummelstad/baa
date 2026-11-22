//! The command row over the list and reading pane: the ribbon's Home set in ASCII, pills in
//! Arc, from one table (mocks A2/B2; WP-I). The ribbon's other tabs are stage 3.

use baa_mail::{Action, Role};
use baa_web::WebHost;
use gpui::{div, prelude::*, px, AnyElement, Context, Window};

use crate::{
    browser::{menus::MenuEntry, Browser, MenuKind},
    icons::icon,
    theme::{ascii, r, surface, Theme},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Command {
    Delete,
    Archive,
    Move,
    Reply,
    ReplyAll,
    Forward,
    Flag,
    Read,
}

/// (command, icon, Arc label, ASCII label). Flag and Read name what a click does.
const COMMANDS: [(Command, &str, &str, &str); 8] = [
    (Command::Delete, "trash-2", "Delete", "⌫ Delete"),
    (Command::Archive, "archive", "Archive", "▤ Archive"),
    (Command::Move, "folder", "Move to", "→ Move"),
    (Command::Reply, "arrow-left", "Reply", "↩ Reply"),
    (Command::ReplyAll, "arrow-left", "Reply all", "⇇ All"),
    (Command::Forward, "arrow-right", "Forward", "↪ Forward"),
    (Command::Flag, "flag", "Flag", "⚑ Flag"),
    (Command::Read, "mail-open", "Mark read", "✉ Read"),
];

impl<H: WebHost> Browser<H> {
    pub(crate) fn run_mail_command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        let Some(message) = self.mail.as_ref().and_then(|m| m.reader.as_ref()).and_then(|r| r.message().cloned()) else { return };
        match command {
            Command::Delete => self.act_on_selected(Action::Delete, cx),
            Command::Archive => self.act_on_selected(Action::Archive, cx),
            Command::Move => self.open_menu(MenuKind::MailMove, window.mouse_position(), window, cx),
            Command::Reply => self.reply_to_selected(false, window, cx),
            Command::ReplyAll => self.reply_to_selected(true, window, cx),
            Command::Forward => self.forward_selected(window, cx),
            Command::Flag => self.act_on_selected(Action::Flag(!message.flagged), cx),
            Command::Read => self.act_on_selected(Action::Read(message.unread), cx),
        }
    }

    /// Move: the other folders of the message's own account.
    pub(crate) fn mail_move_menu(&self) -> Vec<MenuEntry<H>> {
        let Some(message) = self.mail.as_ref().and_then(|m| m.reader.as_ref()).and_then(|r| r.message().cloned()) else { return Vec::new() };
        let Some((mail, space)) = self.mail_space() else { return Vec::new() };
        mail.folders(space)
            .into_iter()
            .filter(|f| f.account == message.account && f.role != Role::Drafts)
            .map(|f| {
                let id = f.id;
                MenuEntry::Item { label: f.name, checked: false, enabled: true, run: Box::new(move |this: &mut Browser<H>, _: &mut Window, cx: &mut Context<Browser<H>>| this.act_on_selected(Action::Move(id), cx)) }
            })
            .collect()
    }

    pub(crate) fn render_commands(&self, _theme: &Theme, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let message = self.mail.as_ref().and_then(|m| m.reader.as_ref()).and_then(|r| r.message());
        let on = message.is_some();
        let (flagged, unread) = message.map_or((false, false), |m| (m.flagged, m.unread));
        let items = COMMANDS.iter().map(|&(command, glyph, label, ascii_label)| {
            let label = match command {
                Command::Flag if flagged => if ascii() { "⚑ Unflag" } else { "Unflag" },
                Command::Read if !unread => if ascii() { "✉ Unread" } else { "Mark unread" },
                _ if ascii() => ascii_label,
                _ => label,
            };
            let ink = if on { surface::ink() } else { surface::muted() };
            div()
                .id(("mail-command", command as u64))
                .occlude()
                .h(px(26.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(12.))
                .text_color(ink)
                .when(on, |d| d.cursor_pointer().hover(|d| d.bg(surface::key())))
                .when(!ascii(), |d| d.rounded(r(7.)).child(icon(glyph, 13., ink.into())))
                .when(on, |d| d.on_click(cx.listener(move |this, _, window, cx| this.run_mail_command(command, window, cx))))
                .child(label)
        });
        let row = div().flex().items_center().gap(px(2.)).px(px(10.)).h(px(38.)).border_b_1().border_color(surface::line());
        let row = if ascii() {
            row.child(div().px(px(6.)).mr(px(6.)).text_size(px(11.)).text_color(surface::muted()).child("HOME"))
        } else {
            row
        };
        row.debug_selector(|| "mail-commands".into()).children(items).into_any_element()
    }
}
