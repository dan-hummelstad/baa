//! The Fold's Mail section: every account with its folders. The mail itself stays in the
//! mail view; picking a folder here opens it there.

use baa_mail::{AccountId, Filter, Folder, FolderId, Role, View};
use baa_model::SpaceId;
use baa_web::WebHost;
use gpui::{div, prelude::*, px, AnyElement, Context, Hsla, SharedString, Window};

use super::{list::account_colour, Field};
use crate::{
    browser::{feel, menus::MenuEntry, Browser, MenuKind},
    icons::icon,
    library::{fold_action, fold_rule, fold_search, ink_soft, plural},
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
    indent: bool,
    unread: bool,
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
        let (ink, muted) = (theme.ink, theme.ink_muted);
        let unread = inbox_unread(&folders, None);

        let search = fold_search(theme)
            .child(self.render_mail_field(Field::Search, "Search all mail", ink, ink_soft(theme), window, cx))
            .child(div().text_size(px(10.)).text_color(theme.ink_faint).child("⌘F"));

        let new_message = {
            let key = ("nav-new", 0);
            let h = self.motion.get(key, Feel::Hover);
            let on_accent = if ascii() { crate::theme::surface::bg().into() } else { gpui::white() };
            let el = div()
                .id(key)
                .flex_none()
                .h(px(32.))
                .px(px(11.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(r(8.))
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::BOLD)
                .bg(mix(theme.accent, ink, h * 0.15))
                .text_color(on_accent)
                .on_click(cx.listener(|this, _, window, cx| this.new_message(window, cx)))
                .when(!ascii(), |d| d.child(icon("pencil-line", 13., on_accent)))
                .child(if ascii() { "+ NEW" } else { "New" });
            feel(el, key, cx)
        };

        let mut list = div().id("mail-nav-body").flex_1().min_h_0().overflow_y_scroll().px(px(9.)).py(px(8.)).flex().flex_col().gap(px(2.));
        if accounts.is_empty() {
            list = list.child(
                div()
                    .debug_selector(|| "mail-add-account".into())
                    .flex_none()
                    .p(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .rounded(r(12.))
                    .text_size(px(12.5))
                    .text_color(ink)
                    .when(ascii(), |d| d.border_1().border_color(crate::theme::ascii::rule()))
                    .when(!ascii(), |d| d.bg(theme.glass_strong))
                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Add an account"))
                    .child(div().text_size(px(11.)).text_color(muted).child("Read and send mail here, next to your tabs. Accounts are kept in this Space, or in all of them."))
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
            let all = NavRow {
                key: ("nav-inbox", 0),
                icon: "inbox",
                label: "All inboxes".into(),
                count: unread as usize,
                selected: in_folder(None) && scope.is_none(),
                indent: false,
                unread: true,
            };
            list = list.child(self.nav_row(theme, all, cx, |this, _, cx| this.open_folder(None, cx)));
            for (n, (filter, name, glyph)) in [(Filter::Unread, "Unread", "circle-dot"), (Filter::Flagged, "Flagged", "flag")].into_iter().enumerate() {
                let row = NavRow {
                    key: ("nav-filter", n as u64),
                    icon: glyph,
                    label: name.into(),
                    count: counted(filter),
                    selected: ui.filter == filter && ui.state.folder.is_none(),
                    indent: false,
                    unread: false,
                };
                list = list.child(self.nav_row(theme, row, cx, move |this, _, cx| this.open_list(None, filter, cx)));
            }
            list = list.child(div().flex_none().px(px(6.)).py(px(4.)).child(fold_rule(theme)));

            for (i, account) in accounts.iter().enumerate() {
                let id = account.id;
                let folded = ui.folded.contains(&id);
                let key = ("nav-account", id.0);
                let h = self.motion.get(key, Feel::Hover);
                let own_unread = inbox_unread(&folders, Some(id));
                let header = div()
                    .id(key)
                    .debug_selector(move || format!("nav-account-{}", id.0))
                    .flex_none()
                    .h(px(34.))
                    .pt(px(8.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(r(8.))
                    .bg(fade(theme.hover, h * 0.5))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(ui) = &mut this.mail {
                            if !ui.folded.remove(&id) {
                                ui.folded.insert(id);
                            }
                        }
                        cx.notify();
                    }))
                    .child(icon(if folded { "chevron-right" } else { "chevron-down" }, 12., ink_soft(theme)))
                    .child(div().flex_none().size(px(8.)).rounded_full().bg(account_colour(i)))
                    .child(div().min_w_0().truncate().text_size(px(12.)).font_weight(gpui::FontWeight::BOLD).text_color(ink).child(account.address.clone()))
                    .child(div().flex_1().text_size(px(10.5)).text_color(ink_soft(theme)).child(crate::settings::mail::provider_name(account.provider)))
                    .when(folded && own_unread > 0, |d| {
                        d.child(div().text_size(px(11.)).font_weight(gpui::FontWeight::BOLD).text_color(theme.accent).child(own_unread.to_string()))
                    });
                list = list.child(feel(header, key, cx));
                if folded {
                    continue;
                }
                let mut own: Vec<&Folder> = folders.iter().filter(|f| f.account == id).collect();
                own.sort_by_key(|f| rank(f.role));
                for f in own {
                    let folder = f.id;
                    let inbox = f.role == Role::Inbox;
                    let row = NavRow {
                        key: ("nav-folder", folder.0),
                        icon: role_icon(f.role),
                        label: f.name.clone().into(),
                        count: f.unread as usize,
                        // A scope chip in the mail view on this account's mail is its inbox.
                        selected: in_folder(Some(folder)) || (inbox && in_folder(None) && scope == Some(id)),
                        indent: true,
                        unread: inbox,
                    };
                    list = list.child(self.nav_row(theme, row, cx, move |this, _, cx| {
                        if let Some(mail) = &this.mail {
                            mail.mail.check_folder(folder);
                        }
                        this.open_folder(Some(folder), cx)
                    }));
                }
            }
        }

        let summary = format!("{unread} unread · {}", plural(accounts.len(), "account"));
        let picker = (!accounts.is_empty()).then(|| {
            fold_action("mail-account-select", "Accounts…", theme)
                .debug_selector(|| "mail-account-select".into())
                .on_click(cx.listener(|this, e: &gpui::ClickEvent, window, cx| this.open_menu(MenuKind::MailScope, e.position(), window, cx)))
                .into_any_element()
        });
        root.flex()
            .flex_col()
            .child(div().flex_none().px(px(13.)).py(px(10.)).flex().gap(px(8.)).child(search).child(new_message))
            .child(fold_rule(theme))
            .child(list)
            .child(Self::fold_footer(summary, picker, theme))
            .into_any_element()
    }

    /// A folder of the nav, or All inboxes (`None`): a folder names its account, so a scope
    /// chip set in the mail view goes.
    fn open_folder(&mut self, folder: Option<FolderId>, cx: &mut Context<Self>) {
        if let Some(ui) = &mut self.mail {
            ui.state.account = None;
        }
        self.open_list(folder, Filter::All, cx)
    }

    pub(crate) fn mail_scope_menu(&self) -> Vec<MenuEntry<H>> {
        let Some((mail, space)) = self.mail_space() else { return Vec::new() };
        let scope = self.mail.as_ref().and_then(|m| m.state.account);
        let pick = |label: String, account: Option<AccountId>| MenuEntry::Item {
            label,
            checked: scope == account,
            enabled: true,
            run: Box::new(move |this: &mut Browser<H>, _: &mut Window, cx: &mut Context<Browser<H>>| this.set_scope(account, cx)),
        };
        let mut items = vec![pick("All inboxes".into(), None)];
        items.extend(mail.accounts(space).into_iter().map(|a| pick(a.address, Some(a.id))));
        let others: Vec<(SpaceId, String)> = mail.other_spaces(space).into_iter().filter_map(|(id, _)| Some((id, self.session.space(id)?.name.clone()))).collect();
        if !others.is_empty() {
            items.push(MenuEntry::Separator);
        }
        items.extend(others.into_iter().map(|(id, name)| MenuEntry::Item {
            label: format!("Go to {name}"),
            checked: false,
            enabled: true,
            run: Box::new(move |this: &mut Browser<H>, window: &mut Window, cx: &mut Context<Browser<H>>| this.show_mail_of(id, window, cx)),
        }));
        items
    }

    fn nav_row(&self, theme: &Theme, row: NavRow, cx: &mut Context<Self>, go: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> AnyElement {
        let h = self.motion.get(row.key, Feel::Hover);
        let tone: Hsla = if row.selected { theme.accent } else if ascii() { theme.ink_muted } else { theme.ink };
        let (name, n) = row.key;
        let el = div()
            .id(row.key)
            .debug_selector(move || format!("{name}-{n}"))
            .flex_none()
            .h(px(if ascii() { 22. } else { 30. }))
            .pl(px(if row.indent { 24. } else { 10. }))
            .pr(px(10.))
            .flex()
            .items_center()
            .gap(px(9.))
            .rounded(r(8.))
            .text_size(px(12.5))
            .text_color(tone)
            .when(row.selected, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
            .on_click(cx.listener(move |this, _, window, cx| go(this, window, cx)));
        let el = if ascii() {
            el.child(if row.selected { "▸" } else { " " }).when(row.selected, |d| d.bg(fade(theme.accent, 0.15)))
        } else {
            el.bg(if row.selected { fade(theme.accent, 0.12) } else { fade(theme.hover, h) })
                .child(icon(row.icon, 14., if row.selected { theme.accent } else { theme.ink_muted }))
        };
        let count_tone = if row.unread { theme.accent } else { ink_soft(theme) };
        feel(
            el.child(div().flex_1().min_w_0().truncate().child(row.label)).when(row.count > 0, |d| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(count_tone)
                        .when(row.unread, |d| d.font_weight(gpui::FontWeight::BOLD))
                        .child(row.count.to_string()),
                )
            }),
            row.key,
            cx,
        )
        .into_any_element()
    }
}
