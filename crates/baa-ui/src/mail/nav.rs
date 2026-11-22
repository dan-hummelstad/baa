//! The mail nav in the sidebar's place (mocks A2/B2, left; WP-H).

use baa_mail::{AccountId, Filter, Folder, FolderId, Role, View};
use baa_model::SpaceId;
use baa_web::WebHost;
use gpui::{div, prelude::*, px, AnyElement, Context, Hsla, SharedString, Window};

use super::Field;
use crate::{
    browser::{feel, Browser},
    icons::icon,
    motion::{Feel, Key},
    theme::{ascii, fade, mix, r, Theme},
};

/// The unread count of the inboxes in scope: what the nav, the chips, the door and the status
/// line show.
pub(crate) fn inbox_unread(folders: &[Folder], account: Option<AccountId>) -> u32 {
    folders.iter().filter(|f| f.role == Role::Inbox && account.is_none_or(|a| f.account == a)).map(|f| f.unread).sum()
}

fn rank(role: Role) -> usize {
    [Role::Inbox, Role::Drafts, Role::Sent, Role::Archive, Role::Junk, Role::Trash, Role::Other].iter().position(|r| *r == role).unwrap_or(usize::MAX)
}

fn role_icon(role: Role) -> &'static str {
    match role {
        Role::Inbox => "inbox",
        Role::Drafts => "file-pen",
        Role::Sent => "send",
        Role::Archive => "archive",
        Role::Junk => "ban",
        Role::Trash => "trash-2",
        Role::Other => "folder",
    }
}

struct NavRow {
    key: Key,
    icon: &'static str,
    label: SharedString,
    count: usize,
    selected: bool,
}

fn heading(theme: &Theme, label: &'static str) -> AnyElement {
    if ascii() {
        crate::theme::ascii::rule_label(label, theme.ink_faint).h(px(22.)).mt(px(6.)).into_any_element()
    } else {
        div().flex_none().px(px(8.)).pt(px(12.)).pb(px(4.)).text_size(px(10.)).text_color(theme.ink_faint).child(label).into_any_element()
    }
}

impl<H: WebHost> Browser<H> {
    pub(crate) fn render_nav(&self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let root = div().size_full().debug_selector(|| "mail-nav".into());
        let (Some(ui), Some((mail, space))) = (&self.mail, self.mail_space()) else { return root.into_any_element() };
        let accounts = mail.accounts(space);
        let folders = mail.folders(space);
        let scope = ui.state.account;
        let in_folder = |id: Option<FolderId>| ui.state.folder == id && ui.filter == Filter::All;
        let counted = |filter| mail.count(&View { space, folder: None, account: scope, filter, search: String::new() });
        let space_name = self.session.space(space.id()).map(|s| s.name.clone()).unwrap_or_default();
        let (ink, muted) = (theme.ink, theme.ink_muted);

        let search = div()
            .flex_none()
            .h(px(30.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(r(8.))
            .text_size(px(12.))
            .when(ascii(), |d| d.border_1().border_color(crate::theme::ascii::rule()))
            .when(!ascii(), |d| d.bg(theme.glass))
            .child(icon("search", 13., muted))
            .child(self.render_mail_field(Field::Search, "Search mail", ink, muted, window, cx))
            .child(div().text_size(px(10.)).text_color(theme.ink_faint).child("⌘F"));

        let header = div()
            .flex_none()
            .mt(px(8.))
            .px(px(10.))
            .py(px(7.))
            .rounded(r(10.))
            .when(ascii(), |d| d.border_1().border_color(crate::theme::ascii::rule()))
            .when(!ascii(), |d| d.bg(theme.glass))
            .child(div().text_size(px(12.5)).text_color(ink).child("All inboxes"))
            .child(div().text_size(px(10.5)).text_color(muted).truncate().child(format!("{} mailboxes · {space_name}", accounts.len())));

        let new_message = {
            let key = ("nav-new", 0);
            let h = self.motion.get(key, Feel::Hover);
            let el = div()
                .id(key)
                .flex_none()
                .mt(px(8.))
                .h(px(32.))
                .px(px(12.))
                .flex()
                .items_center()
                .justify_between()
                .rounded(r(9.))
                .text_size(px(12.5))
                .bg(mix(theme.accent, ink, h * 0.15))
                .text_color(if ascii() { crate::theme::surface::bg().into() } else { gpui::white() })
                .on_click(cx.listener(|this, _, window, cx| this.new_message(window, cx)))
                .child(if ascii() {
                    div().child("+ NEW MESSAGE")
                } else {
                    div().flex().items_center().gap(px(6.)).child(icon("pencil-line", 13., gpui::white())).child("New message")
                })
                .child(div().text_size(px(10.)).opacity(0.7).child("⌘N"));
            feel(el, key, cx)
        };

        let mut body = div().id("mail-nav-body").flex_1().min_h_0().overflow_y_scroll().flex().flex_col().child(heading(theme, "FOLDERS"));
        let inbox = NavRow { key: ("nav-inbox", 0), icon: "inbox", label: "Inbox".into(), count: inbox_unread(&folders, scope) as usize, selected: in_folder(None) };
        body = body.child(self.nav_row(theme, inbox, cx, |this, _, cx| this.open_list(None, Filter::All, cx)));
        let shown: Vec<_> = accounts.iter().filter(|a| scope.is_none_or(|s| a.id == s)).collect();
        for account in &shown {
            let mut own: Vec<&Folder> = folders.iter().filter(|f| f.account == account.id && f.role != Role::Inbox).collect();
            own.sort_by_key(|f| rank(f.role));
            if shown.len() > 1 && !own.is_empty() {
                body = body.child(div().flex_none().px(px(8.)).pt(px(6.)).text_size(px(10.5)).text_color(theme.ink_faint).truncate().child(account.address.clone()));
            }
            for f in own {
                let id = f.id;
                let row = NavRow { key: ("nav-folder", id.0), icon: role_icon(f.role), label: f.name.clone().into(), count: f.unread as usize, selected: in_folder(Some(id)) };
                body = body.child(self.nav_row(theme, row, cx, move |this, _, cx| {
                    if let Some(mail) = &this.mail {
                        mail.mail.check_folder(id);
                    }
                    this.open_list(Some(id), Filter::All, cx)
                }));
            }
        }

        body = body.child(heading(theme, "SEARCH FOLDERS"));
        for (n, (filter, name, glyph)) in [(Filter::Unread, "Unread", "circle-dot"), (Filter::Flagged, "Flagged", "flag")].into_iter().enumerate() {
            let row = NavRow { key: ("nav-filter", n as u64), icon: glyph, label: name.into(), count: counted(filter), selected: ui.filter == filter && ui.state.folder.is_none() };
            body = body.child(self.nav_row(theme, row, cx, move |this, _, cx| this.open_list(None, filter, cx)));
        }

        let others = self.mail_other_spaces();
        if !others.is_empty() {
            body = body.child(heading(theme, "OTHER SPACES"));
            for (id, name, unread) in others {
                let row = NavRow { key: ("nav-space", id.0), icon: "mail", label: name, count: unread as usize, selected: false };
                body = body.child(self.nav_row(theme, row, cx, move |this, window, cx| this.show_mail_of(id, window, cx)));
            }
        }

        root.flex().flex_col().p(px(8.)).child(search).child(header).child(new_message).child(body).into_any_element()
    }

    /// The other Spaces that have mail, with their names and unread counts.
    pub(crate) fn mail_other_spaces(&self) -> Vec<(SpaceId, SharedString, u32)> {
        let Some((mail, space)) = self.mail_space() else { return Vec::new() };
        mail.other_spaces(space)
            .into_iter()
            .filter_map(|(id, unread)| Some((id, self.session.space(id)?.name.clone().into(), unread)))
            .collect()
    }

    fn nav_row(&self, theme: &Theme, row: NavRow, cx: &mut Context<Self>, go: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> AnyElement {
        let h = self.motion.get(row.key, Feel::Hover);
        let tone: Hsla = if row.selected && ascii() { theme.accent } else if ascii() { theme.ink_muted } else { theme.ink };
        let el = div()
            .id(row.key)
            .flex_none()
            .h(px(if ascii() { 22. } else { 28. }))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(r(8.))
            .text_size(px(12.5))
            .text_color(tone)
            .on_click(cx.listener(move |this, _, window, cx| go(this, window, cx)));
        let el = if ascii() {
            el.child(if row.selected { "▸" } else { " " }).when(row.selected, |d| d.bg(fade(theme.accent, 0.15)))
        } else {
            el.bg(if row.selected { theme.glass_strong } else { fade(theme.hover, h) }).child(icon(row.icon, 14., if row.selected { theme.accent } else { theme.ink_muted }))
        };
        feel(
            el.child(div().flex_1().min_w_0().truncate().child(row.label))
                .when(row.count > 0, |d| d.child(div().text_size(px(11.)).text_color(theme.ink_muted).child(row.count.to_string()))),
            row.key,
            cx,
        )
        .into_any_element()
    }
}
