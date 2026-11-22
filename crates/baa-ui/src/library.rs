//! The Fold (was the Archive), in the sidebar: what lives outside any Space. Opened from the
//! archive button, ⌘⇧A, a swipe up over the bottom bar or a ⌘-swipe, on the section it was last
//! on. Every section has the same spine: identity, segments, toolbar, list, footer. Sections:
//! Mail (each account's folders, while the page area shows the mail view), Downloads
//! (kept across launches, grouped by age, searchable and filtered by kind),
//! Archived Tabs, and Chat (the Claude panel, ⌘J).

use crate::theme::r;
use std::path::{Path, PathBuf};

use baa_model::{now, DownloadRecord, TabId};
use baa_web::{DownloadState, WebHost};
use gpui::{div, img, prelude::*, px, relative, AnyElement, Context, Hsla, KeyDownEvent, Window};

use crate::{
    browser::{feel, host_of, shadow, tab_title, Browser, MenuKind, SIDEBAR_WIDTH},
    icons::icon,
    input::{Key, TextField},
    motion::Feel,
    theme::{ascii, fade, mix, Theme},
};

/// The mock's in-between inks: group headers and chips; placeholders and meta lines.
pub(crate) fn ink_mid(theme: &Theme) -> Hsla {
    mix(theme.ink_muted, theme.ink, 0.5)
}
pub(crate) fn ink_soft(theme: &Theme) -> Hsla {
    mix(theme.ink_faint, theme.ink_muted, 0.5)
}

/// The mocks' raised white card (active segment, chat title, switcher); ASCII Chrome boxes it instead.
pub(crate) fn raised<E: Styled>(d: E, theme: &Theme) -> E {
    d.bg(theme.glass_strong)
        .border_1()
        .border_color(if ascii() { crate::theme::ascii::frame().into() } else { Hsla { a: 0.79, ..gpui::white() } })
        .shadow(vec![shadow(Hsla { a: 0.14, ..theme.ink }, 3., 14.)])
}

pub(crate) fn fold_rule(theme: &Theme) -> gpui::Div {
    div().flex_none().h(px(1.)).bg(if ascii() { crate::theme::ascii::rule().into() } else { theme.divider })
}

pub(crate) fn fold_search(theme: &Theme) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .h(px(32.))
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(7.))
        .rounded(r(9.))
        .text_size(px(12.5))
        .border_1()
        .border_color(if ascii() { crate::theme::ascii::rule().into() } else { theme.divider })
        .when(!ascii(), |d| d.bg(theme.glass))
        .child(icon("search", 14., ink_soft(theme)))
}

pub(crate) fn fold_action(id: &'static str, label: &'static str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .text_size(px(11.5))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.accent)
        .cursor_pointer()
        .child(label)
}

/// The sidebar while the Fold is open.
pub(crate) const LIBRARY_WIDTH: f32 = 374.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Section {
    Mail,
    Downloads,
    Archive,
    Chat,
}

/// What the Downloads filter keeps.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Kind {
    All,
    Images,
    Videos,
    Audio,
    Documents,
    Archives,
}

pub(crate) const KINDS: [(Kind, &str); 6] = [
    (Kind::All, "All Downloads"),
    (Kind::Images, "Images"),
    (Kind::Videos, "Movies"),
    (Kind::Audio, "Audio"),
    (Kind::Documents, "Documents"),
    (Kind::Archives, "Archives"),
];

pub(crate) struct Library {
    pub section: Section,
    pub search: TextField,
    pub filter: Kind,
}

/// What a file is, from its extension: (kind, "Zip Archive", icon).
fn file_kind(path: &str) -> (Kind, &'static str, &'static str) {
    let ext = Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "avif" | "bmp" | "svg" => (Kind::Images, "Image", "file-image"),
        "mp4" | "mov" | "webm" | "mkv" | "avi" | "m4v" => (Kind::Videos, "Movie", "file-video-camera"),
        "mp3" | "wav" | "m4a" | "flac" | "aac" | "ogg" => (Kind::Audio, "Audio", "file-music"),
        "zip" | "gz" | "tgz" | "tar" | "7z" | "rar" | "xz" | "bz2" => (Kind::Archives, "Zip Archive", "file-archive"),
        "dmg" | "iso" | "pkg" => (Kind::Archives, "Disk Image", "disc"),
        "pdf" => (Kind::Documents, "PDF Document", "file-text"),
        "csv" | "xls" | "xlsx" | "numbers" => (Kind::Documents, "Spreadsheet", "file-spreadsheet"),
        "ics" => (Kind::Documents, "Calendar Event", "calendar"),
        "txt" | "md" | "doc" | "docx" | "pages" | "rtf" | "json" | "html" => (Kind::Documents, "Document", "file-text"),
        _ => (Kind::All, "Download", "file"),
    }
}

/// "Zip Archive from github.com"; data and blob URLs have no host to name.
fn detail(path: &str, url: &str) -> String {
    let (_, kind, _) = file_kind(path);
    let host = host_of(url);
    if url.starts_with("http") && !host.is_empty() {
        format!("{kind} from {host}")
    } else {
        kind.to_string()
    }
}

/// Arc's buckets: "Today", "Yesterday", "3 days ago", "2 weeks ago", ...
pub(crate) fn ago(now: i64, then: i64) -> String {
    let days = (now - then).max(0) / 86_400;
    let plural = |n: i64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match days {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=6 => plural(days, "day"),
        7..=29 => plural(days / 7, "week"),
        30..=364 => plural(days / 30, "month"),
        _ => plural(days / 365, "year"),
    }
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// Every word of `query` appears in one of `fields`, ignoring case.
fn matches(query: &str, fields: &[&str]) -> bool {
    let hay = fields.join(" ").to_lowercase();
    query.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
}

pub(crate) fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// Group headers with how many rows each holds; `rows` are (group, row) in list order.
fn grouped(rows: Vec<(String, AnyElement)>, theme: &Theme) -> Vec<AnyElement> {
    let mut out = Vec::new();
    let mut rows = rows.into_iter().peekable();
    while let Some((group, first)) = rows.next() {
        let mut members = vec![first];
        while let Some((_, row)) = rows.next_if(|(g, _)| *g == group) {
            members.push(row);
        }
        out.push(
            div()
                .flex_none()
                .pt(px(13.))
                .pb(px(5.))
                .px(px(6.))
                .flex()
                .items_center()
                .text_size(px(10.5))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(ink_soft(theme))
                .child(div().flex_1().child(group.to_uppercase()))
                .child(div().font_weight(gpui::FontWeight::NORMAL).child(members.len().to_string()))
                .into_any_element(),
        );
        out.extend(members);
    }
    out
}

impl<H: WebHost> Browser<H> {
    /// The sidebar's width now: wider while the Fold is open.
    pub(crate) fn sidebar_width(&self) -> f32 {
        if self.library.is_some() {
            LIBRARY_WIDTH
        } else {
            SIDEBAR_WIDTH
        }
    }

    /// Mail is the active Space showing mail ([`Browser::show_mail`]); any other section
    /// leaves mail for the browser.
    pub(crate) fn open_library(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        // library_last may be the Mail of a Space that has mail, and this one may not.
        match section {
            Section::Mail if self.mail_here() => return self.show_mail(true, window, cx),
            Section::Mail => self.set_library(Section::Downloads, cx),
            section => self.set_library(section, cx),
        }
        if self.mail_in_front() {
            self.show_mail(false, window, cx);
        }
    }

    fn set_library(&mut self, section: Section, cx: &mut Context<Self>) {
        self.library_last = section;
        match &mut self.library {
            Some(library) => library.section = section,
            None => {
                self.library = Some(Library { section, search: TextField::default(), filter: Kind::All });
                self.motion.from(("library", 0), Feel::Glide, 0.0, 1.0);
            }
        }
        self.sidebar_w.set_target(LIBRARY_WIDTH);
        cx.notify();
    }

    /// After every session change: the Fold is open on Mail exactly while the active
    /// Space shows mail, so a Space switch, a split's focus and ⌘⇧M all move it.
    pub(crate) fn sync_library(&mut self, cx: &mut Context<Self>) {
        let on_mail = self.library.as_ref().is_some_and(|l| l.section == Section::Mail);
        if self.mail_in_front() && !on_mail {
            self.set_library(Section::Mail, cx);
        } else if !self.mail_in_front() && on_mail {
            self.collapse_library();
        }
    }

    pub(crate) fn open_library_last(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.library_last {
            // Chat also takes focus for its field.
            Section::Chat => self.open_chat(window, cx),
            last => self.open_library(last, window, cx),
        }
    }

    pub(crate) fn toggle_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.library.is_some() {
            self.close_library(window, cx);
        } else {
            self.open_library_last(window, cx);
        }
    }

    /// The Fold always shows docked; closing it brings back the user's `sidebar_open`, which it never changes.
    pub(crate) fn docked(&self) -> bool {
        self.sidebar_open || self.library.is_some()
    }

    /// Back to the Spaces; the first comes in from the right, as it lies. Closed on Mail,
    /// the page the Space was on comes back.
    pub(crate) fn close_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(library) = self.collapse_library() else { return };
        self.close_overlays(window, cx);
        if library.section == Section::Mail && self.mail_in_front() {
            self.show_mail(false, window, cx);
        }
    }

    fn collapse_library(&mut self) -> Option<Library> {
        let library = self.library.take()?;
        self.sidebar_w.set_target(if self.sidebar_open { SIDEBAR_WIDTH } else { 0. });
        self.slide.snap(1.0);
        self.slide.set_target(0.0);
        Some(library)
    }

    /// Finished downloads go into the store, so the Fold keeps them.
    pub(crate) fn remember_downloads(&mut self, items: &[baa_web::Download]) {
        // An incognito download is the user's file, but no record of it is kept.
        for d in items.iter().filter(|d| d.state == DownloadState::Complete && !d.ephemeral) {
            if !self.recorded_downloads.insert(d.id) {
                continue;
            }
            let record = DownloadRecord {
                path: d.path.clone(),
                url: d.url.clone(),
                bytes: d.total_bytes.max(d.received_bytes),
                finished_at: now() as i64,
            };
            if let Some(Err(e)) = self.store().map(|s| s.record_download(&record)) {
                log::error!("saving download: {e}");
            }
            self.saved_downloads.retain(|s| s.path != record.path);
            self.saved_downloads.insert(0, record);
        }
    }

    pub(crate) fn forget_download(&mut self, path: &str, cx: &mut Context<Self>) {
        if let Some(Err(e)) = self.store().map(|s| s.remove_download(path)) {
            log::error!("removing download: {e}");
        }
        self.saved_downloads.retain(|d| d.path != path);
        cx.notify();
    }

    /// "Clear list": the Fold forgets every finished download; the files stay where they are.
    /// An incognito window has no store, so nothing is written from one.
    pub(crate) fn clear_downloads(&mut self, cx: &mut Context<Self>) {
        if let Some(Err(e)) = self.store().map(|s| s.clear_downloads_since(i64::MIN)) {
            log::error!("clearing downloads: {e}");
        }
        self.saved_downloads.clear();
        cx.notify();
    }

    pub(crate) fn on_library_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(library) = &mut self.library else { return };
        let key = library.search.handle(&event.keystroke, cx);
        match key {
            Key::Cancel if library.search.text.is_empty() => self.close_library(window, cx),
            Key::Cancel => library.search = TextField::default(),
            _ => {}
        }
        key.stop_unless_typed(cx);
        cx.notify();
    }

    pub(crate) fn render_library(&self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(library) = &self.library else { return div().into_any_element() };
        let t = self.motion.get(("library", 0), Feel::Glide);
        let body = match library.section {
            Section::Mail => self.render_nav(theme, window, cx),
            Section::Chat => div()
                .size_full()
                .px(px(13.))
                .flex()
                .child(self.render_agent_panel(theme, window, cx).unwrap_or_else(|| div().into_any_element()))
                .into_any_element(),
            Section::Downloads | Section::Archive => self.render_fold_list(library, theme, window, cx),
        };
        let slide = self.slide.value();
        div()
            .w(px(LIBRARY_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            // Comes in from the left, where it sits before the first Space.
            .ml(px(-40. * (1. - t)))
            .opacity(t.clamp(0., 1.))
            // Vertical scrolls stay with the lists: the swipe takes only mostly-horizontal ones.
            .on_scroll_wheel(cx.listener(|this, e, window, cx| this.on_chrome_scroll(e, window, cx)))
            .child(self.render_fold_identity(theme, cx))
            .child(fold_rule(theme))
            .child(self.render_fold_segments(library, theme, cx))
            .child(fold_rule(theme))
            .child(
                // Clipped, so a Space switch's slide never passes over the segments.
                div().flex_1().min_h_0().overflow_hidden().child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .relative()
                        .left(px(slide * SIDEBAR_WIDTH))
                        .opacity(1.0 - slide.abs().min(1.0) * 0.6)
                        .child(body),
                ),
            )
            .into_any_element()
    }

    fn render_fold_identity(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let soft = ink_soft(theme);
        let close = self
            .icon_button("library-back", "x", true, 0., 0., theme, cx)
            .on_click(cx.listener(|this, _, window, cx| this.close_library(window, cx)));
        div()
            .flex_none()
            .h(px(44.))
            .px(px(13.))
            .flex()
            .items_center()
            .gap(px(9.))
            .child(img(crate::icons::MARK).size(px(19.)).rounded(r(5.)))
            .child(div().text_size(px(13.5)).font_weight(gpui::FontWeight::BOLD).text_color(theme.ink).child("The Fold"))
            .child(div().flex_1())
            // The gesture that opened it, so the way out is always in sight.
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .px(px(7.))
                    .py(px(3.))
                    .rounded(r(7.))
                    .bg(theme.hover)
                    .text_size(px(11.))
                    .text_color(soft)
                    .child("⌘")
                    .child(icon("chevrons-left-right", 13., soft)),
            )
            .child(close)
    }

    fn render_fold_segments(&self, library: &Library, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        // Only the open segment shows its count.
        let n = match library.section {
            Section::Mail => self.mail_space().map_or(0, |(mail, space)| crate::mail::inbox_unread(&mail.folders(space), None) as usize),
            Section::Downloads => self.host.downloads().read(cx).items.iter().filter(|d| d.state == DownloadState::InProgress).count(),
            Section::Archive => self.session.archived().len(),
            Section::Chat => 0,
        };
        let mut sections = vec![(Section::Downloads, "arrow-down-to-line", "Downloads"), (Section::Archive, "archive", "Tabs"), (Section::Chat, "sparkles", "Chat")];
        if self.mail_here() {
            sections.insert(0, (Section::Mail, "mail", "Mail"));
        }
        let mut segs = Vec::new();
        for (section, name, label) in sections {
            let key = ("library-nav", section as u64);
            let h = self.motion.get(key, Feel::Hover);
            let on = library.section == section;
            let el = div()
                .id(key)
                .when(section == Section::Mail, |d| d.debug_selector(|| "library-mail".into()))
                .h(px(34.))
                .px(px(if on { 11. } else { 10. }))
                .flex()
                .items_center()
                .gap(px(7.))
                .rounded(r(10.))
                .when(on, |d| raised(d, theme))
                .when(!on, |d| d.border_1().border_color(gpui::transparent_black()).bg(fade(theme.hover, h)))
                .on_click(cx.listener(move |this, _, window, cx| match section {
                    Section::Chat => this.open_chat(window, cx),
                    _ => this.open_library(section, window, cx),
                }))
                .child(icon(name, 16., if on { theme.accent } else { mix(theme.ink_muted, theme.ink, h) }))
                .when(on, |d| {
                    d.child(div().text_size(px(12.5)).font_weight(gpui::FontWeight::BOLD).text_color(theme.ink).child(label)).when(n > 0, |d| {
                        d.child(
                            div()
                                .px(px(5.))
                                .rounded(r(6.))
                                .bg(fade(theme.accent, 0.12))
                                .text_size(px(10.5))
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(theme.accent)
                                .child(n.to_string()),
                        )
                    })
                });
            segs.push(feel(el, key, cx).into_any_element());
        }
        let section = library.section;
        let settings = self.icon_button("library-settings", "sliders-horizontal", true, 0., 0., theme, cx).on_click(cx.listener(move |this, _, _, cx| {
            use crate::settings::Tab;
            let (tab, row) = match section {
                Section::Mail => (Tab::Mail, None),
                Section::Archive => (Tab::General, Some("archive")),
                Section::Downloads => (Tab::General, None),
                Section::Chat => (Tab::Claude, None),
            };
            this.open_settings_at(Some(tab), row, cx)
        }));
        div().flex_none().h(px(50.)).px(px(10.)).flex().items_center().gap(px(3.)).children(segs).child(div().flex_1()).child(settings)
    }

    /// The Fold's last row, where the bottom bar was: what the section holds and the one
    /// action that goes with it. A swipe down here closes the Fold.
    pub(crate) fn fold_footer(&self, summary: String, action: Option<AnyElement>, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_none()
            .flex()
            .flex_col()
            .child(fold_rule(theme))
            .child(
                div()
                    .id("library-bottom")
                    .debug_selector(|| "library-bottom".into())
                    .h(px(46.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .bg(theme.glass)
                    .text_size(px(11.5))
                    .on_scroll_wheel(cx.listener(|this, e, window, cx| this.on_bottom_bar_scroll(e, window, cx)))
                    .child(div().flex_1().min_w_0().truncate().text_color(ink_soft(theme)).child(summary))
                    .children(action),
            )
            .into_any_element()
    }

    fn render_fold_list(&self, library: &Library, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let downloads = library.section == Section::Downloads;
        let search = fold_search(theme)
            .id("library-search")
            .track_focus(&self.library_focus)
            .on_key_down(cx.listener(Self::on_library_key))
            .on_click(cx.listener(|this, _, window, cx| window.focus(&this.library_focus, cx)))
            .child(library.search.render(
                self.library_focus.is_focused(window),
                if downloads { "Search downloads" } else { "Search archived tabs" },
                theme.ink,
                ink_soft(theme),
                Hsla { a: 0.3, ..theme.accent },
            ));
        let filter = downloads.then(|| {
            let key = ("library-filter", 0);
            let h = self.motion.get(key, Feel::Hover);
            let label = KINDS.iter().find(|(k, _)| *k == library.filter && *k != Kind::All).map_or("All files", |(_, l)| *l);
            feel(
                div()
                    .id("library-filter")
                    .h(px(32.))
                    .px(px(9.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(5.))
                    .rounded(r(8.))
                    .bg(mix(theme.hover, theme.glass_strong, 0.5 * h))
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.ink)
                    .on_click(cx.listener(|this, e: &gpui::ClickEvent, window, cx| {
                        this.open_menu(MenuKind::LibraryFilter, e.position(), window, cx)
                    }))
                    .child(icon("list-filter", 13., theme.ink_muted))
                    .child(label)
                    .child(icon("chevron-down", 12., ink_soft(theme))),
                key,
                cx,
            )
        });
        let list = if downloads { self.download_rows(library, theme, cx) } else { self.archive_rows(library, theme, cx) };
        let empty = list.is_empty();
        let banner = (!downloads && self.settings.archive_after_hours > 0).then(|| self.archive_banner(theme, cx));
        let (summary, action) = if downloads {
            let n = self.saved_downloads.len();
            let clear = (n > 0).then(|| {
                fold_action("library-clear", "Clear list", theme)
                    .on_click(cx.listener(|this, _, _, cx| this.clear_downloads(cx)))
                    .into_any_element()
            });
            (plural(n, "file"), clear)
        } else {
            (plural(self.session.archived().len(), "tab"), None)
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_none().px(px(13.)).py(px(10.)).flex().gap(px(8.)).child(search).children(filter))
            .child(fold_rule(theme))
            .child(
                div()
                    .id("library-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(9.))
                    .pb(px(6.))
                    .flex()
                    .flex_col()
                    .children(banner)
                    .children(list)
                    .when(empty, |d| {
                        d.child(
                            div()
                                .pt(px(24.))
                                .flex()
                                .justify_center()
                                .text_size(px(13.))
                                .text_color(theme.ink_muted)
                                .child(if downloads { "No downloads" } else { "No archived tabs" }),
                        )
                    }),
            )
            .child(self.fold_footer(summary, action, theme, cx))
            .into_any_element()
    }

    /// Tabs land here without being asked, so the list says why.
    fn archive_banner(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let after = match self.settings.archive_after_hours {
            24 => "a day".to_string(),
            168 => "a week".to_string(),
            h => format!("{h}h"),
        };
        div()
            .flex_none()
            .mt(px(8.))
            .h(px(33.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(r(9.))
            .bg(fade(theme.accent, 0.12))
            .text_size(px(12.))
            .child(icon("hourglass", 15., theme.accent))
            .child(div().flex_1().min_w_0().truncate().text_color(theme.ink_muted).child(format!("Tabs fold away after {after} idle")))
            .child(
                fold_action("library-archive-change", "Change", theme)
                    .on_click(cx.listener(|this, _, _, cx| this.open_settings_at(Some(crate::settings::Tab::General), Some("archive"), cx))),
            )
            .into_any_element()
    }

    /// A row: thumbnail, title, detail. `dim` for files no longer on disk.
    fn library_row(
        &self,
        key: crate::motion::Key,
        thumb: AnyElement,
        (title, detail): (String, String),
        dim: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
        let el = div()
            .id(key)
            .h(px(52.))
            .px(px(10.))
            .mx(px(2. * p))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .rounded(r(10.))
            .bg(fade(theme.hover, h))
            .opacity(if dim { 0.5 } else { 1. })
            .child(div().size(px(30.)).flex_none().flex().items_center().justify_center().child(thumb))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().truncate().text_size(px(13.)).font_weight(gpui::FontWeight::SEMIBOLD).child(title))
                    .child(div().truncate().text_size(px(11.5)).text_color(ink_soft(theme)).child(detail)),
            );
        feel(el, key, cx)
    }

    fn download_rows(&self, library: &Library, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let query = library.search.text.trim();
        let keep = |path: &str, url: &str| {
            (library.filter == Kind::All || file_kind(path).0 == library.filter)
                && matches(query, &[&file_name(path), url])
        };
        let mut rows = Vec::new();
        let running: Vec<baa_web::Download> = self
            .host
            .downloads()
            .read(cx)
            .items
            .iter()
            .filter(|d| d.state == DownloadState::InProgress && keep(&d.path, &d.url))
            .cloned()
            .collect();
        for d in running {
            let id = d.id;
            let fraction = (d.total_bytes > 0).then(|| d.received_bytes as f32 / d.total_bytes as f32);
            let (_, _, icon_name) = file_kind(&d.path);
            let row = self
                .library_row(
                    ("library-running", id),
                    icon(icon_name, 19., theme.ink_muted).into_any_element(),
                    (file_name(&d.path), crate::prompts::bytes(d.received_bytes)),
                    false,
                    theme,
                    cx,
                )
                .child(
                    div()
                        .id(("library-cancel", id))
                        .size(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(r(6.))
                        .hover(|s| s.bg(theme.glass_strong))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.host.cancel_download(id);
                        }))
                        .child(icon("x", 13., theme.ink_muted)),
                );
            let card = raised(div().flex().flex_col().rounded(r(11.)), theme)
                .child(row)
                .child(
                    div()
                        .mx(px(50.))
                        .mb(px(8.))
                        .h(px(4.))
                        .rounded(r(2.))
                        .bg(theme.divider)
                        .child(div().h_full().w(relative(fraction.unwrap_or(0.3).clamp(0.03, 1.))).rounded(r(2.)).bg(theme.accent)),
                );
            rows.push(("Downloading".to_string(), card.into_any_element()));
        }
        let now = now() as i64;
        for (i, d) in self.saved_downloads.iter().enumerate().filter(|(_, d)| keep(&d.path, &d.url)) {
            let path = PathBuf::from(&d.path);
            let present = path.exists();
            let (kind, _, icon_name) = file_kind(&d.path);
            let fallback = move || icon(icon_name, 19., Hsla { a: 0.6, ..gpui::black() }).into_any_element();
            let thumb = if kind == Kind::Images && present {
                img(path.clone())
                    .size(px(30.))
                    .rounded(r(6.))
                    .object_fit(gpui::ObjectFit::Cover)
                    .with_fallback(fallback)
                    .into_any_element()
            } else {
                icon(icon_name, 19., theme.ink_muted).into_any_element()
            };
            let detail = if present { detail(&d.path, &d.url) } else { "Deleted".into() };
            let (open, menu_path, url) = (path.clone(), d.path.clone(), d.url.clone());
            let row = self
                .library_row(("library-download", i as u64), thumb, (file_name(&d.path), detail), !present, theme, cx)
                .on_click(cx.listener(move |_, _, _, cx| {
                    if open.exists() {
                        cx.open_with_system(&open)
                    }
                }))
                .on_mouse_down(
                    gpui::MouseButton::Right,
                    cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.open_menu(MenuKind::Download { path: menu_path.clone(), url: url.clone() }, e.position, window, cx);
                    }),
                );
            rows.push((ago(now, d.finished_at), row.into_any_element()));
        }
        grouped(rows, theme)
    }

    fn archive_rows(&self, library: &Library, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let query = library.search.text.trim();
        let now = now() as i64;
        let tabs: Vec<(TabId, String, String, Option<String>, i64)> = self
            .session
            .archived()
            .iter()
            .filter(|t| matches(query, &[tab_title(t), &t.url]))
            .map(|t| {
                let at = t.archived_at.unwrap_or(t.last_active) as i64;
                (t.id, tab_title(t).to_string(), t.url.clone(), t.favicon.clone(), at)
            })
            .collect();
        let mut rows = Vec::new();
        for (id, title, url, favicon, at) in tabs {
            // The mock's site tile: the favicon on a raised square.
            let thumb = div()
                .size(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(r(8.))
                .bg(theme.glass_strong)
                .child(
                    favicon
                        .and_then(|f| self.favicons.get(&f).cloned())
                        .map(|image| img(image).size(px(18.)).rounded(r(4.)).into_any_element())
                        .unwrap_or_else(|| icon("globe", 18., theme.ink_muted).into_any_element()),
                )
                .into_any_element();
            let row = self
                .library_row(("library-archived", id.0), thumb, (title, host_of(&url)), false, theme, cx)
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(tab) = this.session.restore_archived(id, now as u64) {
                        this.collapse_library();
                        this.activate_tab(tab, window, cx);
                    }
                }));
            rows.push((ago(now, at), row.into_any_element()));
        }
        grouped(rows, theme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_like_arc() {
        let day = 86_400;
        assert_eq!(ago(10 * day, 10 * day), "Today");
        assert_eq!(ago(10 * day, 9 * day), "Yesterday");
        assert_eq!(ago(10 * day, 7 * day), "3 days ago");
        assert_eq!(ago(30 * day, 22 * day), "1 week ago");
        assert_eq!(ago(100 * day, 30 * day), "2 months ago");
    }

    #[test]
    fn downloads_say_what_and_where_from() {
        assert_eq!(detail("/d/hachiman.zip", "https://release-assets.github.com/x"), "Zip Archive from release-assets.github.com");
        assert_eq!(detail("/d/r.ics", "blob:https://a.example/1"), "Calendar Event");
        assert_eq!(file_kind("/d/A.JPEG").0, Kind::Images);
        assert!(matches("hach zip", &["hachiman-macos.zip", "https://x"]));
    }
}
