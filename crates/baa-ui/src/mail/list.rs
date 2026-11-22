//! The message list and the status line under it (mocks A2/B2, middle; WP-H).

use std::ops::Range;

use baa_mail::{AccountId, Category, Filter, FolderId, Role, Row, SyncState, Tab, ThreadId, View};
use baa_web::WebHost;
use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, Hsla, SharedString, Window};

use super::{nav::inbox_unread, now_secs};
use crate::{
    browser::Browser,
    icons::icon,
    input::{Key, TextField},
    theme::{ascii, r, surface, Theme},
};

const ROW_H: f32 = 82.;
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

const PILLS: [(Option<Category>, &str, &str); 4] = [
    (Some(Category::Primary), "Primary", "user"),
    (Some(Category::Social), "Social", "users"),
    (Some(Category::Promotions), "Promotions", "megaphone"),
    (None, "All Mail", "inbox"),
];

pub(crate) fn inbox_filter(categories: bool) -> Filter {
    if categories { Filter::Category(Category::Primary) } else { Filter::All }
}

#[derive(Clone)]
pub(crate) struct DraggedThread {
    thread: ThreadId,
    subject: SharedString,
    theme: Theme,
}

impl Render for DraggedThread {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(280.))
            .h(px(28.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(r(8.))
            .bg(self.theme.glass_strong)
            .text_color(self.theme.ink)
            .text_size(px(12.5))
            .child(div().truncate().child(self.subject.clone()))
    }
}

/// The dot of the account at `index` among its Space's accounts, in the list and the reader.
pub(crate) fn account_colour(index: usize) -> Hsla {
    const HUES: [(f32, f32, f32); 6] = [(0.70, 0.65, 0.58), (0.08, 0.85, 0.52), (0.38, 0.55, 0.42), (0.58, 0.75, 0.52), (0.92, 0.65, 0.58), (0.50, 0.65, 0.42)];
    let (h, s, l) = HUES[index % HUES.len()];
    gpui::hsla(h, s, l, 1.)
}

/// (year, month, day) of a day count since 1970-01-01 (Hinnant's civil_from_days).
fn civil(days: u64) -> (i64, usize, u64) {
    let z = days as i64 + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u64;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as usize;
    (yoe + era * 400 + (month <= 2) as i64, month, day)
}

/// `secs` on the local clock: shifted by the time zone's offset from UTC then.
fn local(secs: u64) -> u64 {
    let t = secs as libc::time_t;
    // SAFETY: `localtime_r` only writes the `tm` it is given, which is zeroed and ours.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let offset = if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() { 0 } else { tm.tm_gmtoff };
    secs.saturating_add_signed(offset)
}

fn short_date(secs: u64, now: u64) -> String {
    let ((year, month, day), (this_year, ..)) = (civil(secs / 86_400), civil(now / 86_400));
    let name = MONTHS[month - 1];
    if year == this_year { format!("{day} {name}") } else { format!("{day} {name} {year}") }
}

/// Today: the clock; yesterday: "Yesterday"; before: the date.
pub(crate) fn when(secs: u64) -> String {
    when_at(local(secs), local(now_secs()))
}

fn when_at(secs: u64, now: u64) -> String {
    match (now / 86_400).saturating_sub(secs / 86_400) {
        0 => format!("{:02}:{:02}", secs % 86_400 / 3600, secs % 3600 / 60),
        1 => "Yesterday".into(),
        _ => short_date(secs, now),
    }
}

fn age(secs: u64) -> String {
    match secs {
        0..5 => "just now".into(),
        5..60 => format!("{secs}s ago"),
        60..3600 => format!("{} min ago", secs / 60),
        _ => format!("{} h ago", secs / 3600),
    }
}

fn status_line(states: &[SyncState], synced_secs_ago: Option<u64>, unread: u32) -> String {
    let has = |s| states.contains(&s);
    if has(SyncState::SignInAgain) {
        return "sign in again".into();
    }
    if has(SyncState::Offline) {
        return "offline".into();
    }
    if has(SyncState::Syncing) {
        return "syncing…".into();
    }
    let head = match (synced_secs_ago, ascii()) {
        (Some(s), true) => format!("imap ✓ synced {}", age(s)),
        (Some(s), false) => format!("Synced {}", age(s)),
        (None, _) => "imap".into(),
    };
    format!("{head} · {unread} unread")
}

impl<H: WebHost> Browser<H> {
    /// A folder or a search folder opened from the nav: the list's tab comes to the front.
    pub(crate) fn open_list(&mut self, folder: Option<FolderId>, filter: Filter, cx: &mut Context<Self>) {
        let inbox = filter == Filter::All && self.is_inbox(folder);
        let categories = self.settings.mail.categories;
        let Some(ui) = &mut self.mail else { return };
        ui.state.folder = folder;
        ui.filter = if inbox { inbox_filter(categories) } else { filter };
        ui.state.scroll = 0;
        ui.list.scroll_to_item(0, gpui::ScrollStrategy::Top);
        self.show_tab(Tab::List, cx);
    }

    fn is_inbox(&self, folder: Option<FolderId>) -> bool {
        let Some(id) = folder else { return true };
        self.mail_space().is_some_and(|(mail, space)| mail.folders(space).iter().any(|f| f.id == id && f.role == Role::Inbox))
    }

    pub(crate) fn move_to_category(&mut self, thread: ThreadId, category: Category, cx: &mut Context<Self>) {
        if let Some(ui) = &self.mail {
            ui.mail.move_to_category(thread, category);
        }
        cx.notify();
    }

    fn render_pills(&self, view: &View, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let shown = self.settings.mail.categories
            && view.search.is_empty()
            && matches!(view.filter, Filter::All | Filter::Category(_))
            && self.is_inbox(view.folder);
        let (Some(ui), true) = (&self.mail, shown) else { return None };
        let mut row = div().flex_none().px(px(12.)).pb(px(8.)).flex().gap(px(6.));
        for (i, (category, name, glyph)) in PILLS.into_iter().enumerate() {
            let filter = category.map_or(Filter::All, Filter::Category);
            let on = view.filter == filter;
            let unread = category.is_some() && ui.mail.unread(&View { filter, ..view.clone() }) > 0;
            let ink: Hsla = if on { theme.ink } else { surface::icon().into() };
            let mut pill = div()
                .id(("mail-pill", i))
                .debug_selector(move || format!("mail-pill-{i}"))
                .h(px(28.))
                .px(px(if on || ascii() { 12. } else { 10. }))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(r(14.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(ink)
                .when(on, |d| d.bg(surface::key()).font_weight(FontWeight::SEMIBOLD))
                .when(!on, |d| d.hover(|d| d.bg(surface::key())))
                .when(!ascii(), |d| d.child(icon(glyph, 14., ink)))
                .when(on || ascii(), |d| d.child(name))
                .when(unread && !on, |d| d.child(div().size(px(6.)).rounded_full().bg(theme.accent)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(ui) = &mut this.mail {
                        ui.filter = filter;
                        ui.state.scroll = 0;
                        ui.list.scroll_to_item(0, gpui::ScrollStrategy::Top);
                    }
                    cx.notify();
                }));
            if let Some(category) = category {
                let accent = theme.accent;
                pill = pill
                    .drag_over::<DraggedThread>(move |s, _, _, _| s.border_1().border_color(accent))
                    .on_drop(cx.listener(move |this, d: &DraggedThread, _, cx| this.move_to_category(d.thread, category, cx)));
            }
            row = row.child(pill);
        }
        Some(row.into_any_element())
    }

    /// A scope chip or the account picker: one account's mail, or all of them (`None`).
    pub(crate) fn set_scope(&mut self, account: Option<AccountId>, cx: &mut Context<Self>) {
        let Some(ui) = &mut self.mail else { return };
        ui.state.account = account;
        ui.state.scroll = 0;
        ui.list.scroll_to_item(0, gpui::ScrollStrategy::Top);
        self.show_tab(Tab::List, cx);
    }

    /// What the list shows, for its header, its tab and the preview's way back: the name, a
    /// number, and whether that number counts unread mail (else the matches).
    pub(crate) fn list_title(&self) -> (String, usize, bool) {
        let (Some(ui), Some((mail, space))) = (&self.mail, self.mail_space()) else { return (String::new(), 0, false) };
        let Some(view) = ui.view() else { return (String::new(), 0, false) };
        let folders = mail.folders(space);
        let named = view.folder.and_then(|id| folders.iter().find(|f| f.id == id));
        let several = view.account.is_none() && mail.accounts(space).len() > 1;
        match (!view.search.is_empty(), view.filter, named) {
            (true, ..) => ("Search".into(), mail.count(&view), false),
            (_, Filter::Unread, _) => ("Unread".into(), mail.count(&view), false),
            (_, Filter::Flagged, _) => ("Flagged".into(), mail.count(&view), false),
            (_, _, Some(f)) => (f.name.clone(), f.unread as usize, true),
            _ => ((if several { "All inboxes" } else { "Inbox" }).into(), inbox_unread(&folders, view.account) as usize, true),
        }
    }

    /// The list: a column beside the reading pane when `beside`, else across the mail view.
    pub(crate) fn render_list(&self, beside: bool, theme: &Theme, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let col = div().h_full().flex().flex_col();
        let col = if beside { col.w(px(380.)).flex_none() } else { col.w_full() };
        let (Some(ui), Some((mail, space))) = (&self.mail, self.mail_space()) else { return col.into_any_element() };
        let Some(view) = ui.view() else { return col.into_any_element() };
        let accounts = mail.accounts(space);
        let folders = mail.folders(space);
        let count = mail.count(&view);
        let unread = inbox_unread(&folders, view.account);
        let searching = !view.search.is_empty();
        let (title, n, of_unread) = self.list_title();
        let pill = match of_unread {
            true => Some(n).filter(|n| *n > 0).map(|n| format!("{n} unread")),
            false => Some(n.to_string()),
        };
        let header = div()
            .flex_none()
            .h(px(46.))
            .px(px(14.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).truncate().child(title))
            .children(pill.map(|p| div().flex_none().px(px(7.)).py(px(1.)).rounded(r(8.)).text_size(px(11.)).bg(surface::key()).text_color(theme.accent).child(p)));

        let chips = (accounts.len() > 1).then(|| {
            let chip = |key: u64, label: String, dot: Option<Hsla>, n: u32, account: Option<AccountId>, cx: &mut Context<Self>| {
                let on = view.account == account;
                div()
                    .id(("mail-chip", key))
                    .h(px(22.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .rounded(r(11.))
                    .border_1()
                    .border_color(if on { theme.accent } else { surface::border().into() })
                    .when(on, |d| d.bg(surface::key()))
                    .text_size(px(11.))
                    .children(dot.map(|c| div().size(px(6.)).rounded_full().bg(c)))
                    .child(label)
                    .when(n > 0, |d| d.child(div().text_color(surface::muted()).child(n.to_string())))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_scope(account, cx)))
            };
            let mut row = div().flex_none().px(px(12.)).pb(px(8.)).flex().flex_wrap().gap(px(6.)).child(chip(0, "All inboxes".into(), None, unread, None, cx));
            for (i, a) in accounts.iter().enumerate() {
                row = row.child(chip(a.id.0 + 1, a.address.clone(), Some(account_colour(i)), inbox_unread(&folders, Some(a.id)), Some(a.id), cx));
            }
            row
        });

        let body = if accounts.is_empty() {
            self.list_empty("No mail account in this Space", true, theme, cx)
        } else if count == 0 {
            self.list_empty(if searching { "No messages match" } else { "Nothing here" }, false, theme, cx)
        } else {
            let theme = *theme;
            gpui::uniform_list("mail-rows", count, cx.processor(move |this, range: Range<usize>, _, cx| this.list_rows(&theme, range, cx)))
                .size_full()
                .track_scroll(&ui.list)
                .into_any_element()
        };

        let states: Vec<SyncState> = accounts.iter().filter(|a| view.account.is_none_or(|s| a.id == s)).filter_map(|a| ui.status.get(&a.id).copied()).collect();
        let status = (!accounts.is_empty()).then(|| {
            let line = status_line(&states, ui.synced_at.map(|t| t.elapsed().as_secs()), unread);
            div()
                .flex_none()
                .h(px(24.))
                .px(px(14.))
                .flex()
                .items_center()
                .border_t_1()
                .border_color(surface::line())
                .text_size(px(if ascii() { 11. } else { 10.5 }))
                .text_color(surface::muted())
                .child(div().truncate().child(line))
        });

        let pills = self.render_pills(&view, theme, cx);
        col.child(header).children(pills).children(chips).child(div().flex_1().min_h_0().child(body)).children(status).into_any_element()
    }

    fn list_empty(&self, text: &'static str, add: bool, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.))
            .text_size(px(12.5))
            .text_color(surface::muted())
            .child(text)
            .when(add, |d| {
                d.child(
                    div()
                        .id("mail-add-account-button")
                        .h(px(28.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .rounded(r(8.))
                        .bg(theme.accent)
                        .text_color(if ascii() { surface::bg().into() } else { gpui::white() })
                        .on_click(cx.listener(|this, _, _, cx| this.open_mail_settings(cx)))
                        .child("Add an account"),
                )
            })
            .into_any_element()
    }

    fn list_rows(&self, theme: &Theme, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let (Some(ui), Some((mail, space))) = (&self.mail, self.mail_space()) else { return Vec::new() };
        let Some(view) = ui.view() else { return Vec::new() };
        let accounts = mail.accounts(space);
        let now = local(now_secs());
        let start = range.start;
        mail.rows(&view, range)
            .iter()
            .enumerate()
            .map(|(n, row)| {
                let dot = match accounts.len() > 1 {
                    true => account_colour(accounts.iter().position(|a| a.id == row.account).unwrap_or(0)),
                    false => theme.accent,
                };
                self.list_row(theme, start + n, row, dot, now, cx)
            })
            .collect()
    }

    fn list_row(&self, theme: &Theme, index: usize, row: &Row, dot: Hsla, now: u64, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.mail.as_ref().is_some_and(|ui| ui.state.selected == Some(row.thread));
        let thread = row.thread;
        let mark = |name: &'static str, glyph: &'static str, c: Hsla| if ascii() { div().text_size(px(11.)).text_color(c).child(glyph).into_any_element() } else { icon(name, 12., c).into_any_element() };
        let line = |h: f32| div().h(px(h)).flex().items_center().gap(px(8.)).min_w_0();
        let text = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .pt(px(7.))
            .pr(px(12.))
            .flex()
            .flex_col()
            .when(!selected, |d| d.border_b_1().border_color(surface::line()))
            .child(
                line(18.)
                    .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(SharedString::from(row.from.clone())))
                    .child(div().flex_none().text_size(px(11.5)).text_color(surface::muted()).child(when_at(local(row.date), now))),
            )
            .child(
                line(17.)
                    .child(div().flex_1().min_w_0().truncate().text_size(px(12.5)).child(SharedString::from(row.subject.clone())))
                    .when(row.attachments, |d| d.child(mark("paperclip", "▯", surface::icon().into())))
                    .when(row.flagged, |d| d.child(mark("flag", "⚑", surface::warn().into()))),
            )
            .child(div().text_size(px(12.)).line_height(px(15.)).line_clamp(2).text_color(surface::muted()).child(SharedString::from(row.snippet.clone())));
        div()
            .id(("mail-row", index))
            .debug_selector(|| format!("mail-row-{index}"))
            .w_full()
            .h(px(ROW_H))
            .px(px(6.))
            .cursor_pointer()
            .child(
                div()
                    .size_full()
                    .flex()
                    .rounded(r(8.))
                    .when(selected, |d| d.bg(surface::key()))
                    .child(div().flex_none().w(px(22.)).pt(px(12.)).flex().justify_center().when(row.unread, |d| d.child(div().size(px(8.)).rounded_full().bg(dot))))
                    .child(text),
            )
            .on_drag(DraggedThread { thread, subject: row.subject.clone().into(), theme: *theme }, |d, _, _, cx| cx.new(|_| d.clone()))
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(ui) = &mut this.mail {
                    ui.state.scroll = ui.list.0.borrow().base_handle.logical_scroll_top().0;
                    window.focus(&ui.focus, cx);
                }
                this.select_thread(thread, window, cx);
            }))
            .into_any_element()
    }

    /// A key in the search field; `before` is its text before the key.
    pub(crate) fn on_search_key(&mut self, key: Key, before: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ui) = &mut self.mail else { return };
        match key {
            Key::Edited if ui.search.text != before => {
                ui.state.scroll = 0;
                ui.list.scroll_to_item(0, gpui::ScrollStrategy::Top);
            }
            Key::Cancel => {
                ui.search = TextField::default();
                ui.state.scroll = 0;
                ui.list.scroll_to_item(0, gpui::ScrollStrategy::Top);
                self.leave_mail_field(window, cx);
            }
            Key::Submit | Key::Down => self.leave_mail_field(window, cx),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_a_clock_today_yesterday_and_a_date_before() {
        let now = 1_763_800_000; // 2025-11-22 08:26 UTC
        assert_eq!(when_at(now - 3600, now), "07:26");
        assert_eq!(when_at(now - 86_400, now), "Yesterday");
        assert_eq!(when_at(now - 86_400 * 8, now), "14 Nov");
        assert_eq!(when_at(now - 86_400 * 400, now), "18 Oct 2024");
    }

    #[test]
    fn the_status_line_names_the_worst_state_first() {
        assert_eq!(status_line(&[SyncState::Synced, SyncState::Offline], Some(3), 2), "offline");
        assert_eq!(status_line(&[SyncState::Syncing], None, 2), "syncing…");
        assert_eq!(status_line(&[SyncState::SignInAgain, SyncState::Offline], None, 2), "sign in again");
        assert!(status_line(&[SyncState::Synced], Some(40), 12).ends_with("40s ago · 12 unread"));
    }
}
