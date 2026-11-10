//! ⌘T / ⌘L command bar (PLAN.md §3, pen.dev "Arc — Command Bar"): URL or
//! search with the engine's suggestions as you type, fuzzy matches over open
//! tabs, Spaces and quick commands, each with its keyboard shortcut.

use crate::theme::r;
use crate::{
    pages::{self, Page},
    settings::Row as SettingRow,
};
use baa_model::{HistoryItem, Session, Settings, SpaceId, TabId};
use baa_web::{Suggestion, WebHost};
use gpui::{div, prelude::*, px, relative, Context, Hsla, SharedString, Window};

use crate::{
    actions,
    browser::{feel, host_of, shadow, tab_title, Browser},
    fuzzy,
    icons::icon,
    input::{Key, TextField},
    motion::{Feel, Spring},
    theme::{fade, mix, surface, Theme},
};

const MAX_ROWS: usize = 9;
const MAX_SUGGESTIONS: usize = 4;
/// Pages listed under a typed address.
const MAX_PAGES: usize = 6;
/// Pages and settings rows that join a plain-word search, each.
const MAX_EXTRA: usize = 3;
/// Visited pages listed under a typed query.
const MAX_HISTORY: usize = 3;
/// Typing pauses this long before asking the search engine.
const SUGGEST_DELAY: std::time::Duration = std::time::Duration::from_millis(120);
const WIDTH: f32 = 660.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// ⌘T: Enter opens a new tab.
    NewTab,
    /// ⌘L: prefilled with the current URL; Enter navigates the current tab.
    Address,
    /// ⌘Y: searches pages visited before; Enter opens one in a new tab.
    History,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    NewSpace,
    NewFolder,
    NewLiveFolder,
    NewIncognitoSpace,
    ToggleSidebar,
    CopyUrl,
    Find,
    ReopenClosedTab,
    RenameSpace,
    ChangeSpaceColor,
    DeleteSpace,
    SwapSidebarSide,
    DevTools,
    Extensions,
    ExtensionStore,
    Print,
    ViewSource,
    ReaderMode,
    PictureInPicture,
    TranslatePage,
    MuteTab,
    SiteSettings,
    Downloads,
    History,
    ClearHistory,
    Settings,
    ImportFromArc,
    SetUpPasswords,
    ClearToday,
    Archive,
    SplitView,
    ResponsiveView,
}

/// In the order an empty bar lists them.
const COMMANDS: [(Command, &str); 32] = [
    (Command::NewSpace, "New Space"),
    (Command::ToggleSidebar, "Toggle Sidebar"),
    (Command::CopyUrl, "Copy URL"),
    (Command::Find, "Find in Page"),
    (Command::ReopenClosedTab, "Reopen Closed Tab"),
    (Command::NewIncognitoSpace, "New Incognito Space"),
    (Command::NewFolder, "New Folder"),
    (Command::NewLiveFolder, "New Live Folder from This Page"),
    (Command::RenameSpace, "Rename Space"),
    (Command::ChangeSpaceColor, "Change Space Color"),
    (Command::SwapSidebarSide, "Move Sidebar to Other Side"),
    (Command::DevTools, "Developer Tools"),
    (Command::Extensions, "Extensions"),
    (Command::ExtensionStore, "Chrome Web Store"),
    (Command::DeleteSpace, "Delete Space"),
    (Command::Print, "Print…"),
    (Command::ViewSource, "View Page Source"),
    (Command::ReaderMode, "Reader Mode"),
    (Command::PictureInPicture, "Picture in Picture"),
    (Command::TranslatePage, "Translate Page"),
    (Command::MuteTab, "Mute or Unmute Tab"),
    (Command::SiteSettings, "Site Settings"),
    (Command::Downloads, "Downloads"),
    (Command::History, "History"),
    (Command::ClearHistory, "Clear Browsing Data…"),
    (Command::Settings, "Settings"),
    (Command::ImportFromArc, "Import Spaces from Arc"),
    (Command::SetUpPasswords, "Password Manager Extensions"),
    (Command::ClearToday, "Clear Today"),
    (Command::Archive, "Archived Tabs"),
    (Command::SplitView, "Split View"),
    (Command::ResponsiveView, "Responsive View"),
];
/// How many an empty bar shows, after recent tabs.
const QUICK_COMMANDS: usize = 6;

impl Command {
    /// The action it runs when it has one, for its shortcut chips.
    fn action(self) -> Option<Box<dyn gpui::Action>> {
        Some(match self {
            Command::NewSpace => Box::new(actions::NewSpace),
            Command::NewIncognitoSpace => Box::new(actions::NewIncognitoSpace),
            Command::ToggleSidebar => Box::new(actions::ToggleSidebar),
            Command::CopyUrl => Box::new(actions::CopyUrl),
            Command::Find => Box::new(actions::Find),
            Command::ReopenClosedTab => Box::new(actions::ReopenClosedTab),
            Command::DevTools => Box::new(actions::DevTools),
            Command::Print => Box::new(actions::Print),
            Command::ViewSource => Box::new(actions::ViewSource),
            Command::ReaderMode => Box::new(actions::ReaderMode),
            Command::PictureInPicture => Box::new(actions::PictureInPicture),
            Command::MuteTab => Box::new(actions::MuteTab),
            Command::Downloads => Box::new(actions::ShowDownloads),
            Command::History => Box::new(actions::ShowHistory),
            Command::Settings => Box::new(actions::OpenSettings),
            Command::ClearToday => Box::new(actions::ClearToday),
            Command::Archive => Box::new(actions::ShowArchive),
            Command::SplitView => Box::new(actions::SplitView),
            Command::ResponsiveView => Box::new(actions::ResponsiveView),
            _ => return None,
        })
    }

    fn icon(self) -> &'static str {
        match self {
            Command::NewSpace | Command::NewIncognitoSpace => "plus",
            Command::ToggleSidebar | Command::SwapSidebarSide => "panel-left",
            Command::CopyUrl => "link",
            Command::Find => "search",
            Command::ReopenClosedTab => "rotate-cw",
            Command::NewFolder => "folder",
            Command::NewLiveFolder => "rss",
            Command::DeleteSpace => "x",
            Command::RenameSpace | Command::ChangeSpaceColor => "house",
            Command::Print => "printer",
            Command::ViewSource | Command::DevTools => "code-xml",
            Command::ReaderMode => "book-open",
            Command::PictureInPicture => "picture-in-picture-2",
            Command::TranslatePage => "languages",
            Command::MuteTab => "volume-2",
            Command::SiteSettings => "lock",
            Command::Downloads => "arrow-down-to-line",
            Command::History => "history",
            Command::ClearHistory => "trash-2",
            Command::Settings => "settings",
            Command::ImportFromArc => "import",
            Command::SetUpPasswords => "key-round",
            Command::ClearToday | Command::Archive => "archive",
            Command::SplitView | Command::ResponsiveView => "columns-2",
            _ => "sliders-horizontal",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Item {
    /// Go to this URL (typed, or a search).
    Open(String),
    Tab(TabId),
    /// An archived tab; restored to the top of its Space's Today.
    Archived(TabId),
    /// A Space, with its position for the ⌃n shortcut.
    Space(SpaceId, usize),
    Command(Command),
    /// A first-party or WebUI page, found by its address.
    Page(&'static Page),
    /// A row of the settings window.
    Setting(&'static SettingRow),
    /// `env <label>`: the active tab on this env of its project.
    Env(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    pub item: Item,
    pub label: String,
    pub detail: String,
    /// A search engine suggestion (drawn with the search icon).
    pub suggestion: bool,
    /// A page from history (drawn with the clock).
    pub history: bool,
}

pub struct Bar {
    pub mode: Mode,
    pub field: TextField,
    pub selected: usize,
    pub scale: Spring,
    /// Chrome's omnibox matches for `field`, once they arrive (history, searches, calculator).
    pub matches: Vec<Suggestion>,
    /// History mode: visited pages matching `field`, from baa-model.
    pub history: Vec<HistoryItem>,
    /// Bumped per edit, so a late answer for older text is dropped.
    seq: u64,
}

impl Bar {
    pub fn new(mode: Mode, field: TextField) -> Self {
        let mut scale = Spring::new(0.0);
        scale.set_target(1.0);
        Self {
            mode,
            field,
            selected: 0,
            scale,
            matches: Vec::new(),
            history: Vec::new(),
            seq: 0,
        }
    }

    fn rows(&self, session: &Session, settings: &Settings) -> Vec<Row> {
        if self.mode == Mode::History {
            return self.history.iter().take(MAX_ROWS).map(history_row).collect();
        }
        rows(&self.field.text, session, settings, &self.matches)
    }
}

fn history_row(item: &HistoryItem) -> Row {
    let label = if item.title.is_empty() { &item.url } else { &item.title };
    Row {
        history: true,
        ..row(Item::Open(item.url.clone()), label.clone(), host_of(&item.url))
    }
}

fn row(item: Item, label: impl Into<String>, detail: impl Into<String>) -> Row {
    Row {
        item,
        label: label.into(),
        detail: detail.into(),
        suggestion: false,
        history: false,
    }
}

/// What the bar lists. Empty: recent tabs, then quick commands. Typed: the
/// URL or search first, Chrome's search matches, then the best fuzzy matches
/// across tabs (all Spaces), Spaces and commands, then Chrome's page matches.
pub(crate) fn rows(
    query: &str,
    session: &Session,
    settings: &Settings,
    matches: &[Suggestion],
) -> Vec<Row> {
    let query = query.trim();
    let active = session.active_space();
    let tab_row = |tab: &baa_model::Tab| {
        let space = session.space(tab.space);
        let host = host_of(&tab.url);
        let detail = match space {
            Some(s) if Some(s.id) != active => format!("{} {} · {host}", s.icon, s.name),
            _ => host,
        };
        row(Item::Tab(tab.id), tab_title(tab), detail)
    };
    if query.is_empty() {
        let mut tabs: Vec<&baa_model::Tab> = session.spaces().iter().flat_map(|s| session.space_tabs(s.id)).collect();
        tabs.sort_by_key(|t| std::cmp::Reverse(t.last_active));
        let mut out: Vec<Row> = tabs.into_iter().take(3).map(tab_row).collect();
        out.extend(
            COMMANDS
                .iter()
                .take(QUICK_COMMANDS)
                .map(|(c, label)| row(Item::Command(*c), *label, "")),
        );
        return out;
    }

    let mut out = env_rows(query, session, settings);
    if !out.is_empty() {
        return out;
    }
    let url = settings.resolve_input(query);
    // Typing an address: the pages it could be, then the plain "Open".
    let looks_like_page = pages::looks_like_page(query);
    if looks_like_page {
        out.extend(pages::complete(query, MAX_PAGES).into_iter().map(|p| row(Item::Page(p), p.label, p.url)));
        out.push(row(Item::Open(url.clone()), url, "— Open"));
        return out;
    }
    let search_prefix = settings.search_engine.url.split("%s").next().unwrap_or_default();
    if url.starts_with(search_prefix) {
        out.push(row(Item::Open(url), query, format!("— Search {}", settings.search_engine.name)));
    } else {
        out.push(row(Item::Open(url.clone()), url, "— Open"));
    }
    let typed = out[0].item.clone();
    let (searches, pages): (Vec<&Suggestion>, Vec<&Suggestion>) = matches.iter().partition(|m| m.search);
    out.extend(
        searches
            .into_iter()
            .filter(|m| !m.text.eq_ignore_ascii_case(query))
            .take(MAX_SUGGESTIONS)
            .map(|m| Row {
                suggestion: true,
                // A calculator result carries no URL: searching it is the best Chrome offers.
                ..row(
                    Item::Open(if m.url.is_empty() { settings.resolve_input(&m.text) } else { m.url.clone() }),
                    m.text.clone(),
                    m.description.clone(),
                )
            }),
    );
    let mut scored: Vec<(u32, Row)> = Vec::new();
    for space in session.spaces() {
        for tab in session.space_tabs(space.id) {
            let score = fuzzy::score(query, tab_title(tab)).max(fuzzy::score(query, &tab.url));
            if let Some(score) = score {
                scored.push((score, tab_row(tab)));
            }
        }
    }
    for (i, space) in session.spaces().iter().enumerate() {
        let label = format!("Switch to {}", space.name);
        if let Some(score) = fuzzy::score(query, &label) {
            scored.push((score, row(Item::Space(space.id, i), label, "")));
        }
    }
    for (command, label) in COMMANDS {
        if let Some(score) = fuzzy::score(query, label) {
            scored.push((score, row(Item::Command(command), label, "")));
        }
    }
    // After commands, so equal scores keep: tabs, Spaces, commands, pages, settings.
    let best = |mut hits: Vec<(u32, Row)>| {
        hits.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        hits.truncate(MAX_EXTRA);
        hits
    };
    // Pages that have a command of their own (Settings, Extensions, History, Downloads) are not listed twice.
    scored.extend(best(
        pages::all()
            .filter(|p| matches!(p.route, pages::Route::WebUi | pages::Route::Settings(..)) && p.url != "baa://settings")
            .filter_map(|p| {
                let score = fuzzy::score(query, p.label).max(fuzzy::score(query, p.keywords))?;
                Some((score, row(Item::Page(p), p.label, p.url)))
            })
            .collect(),
    ));
    scored.extend(best(
        crate::settings::rows::ROWS
            .iter()
            .filter_map(|r| Some((crate::settings::rows::score(query, r)?, row(Item::Setting(r), r.label, format!("Settings › {}", r.tab.title())))))
            .collect(),
    ));
    // Stable sort keeps sidebar order among equal scores.
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    // Visited pages come after open tabs and commands: those are one step away.
    let same = |a: &str, b: &str| a.trim_end_matches('/') == b.trim_end_matches('/');
    let open: Vec<&str> = session
        .spaces()
        .iter()
        .flat_map(|s| session.space_tabs(s.id))
        .map(|t| t.url.as_str())
        .collect();
    let pages: Vec<&Suggestion> = pages
        .into_iter()
        .filter(|m| !open.iter().any(|u| same(u, &m.url)) && !matches!(&typed, Item::Open(t) if same(t, &m.url)))
        .take(MAX_HISTORY)
        .collect();
    // Archived tabs rank below open ones: a match that is still open is not listed again.
    let mut archived: Vec<(u32, Row)> = session
        .archived()
        .iter()
        .filter(|t| !open.iter().any(|u| same(u, &t.url)))
        .filter_map(|t| {
            let score = fuzzy::score(query, tab_title(t)).max(fuzzy::score(query, &t.url))?;
            Some((score, row(Item::Archived(t.id), tab_title(t), format!("Archived · {}", host_of(&t.url)))))
        })
        .collect();
    archived.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    let room = MAX_ROWS.saturating_sub(out.len() + pages.len());
    // They take room from Spaces and commands, never from open tabs.
    let open_hits = scored.iter().filter(|(_, r)| matches!(r.item, Item::Tab(_))).count();
    archived.truncate(MAX_EXTRA.min(room.saturating_sub(open_hits)));
    out.extend(scored.into_iter().map(|(_, row)| row).take(room - archived.len()));
    out.extend(archived.into_iter().map(|(_, row)| row));
    out.extend(pages.into_iter().map(|m| {
        let label = if m.description.is_empty() { &m.url } else { &m.description };
        Row { history: true, ..row(Item::Open(m.url.clone()), label.clone(), host_of(&m.url)) }
    }));
    out.truncate(MAX_ROWS);
    out
}

/// `env <label>`: the other envs of the active tab's project whose label starts with `label`.
fn env_rows(query: &str, session: &Session, settings: &Settings) -> Vec<Row> {
    let query = query.to_lowercase();
    let Some(want) = (query == "env").then_some("").or_else(|| query.strip_prefix("env ")).map(str::trim) else {
        return Vec::new();
    };
    let tab = session.active_space().and_then(|id| session.space(id)?.active_tab).and_then(|id| session.tab(id));
    let Some((project, here)) = tab.and_then(|t| settings.env_of(&t.url)) else { return Vec::new() };
    project
        .envs
        .iter()
        .filter(|e| !std::ptr::eq(*e, here) && e.label.to_lowercase().starts_with(want))
        .map(|e| row(Item::Env(e.label.clone()), format!("Switch to {}", e.label), format!("{} · {}", project.name, e.origin)))
        .collect()
}

pub(crate) fn on_key<H: WebHost>(
    this: &mut Browser<H>,
    key: Key,
    window: &mut Window,
    cx: &mut Context<Browser<H>>,
) {
    let Some(bar) = &mut this.bar else { return };
    let count = bar.rows(&this.session, &this.settings).len();
    match key {
        Key::Edited => {
            bar.selected = 0;
            refresh_history(this);
            if this.bar.as_ref().is_some_and(|b| b.mode != Mode::History) {
                request_matches(this, cx);
            }
        }
        Key::Complete => {
            // The autofill: the selected page's address replaces what was typed.
            let rows = bar.rows(&this.session, &this.settings);
            if let Some(Item::Page(page)) = rows.get(bar.selected).map(|r| &r.item) {
                bar.field = TextField { text: page.url.into(), all_selected: false, masked: false };
                bar.selected = 0;
            }
        }
        Key::Up => bar.selected = bar.selected.saturating_sub(1),
        Key::Down => bar.selected = (bar.selected + 1).min(count.saturating_sub(1)),
        Key::Cancel => this.close_overlays(window, cx),
        Key::Submit => submit(this, None, window, cx),
        Key::Ignored => {}
    }
}

/// History mode's list: what is typed, or the latest visits.
pub(crate) fn refresh_history<H: WebHost>(this: &mut Browser<H>) {
    let Some(bar) = &this.bar else { return };
    let query = bar.field.text.trim().to_string();
    let found = match (this.store(), bar.mode) {
        (Some(store), Mode::History) if query.is_empty() => store.recent_history(MAX_ROWS),
        (Some(store), Mode::History) => store.search_history(&query, MAX_ROWS),
        _ => Ok(Vec::new()),
    };
    if let Some(bar) = &mut this.bar {
        bar.history = found.unwrap_or_else(|e| {
            log::error!("reading history: {e}");
            Vec::new()
        });
    }
}

/// Asks Chrome's omnibox once typing pauses; answers for text that has since
/// changed are dropped. Empty text asks too, which stops Chrome's last query.
/// No gate on `settings.search_suggest`: Chrome's `search.suggest_enabled` follows it
/// (PLAN-m3 WP-R2), and without it the omnibox only reads local history.
fn request_matches<H: WebHost>(this: &mut Browser<H>, cx: &mut Context<Browser<H>>) {
    let profile = this.active_space().map(|s| s.profile);
    let Some(bar) = &mut this.bar else { return };
    bar.seq += 1;
    let seq = bar.seq;
    let query = bar.field.text.trim().to_string();
    let Some(profile) = profile else { return };
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(SUGGEST_DELAY).await;
        let weak = this.clone();
        let _ = this.update(cx, |browser, _| {
            if browser.bar.as_ref().is_none_or(|bar| bar.seq != seq) {
                return;
            }
            browser.host.autocomplete(
                profile,
                &query,
                Box::new(move |matches, cx| {
                    let _ = weak.update(cx, |browser, cx| {
                        if let Some(bar) = browser.bar.as_mut().filter(|bar| bar.seq == seq) {
                            bar.matches = matches;
                            cx.notify();
                        }
                    });
                }),
            );
        });
    })
    .detach();
}

/// Run the selected row (or `row`, when clicked).
fn submit<H: WebHost>(
    this: &mut Browser<H>,
    row: Option<usize>,
    window: &mut Window,
    cx: &mut Context<Browser<H>>,
) {
    let Some(bar) = this.bar.as_ref() else { return };
    let mode = bar.mode;
    let rows = bar.rows(&this.session, &this.settings);
    let Some(row) = rows.into_iter().nth(row.unwrap_or(bar.selected)) else {
        return;
    };
    this.close_overlays(window, cx);
    match row.item {
        Item::Open(url) => this.navigate(&url, mode == Mode::Address, window, cx),
        Item::Command(c) => this.run_command(c, window, cx),
        Item::Tab(id) => this.activate_tab(id, window, cx),
        Item::Archived(id) => {
            this.session.restore_archived(id, baa_model::now());
            this.sync(window, cx);
        }
        Item::Space(id, _) => this.activate_space(id, window, cx),
        Item::Page(page) => this.navigate(page.url, mode == Mode::Address, window, cx),
        Item::Setting(r) => this.open_settings_at(Some(r.tab), Some(r.id), cx),
        Item::Env(label) => this.switch_env(Some(&label), window, cx),
    }
}

impl<H: WebHost> Browser<H> {
    pub(crate) fn run_command(&mut self, c: Command, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(action) = c.action() {
            return window.dispatch_action(action, cx);
        }
        match c {
            Command::NewFolder => self.new_folder(None, None, window, cx),
            Command::NewLiveFolder => {
                if let Some((name, source)) = self.live_source_here() {
                    self.new_live_folder(&name, source, window, cx);
                }
            }
            Command::RenameSpace => self.start_rename(window, cx),
            Command::ChangeSpaceColor => {
                if let Some(id) = self.session.active_space() {
                    self.cycle_color_of(id, window, cx);
                }
            }
            Command::DeleteSpace => {
                if let Some(id) = self.session.active_space() {
                    self.ask_delete_space(id, cx);
                }
            }
            Command::SwapSidebarSide => self.swap_sidebar_side(cx),
            Command::Extensions => {
                self.open_own_page("baa://extensions", window, cx);
            }
            Command::ExtensionStore => {
                self.close_overlays(window, cx);
                self.open_url("https://chromewebstore.google.com/", window, cx)
            }
            Command::TranslatePage => self.translate_page(window, cx),
            Command::SiteSettings => self.toggle_popover(crate::prompts::Popover::Site, window, cx),
            Command::ClearHistory => self.open_settings_at(Some(crate::settings::Tab::Privacy), Some("clear_data"), cx),
            Command::ImportFromArc => self.import_arc(window, cx),
            Command::SetUpPasswords => self.set_up_passwords(window, cx),
            // The rest have actions, dispatched above.
            _ => {}
        }
    }
}

/// A shortcut as the mock draws it: one chip per modifier, then the key.
pub(crate) fn chips(binding: &gpui::KeyBinding) -> Vec<SharedString> {
    let mut out = Vec::new();
    for stroke in binding.keystrokes() {
        let k = stroke.inner();
        let m = &k.modifiers;
        let mac = cfg!(target_os = "macos");
        for (on, mac_label, other) in [
            (m.control, "⌃", "Ctrl"),
            (m.alt, "⌥", "Alt"),
            (m.shift, "⇧", "Shift"),
            (m.platform, "⌘", "Win"),
        ] {
            if on {
                out.push(if mac { mac_label } else { other }.into());
            }
        }
        let key = match k.key.as_str() {
            "enter" => "↩".to_string(),
            "left" => "←".into(),
            "right" => "→".into(),
            "up" => "↑".into(),
            "down" => "↓".into(),
            "tab" => "⇥".into(),
            "escape" => "esc".into(),
            other => other.to_uppercase(),
        };
        out.push(key.into());
    }
    out
}

impl<H: WebHost> Browser<H> {
    /// `live` is false while the bar animates out: drawn, but deaf.
    pub(crate) fn render_command_bar(
        &self,
        bar: &Bar,
        live: bool,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = bar.scale.value();
        // Where the selection pill is, in rows; it slides rather than jumps.
        let pill = if live {
            self.motion.get(("bar-sel", 0), Feel::Glide)
        } else {
            bar.selected as f32
        };
        let rows = bar.rows(&self.session, &self.settings);
        let placeholder = match bar.mode {
            Mode::NewTab => "Search or enter address…",
            Mode::Address => "Enter address…",
            Mode::History => "Search history…",
        };
        let accent = theme.accent;
        let ascii = crate::theme::ascii();
        // ASCII Chrome selects with a navy bar and a cursor block, not an accent pill.
        let white = if ascii { theme.ink } else { gpui::white() };
        let list = rows.into_iter().enumerate().map(|(i, row)| {
            // How much the pill covers this row: its text turns white with it.
            let c = (1. - (pill - i as f32).abs()).clamp(0., 1.);
            let ink = mix(surface::ink().into(), white, c);
            let muted = mix(surface::muted().into(), Hsla { a: 0.82, ..white }, c);
            let icon_color = mix(surface::icon().into(), Hsla { a: 0.82, ..white }, c);
            let h = self.motion.get(("bar-row", i as u64), Feel::Hover);
            let leading = match &row.item {
                Item::Tab(id) => self
                    .session
                    .tab(*id)
                    .and_then(|tab| tab.favicon.as_ref())
                    .and_then(|url| self.favicons.get(url))
                    .map(|image| gpui::img(image.clone()).size(px(18.)).rounded(r(4.)).into_any_element())
                    .unwrap_or_else(|| icon("globe", 18., icon_color).into_any_element()),
                Item::Archived(_) => icon("archive", 18., icon_color).into_any_element(),
                Item::Space(id, _) => div()
                    .w(px(18.))
                    .text_size(px(14.))
                    .child(SharedString::from(
                        self.session.space(*id).map(|s| s.icon.clone()).unwrap_or_default(),
                    ))
                    .into_any_element(),
                Item::Command(c) => icon(c.icon(), 18., icon_color).into_any_element(),
                Item::Page(p) if matches!(p.route, pages::Route::WebUi) => icon("globe", 18., icon_color).into_any_element(),
                Item::Page(_) | Item::Setting(_) => icon("settings", 18., icon_color).into_any_element(),
                Item::Open(_) if row.history => icon("history", 18., icon_color).into_any_element(),
                Item::Open(_) if row.suggestion || i == 0 && row.detail.starts_with("— Search") => {
                    icon("search", 18., icon_color).into_any_element()
                }
                Item::Open(_) => icon("globe", 18., icon_color).into_any_element(),
                Item::Env(_) => icon("terminal", 18., icon_color).into_any_element(),
            };
            let binding = match &row.item {
                Item::Command(c) => c.action().and_then(|a| window.highest_precedence_binding_for_action(&*a)),
                Item::Space(_, n) => window.highest_precedence_binding_for_action(&actions::GoToSpace(*n)),
                _ => None,
            };
            let keys = binding.map(|b| chips(&b)).unwrap_or_default();
            let hint = match &row.item {
                Item::Open(_) if row.detail.starts_with("— Search") || row.suggestion => "Search",
                Item::Open(_) => "Open",
                Item::Tab(_) => "Switch to Tab",
                Item::Archived(_) => "Restore",
                Item::Space(..) => "Switch",
                Item::Command(_) => "Run",
                Item::Page(_) => "Open",
                Item::Setting(_) => "Show",
                Item::Env(_) => "Switch",
            };
            let row_el = div()
                .id(("bar-row", i))
                .h(px(42.))
                .px(px(12.))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(12.))
                .rounded(r(9.))
                .bg(fade(surface::key().into(), h * (1. - c)))
                .on_click(cx.listener(move |this, _, window, cx| submit(this, Some(i), window, cx)))
                .when(ascii, |d| d.relative().child(div().absolute().left(px(2.)).opacity(c).text_color(accent).child("▌")))
                .child(leading)
                .child(
                    div()
                        .flex_none()
                        .max_w(px(360.))
                        .truncate()
                        .text_size(px(14.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(ink)
                        .child(SharedString::from(row.label)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(14.))
                        .text_color(muted)
                        .child(SharedString::from(row.detail)),
                )
                .when(!keys.is_empty(), |d| {
                    d.child(div().flex().gap(px(4.)).children(keys.into_iter().map(|key| {
                        div()
                            .h(px(20.))
                            .min_w(px(20.))
                            .px(px(6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(r(5.))
                            .bg(mix(surface::key().into(), Hsla { a: 0.2, ..white }, c))
                            .text_size(px(11.5))
                            .text_color(mix(surface::icon().into(), white, c))
                            .child(key)
                    })))
                })
                .when(c > 0.01, |d| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .opacity(c)
                            .text_size(px(12.5))
                            .text_color(muted)
                            .child(hint)
                            // The arrow slides in with the pill.
                            .child(icon("arrow-right", 13., muted).with_transformation(
                                gpui::Transformation::translate(gpui::point(px(-6. * (1. - c)), px(0.))),
                            )),
                    )
                });
            feel(row_el, ("bar-row", i as u64), cx)
        }).collect::<Vec<_>>();
        let accent_pill = div()
            .absolute()
            .left(px(6.))
            .right(px(6.))
            .top(px(6. + 42. * pill))
            .h(px(42.))
            .rounded(r(9.))
            .bg(if ascii { theme.glass_strong } else { accent });

        let card = div()
            .id("command-bar")
            .when(live, |d| {
                d.track_focus(&self.input_focus)
                    .key_context("CommandBar")
                    .on_key_down(cx.listener(Self::on_input_key))
                    // Clicks inside must not reach the backdrop's close handler.
                    .on_click(|_, _, cx| cx.stop_propagation())
            });
        div()
            .id("command-bar-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            .items_start()
            // Drops in from a little above; leaves the same way.
            .pt(relative(0.17))
            .when(live, |d| {
                d.occlude()
                    .on_click(cx.listener(|this, _, window, cx| this.close_overlays(window, cx)))
            })
            .child(
                card
                        .mt(px(-14. * (1. - t)))
                        // Spring "scale-in": gpui has no transforms, so width and opacity carry it.
                        .w(px(WIDTH * (0.94 + 0.06 * t)))
                        .h_auto()
                        .max_h(px(480.))
                        .opacity(t.clamp(0.0, 1.0))
                        .flex()
                        .flex_col()
                        .rounded(r(14.))
                        .bg(surface::bg())
                        .border_1()
                        .border_color(surface::border())
                        .shadow(vec![
                            shadow(gpui::rgba(0x18122C52).into(), 30., 80.),
                            shadow(gpui::rgba(0x18122C1F).into(), 8., 24.),
                        ])
                        .text_color(surface::ink())
                        .when(ascii, |d| d.relative().border_color(gpui::transparent_black()).child(crate::theme::ascii::outline(14.)))
                        .child(
                            div()
                                .h(px(58.))
                                .px(px(20.))
                                .flex()
                                .items_center()
                                .text_size(px(19.))
                                .border_b_1()
                                .border_color(surface::line())
                                .when(ascii, |d| d.text_size(px(14.)).child(div().mr(px(8.)).text_color(accent).child("▸")))
                                .child(bar.field.render(
                                    placeholder,
                                    surface::ink().into(),
                                    surface::muted().into(),
                                    Hsla { a: 0.25, ..accent },
                                )),
                        )
                        .child(
                            div()
                                .relative()
                                .p(px(6.))
                                .flex()
                                .flex_col()
                                .child(accent_pill)
                                .children(list),
                        )
                        .when(ascii, |d| d.child(crate::theme::ascii::box_title("COMMAND", accent))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> (Session, TabId, TabId) {
        let mut s = Session::first_run();
        let sp = s.active_space().unwrap();
        let gh = s.open_tab(sp, "https://github.com", 1);
        s.tab_mut(gh).unwrap().title = "GitHub".into();
        let ls = s.open_tab(sp, "https://lightshow.example", 2);
        s.tab_mut(ls).unwrap().title = "lightshow".into();
        (s, gh, ls)
    }

    fn search(text: &str) -> Suggestion {
        Suggestion { url: String::new(), text: text.into(), description: String::new(), search: true }
    }

    fn page(url: &str, title: &str) -> Suggestion {
        Suggestion { url: url.into(), text: url.into(), description: title.into(), search: false }
    }

    #[test]
    fn empty_query_lists_recent_tabs_then_quick_commands() {
        let (s, gh, ls) = session();
        let r = rows("", &s, &Settings::default(), &[]);
        assert_eq!(r[0].item, Item::Tab(ls), "most recent first");
        assert_eq!(r[1].item, Item::Tab(gh));
        assert_eq!(r[2].item, Item::Command(Command::NewSpace));
        assert_eq!(r.len(), 2 + QUICK_COMMANDS);
    }

    #[test]
    fn typed_search_then_suggestions_then_best_tab() {
        let (s, gh, _) = session();
        let matches = [search("gh cli"), search("gh")];
        let r = rows("gh", &s, &Settings::default(), &matches);
        assert_eq!(r[0].item, Item::Open("https://www.google.com/search?q=gh".into()));
        assert_eq!((r[0].label.as_str(), r[0].detail.as_str()), ("gh", "— Search Google"));
        assert!(r[1].suggestion);
        assert_eq!(r[1].item, Item::Open("https://www.google.com/search?q=gh+cli".into()));
        assert!(r.iter().any(|r| r.item == Item::Tab(gh)));
    }

    #[test]
    fn urls_spaces_and_commands() {
        let (s, ..) = session();
        assert_eq!(rows("example.com", &s, &Settings::default(), &[])[0].label, "https://example.com/");
        let r = rows("new space", &s, &Settings::default(), &[]);
        assert!(r.iter().any(|r| r.item == Item::Command(Command::NewSpace)));
        let r = rows("personal", &s, &Settings::default(), &[]);
        assert!(r.iter().any(|r| matches!(r.item, Item::Space(_, 0))));
    }

    #[test]
    fn page_matches_become_history_rows_unless_open() {
        let (s, ..) = session();
        let matches = [search("gh cli"), page("https://github.com/", "GitHub"), page("https://docs.rs/gpui", "gpui docs")];
        let r = rows("gh", &s, &Settings::default(), &matches);
        assert!(r[0].detail.starts_with("— Search"));
        assert!(r[1].suggestion);
        let from_history: Vec<&str> = r.iter().filter(|r| r.history).map(|r| r.label.as_str()).collect();
        assert_eq!(from_history, ["gpui docs"], "GitHub is open as a tab already");
        assert!(r.last().unwrap().history);
    }

    #[test]
    fn archived_tabs_rank_below_open_ones_unless_still_open() {
        let (mut s, gh, ls) = session();
        let sp = s.active_space().unwrap();
        let old = s.open_tab(sp, "https://github.com/old", 3);
        s.tab_mut(old).unwrap().title = "GitHub old".into();
        let dup = s.open_tab(sp, "https://github.com", 4);
        s.close_tab(old, 5);
        s.close_tab(dup, 6);
        let r = rows("github", &s, &Settings::default(), &[]);
        let at = |item: Item| r.iter().position(|r| r.item == item);
        assert!(at(Item::Archived(old)).unwrap() > at(Item::Tab(gh)).unwrap());
        assert_eq!(at(Item::Archived(dup)), None, "GitHub is open as a tab already");
        assert_eq!(r[at(Item::Archived(old)).unwrap()].detail, "Archived · github.com");
        assert_eq!(at(Item::Tab(ls)), None);
    }

    #[test]
    fn shortcut_chips_split_modifiers() {
        let binding = gpui::KeyBinding::new("ctrl-shift-=", actions::ToggleSidebar, None);
        let expected: Vec<SharedString> = if cfg!(target_os = "macos") {
            vec!["⌃".into(), "⇧".into(), "=".into()]
        } else {
            vec!["Ctrl".into(), "Shift".into(), "=".into()]
        };
        assert_eq!(chips(&binding), expected);
    }

    #[test]
    fn env_lists_the_other_envs_of_the_tabs_project() {
        let (mut s, _, _) = session();
        let sp = s.active_space().unwrap();
        s.open_tab(sp, "https://staging.shop.com/cart?id=1", 3);
        let env = |label: &str, origin: &str| baa_model::Env { label: label.into(), origin: origin.into() };
        let settings = Settings {
            environments: vec![baa_model::Project {
                name: "shop".into(),
                envs: vec![env("Local", "http://localhost:3000"), env("Staging", "https://staging.shop.com"), env("Prod", "https://shop.com")],
            }],
            ..Default::default()
        };
        let r = rows("env ", &s, &settings, &[]);
        assert_eq!(r.iter().map(|r| r.item.clone()).collect::<Vec<_>>(), [Item::Env("Local".into()), Item::Env("Prod".into())]);
        assert_eq!(rows("ENV pr", &s, &settings, &[]).iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["Switch to Prod"]);
        assert!(!rows("env qa", &s, &settings, &[]).iter().any(|r| matches!(r.item, Item::Env(_))), "falls back to a search");
        assert!(!rows("env prod", &s, &Settings::default(), &[]).iter().any(|r| matches!(r.item, Item::Env(_))));
    }
}
