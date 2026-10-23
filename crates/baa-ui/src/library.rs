//! Arc's Library, in the sidebar: what lives outside any Space. Opened from
//! the archive button or by swiping right past the first Space. Sections:
//! Downloads (kept across launches, grouped by age, searchable and filtered by
//! kind) and Archived Tabs.

use crate::theme::r;
use std::path::{Path, PathBuf};

use baa_model::{now, DownloadRecord, TabId};
use baa_web::{DownloadState, WebHost};
use gpui::{div, img, prelude::*, px, relative, AnyElement, Context, Hsla, KeyDownEvent, Window};

use crate::{
    browser::{feel, host_of, tab_title, Browser, MenuKind, SIDEBAR_WIDTH},
    icons::icon,
    input::{Key, TextField},
    motion::Feel,
    theme::{fade, mix, Theme},
};

/// The sidebar while the Library is open: a section column and a list.
pub(crate) const LIBRARY_WIDTH: f32 = 384.0;
const NAV_W: f32 = 92.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Section {
    Downloads,
    Archive,
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

impl<H: WebHost> Browser<H> {
    /// The sidebar's width now: wider while the Library is open.
    pub(crate) fn sidebar_width(&self) -> f32 {
        if self.library.is_some() {
            LIBRARY_WIDTH
        } else {
            SIDEBAR_WIDTH
        }
    }

    pub(crate) fn open_library(&mut self, section: Section, cx: &mut Context<Self>) {
        match &mut self.library {
            Some(library) => library.section = section,
            None => {
                self.library = Some(Library { section, search: TextField::default(), filter: Kind::All });
                self.motion.from(("library", 0), Feel::Glide, 0.0, 1.0);
            }
        }
        if self.sidebar_open {
            self.sidebar_w.set_target(LIBRARY_WIDTH);
        }
        cx.notify();
    }

    /// Back to the Spaces; the first comes in from the right, as it lies.
    pub(crate) fn close_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.library.take().is_none() {
            return;
        }
        if self.sidebar_open {
            self.sidebar_w.set_target(SIDEBAR_WIDTH);
        }
        self.slide.snap(1.0);
        self.slide.set_target(0.0);
        self.close_overlays(window, cx);
    }

    /// Finished downloads go into the store, so the Library keeps them.
    pub(crate) fn remember_downloads(&mut self, items: &[baa_web::Download]) {
        for d in items.iter().filter(|d| d.state == DownloadState::Complete) {
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

    pub(crate) fn on_library_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(library) = &mut self.library else { return };
        match library.search.handle(&event.keystroke, cx) {
            Key::Cancel if library.search.text.is_empty() => self.close_library(window, cx),
            Key::Cancel => library.search = TextField::default(),
            _ => {}
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(crate) fn render_library(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(library) = &self.library else { return div().into_any_element() };
        let t = self.motion.get(("library", 0), Feel::Glide);
        let nav_item = |section: Section, name: &'static str, label: &'static str, cx: &mut Context<Self>| {
            let key = ("library-nav", section as u64);
            let (h, p) = (self.motion.get(key, Feel::Hover), self.motion.get(key, Feel::Press));
            let on = library.section == section;
            let el = div()
                .id(("library-nav", section as u64))
                .h(px(70.))
                .mx(px(2. * p))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .rounded(r(12.))
                .bg(if on { theme.glass_strong } else { fade(theme.hover, h) })
                .text_size(px(11.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if on { theme.ink } else { mix(theme.ink_muted, theme.ink, h) })
                .on_click(cx.listener(move |this, _, _, cx| this.open_library(section, cx)))
                .child(icon(name, 22., if on { theme.ink } else { mix(theme.ink_muted, theme.ink, h) }))
                .child(label);
            feel(el, key, cx)
        };
        let back = self
            .icon_button("library-back", "arrow-left", true, -2., 0., theme, cx)
            .on_click(cx.listener(|this, _, window, cx| this.close_library(window, cx)));
        let nav = div()
            .w(px(NAV_W))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .pb(px(4.))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(px(8.))
                    .child(nav_item(Section::Downloads, "circle-arrow-down", "Downloads", cx))
                    .child(nav_item(Section::Archive, "archive", "Archived Tabs", cx)),
            )
            .child(div().h(px(32.)).flex().items_center().child(back));

        let placeholder = match library.section {
            Section::Downloads => "Search Downloads…",
            Section::Archive => "Search Archived Tabs…",
        };
        let search = div()
            .id("library-search")
            .flex_1()
            .min_w_0()
            .h(px(34.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(r(10.))
            .bg(theme.glass)
            .text_size(px(13.))
            .track_focus(&self.library_focus)
            .on_key_down(cx.listener(Self::on_library_key))
            .on_click(cx.listener(|this, _, window, cx| window.focus(&this.library_focus, cx)))
            .child(icon("search", 15., theme.ink_muted))
            .child(library.search.render(placeholder, theme.ink, theme.ink_muted, Hsla { a: 0.3, ..theme.accent }));
        let filter = (library.section == Section::Downloads).then(|| {
            let key = ("library-filter", 0);
            let h = self.motion.get(key, Feel::Hover);
            let label = KINDS.iter().find(|(k, _)| *k == library.filter).map_or("Filter", |(k, l)| {
                if *k == Kind::All { "Filter" } else { l }
            });
            feel(
                div()
                    .id("library-filter")
                    .h(px(34.))
                    .px(px(10.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .rounded(r(10.))
                    .bg(mix(theme.glass, theme.glass_strong, 0.5 * h))
                    .text_size(px(13.))
                    .text_color(theme.ink)
                    .on_click(cx.listener(|this, e: &gpui::ClickEvent, _, cx| {
                        this.open_menu(MenuKind::LibraryFilter, e.position(), cx)
                    }))
                    .child(icon("list-filter", 15., theme.ink_muted))
                    .child(label),
                key,
                cx,
            )
        });
        let list = match library.section {
            Section::Downloads => self.download_rows(library, theme, cx),
            Section::Archive => self.archive_rows(library, theme, cx),
        };
        let empty = list.is_empty();
        let body = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().flex().gap(px(6.)).child(search).children(filter))
            .child(
                div()
                    .id("library-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(list)
                    .when(empty, |d| {
                        d.child(
                            div()
                                .pt(px(24.))
                                .flex()
                                .justify_center()
                                .text_size(px(13.))
                                .text_color(theme.ink_muted)
                                .child(match library.section {
                                    Section::Downloads => "No downloads",
                                    Section::Archive => "No archived tabs",
                                }),
                        )
                    }),
            );
        div()
            .w(px(LIBRARY_WIDTH))
            .h_full()
            .pl(px(10.))
            .pr(px(12.))
            .pt(px(4.))
            .flex()
            .gap(px(10.))
            // Comes in from the left, where it sits before the first Space.
            .ml(px(-40. * (1. - t)))
            .opacity(t.clamp(0., 1.))
            .on_scroll_wheel(cx.listener(|this, e, window, cx| this.on_chrome_scroll(e, window, cx)))
            .child(nav)
            .child(body)
            .into_any_element()
    }

    fn group_header(label: String, theme: &Theme) -> AnyElement {
        div()
            .pt(px(10.))
            .pb(px(4.))
            .px(px(8.))
            .text_size(px(12.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(theme.ink_muted)
            .child(label)
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
            .h(px(50.))
            .px(px(8.))
            .mx(px(2. * p))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .rounded(r(10.))
            .bg(fade(theme.hover, h))
            .opacity(if dim { 0.5 } else { 1. })
            .child(div().size(px(32.)).flex_none().flex().items_center().justify_center().child(thumb))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(div().truncate().text_size(px(13.5)).font_weight(gpui::FontWeight::MEDIUM).child(title))
                    .child(div().truncate().text_size(px(12.)).text_color(theme.ink_muted).child(detail)),
            );
        feel(el, key, cx)
    }

    fn download_rows(&self, library: &Library, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let query = library.search.text.trim();
        let keep = |path: &str, url: &str| {
            (library.filter == Kind::All || file_kind(path).0 == library.filter)
                && matches(query, &[&file_name(path), url])
        };
        let mut out = Vec::new();
        let running: Vec<baa_web::Download> = self
            .host
            .downloads()
            .read(cx)
            .items
            .iter()
            .filter(|d| d.state == DownloadState::InProgress && keep(&d.path, &d.url))
            .cloned()
            .collect();
        if !running.is_empty() {
            out.push(Self::group_header("Downloading".into(), theme));
        }
        for d in running {
            let id = d.id;
            let fraction = (d.total_bytes > 0).then(|| d.received_bytes as f32 / d.total_bytes as f32);
            let (_, _, icon_name) = file_kind(&d.path);
            let row = self
                .library_row(
                    ("library-running", id as u64),
                    icon(icon_name, 24., theme.ink_muted).into_any_element(),
                    (file_name(&d.path), crate::prompts::bytes(d.received_bytes)),
                    false,
                    theme,
                    cx,
                )
                .child(
                    div()
                        .id(("library-cancel", id as u64))
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
            out.push(
                div()
                    .flex()
                    .flex_col()
                    .child(row)
                    .child(
                        div()
                            .mx(px(50.))
                            .mb(px(4.))
                            .h(px(3.))
                            .rounded(r(2.))
                            .bg(theme.glass)
                            .child(div().h_full().w(relative(fraction.unwrap_or(0.3).clamp(0.03, 1.))).rounded(r(2.)).bg(theme.accent)),
                    )
                    .into_any_element(),
            );
        }
        let now = now() as i64;
        let mut last_group = String::new();
        for (i, d) in self.saved_downloads.iter().enumerate().filter(|(_, d)| keep(&d.path, &d.url)) {
            let group = ago(now, d.finished_at);
            if group != last_group {
                out.push(Self::group_header(group.clone(), theme));
                last_group = group;
            }
            let path = PathBuf::from(&d.path);
            let present = path.exists();
            let (kind, _, icon_name) = file_kind(&d.path);
            let fallback = move || icon(icon_name, 24., Hsla { a: 0.6, ..gpui::black() }).into_any_element();
            let thumb = if kind == Kind::Images && present {
                img(path.clone())
                    .size(px(30.))
                    .rounded(r(5.))
                    .object_fit(gpui::ObjectFit::Cover)
                    .with_fallback(fallback)
                    .into_any_element()
            } else {
                icon(icon_name, 24., theme.ink_muted).into_any_element()
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
                    cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_menu(MenuKind::Download { path: menu_path.clone(), url: url.clone() }, e.position, cx);
                    }),
                );
            out.push(row.into_any_element());
        }
        out
    }

    fn archive_rows(&self, library: &Library, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let query = library.search.text.trim();
        let now = now() as i64;
        let mut out = Vec::new();
        let mut last_group = String::new();
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
        for (id, title, url, favicon, at) in tabs {
            let group = ago(now, at);
            if group != last_group {
                out.push(Self::group_header(group.clone(), theme));
                last_group = group;
            }
            let thumb = favicon
                .and_then(|f| self.favicons.get(&f).cloned())
                .map(|image| img(image).size(px(18.)).rounded(r(4.)).into_any_element())
                .unwrap_or_else(|| icon("globe", 18., theme.ink_muted).into_any_element());
            let row = self
                .library_row(("library-archived", id.0), thumb, (title, host_of(&url)), false, theme, cx)
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(tab) = this.session.restore_archived(id, now as u64) {
                        this.library = None;
                        this.sidebar_w.set_target(if this.sidebar_open { SIDEBAR_WIDTH } else { 0. });
                        this.activate_tab(tab, window, cx);
                    }
                }));
            out.push(row.into_any_element());
        }
        out
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
