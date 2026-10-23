//! ⌘T / ⌘L command bar (PLAN.md §3, pen.dev "Arc — Command Bar"): URL or
//! search with the engine's suggestions as you type, fuzzy matches over open
//! tabs, Spaces and quick commands, each with its keyboard shortcut.

use crate::theme::r;
use baa_model::{parse_suggestions, HistoryItem, Session, Settings, SpaceId, TabId};
use baa_web::WebHost;
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
    /// Chrome's settings page at this address, in its sheet.
    ChromeSettings(&'static str),
}

/// In the order an empty bar lists them.
const COMMANDS: [(Command, &str); 34] = [
    (Command::NewSpace, "New Space"),
    (Command::ToggleSidebar, "Toggle Sidebar"),
    (Command::CopyUrl, "Copy URL"),
    (Command::Find, "Find in Page"),
    (Command::ReopenClosedTab, "Reopen Closed Tab"),
    (Command::NewIncognitoSpace, "New Incognito Space"),
    (Command::NewFolder, "New Folder"),
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
    (Command::ClearHistory, "Clear Browsing History"),
    (Command::Settings, "Settings"),
    (Command::ImportFromArc, "Import Spaces from Arc"),
    (Command::SetUpPasswords, "Password Manager Extensions"),
    (Command::ChromeSettings("chrome://settings/privacy"), "Privacy and Security"),
    (Command::ChromeSettings("chrome://settings/content"), "Site Settings for All Sites"),
    (Command::ChromeSettings("chrome://settings/clearBrowserData"), "Clear Browsing Data…"),
    (Command::ChromeSettings("chrome://settings/autofill"), "Autofill and Passwords"),
    (Command::ChromeSettings("chrome://settings/languages"), "Languages and Spell Check"),
    (Command::ChromeSettings("chrome://settings/performance"), "Performance"),
    (Command::ChromeSettings("chrome://settings"), "Chrome Settings"),
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
            Command::ChromeSettings(_) => "settings",
            _ => "sliders-horizontal",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// Go to this URL (typed, or a search).
    Open(String),
    Tab(TabId),
    /// A Space, with its position for the ⌃n shortcut.
    Space(SpaceId, usize),
    Command(Command),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
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
    /// The engine's suggestions for `field`, once they arrive.
    pub suggestions: Vec<String>,
    /// Visited pages matching `field`.
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
            suggestions: Vec::new(),
            history: Vec::new(),
            seq: 0,
        }
    }

    fn rows(&self, session: &Session, settings: &Settings) -> Vec<Row> {
        if self.mode == Mode::History {
            return self.history.iter().take(MAX_ROWS).map(history_row).collect();
        }
        rows(&self.field.text, session, settings, &self.suggestions, &self.history)
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
/// URL or search first, the engine's suggestions, then the best fuzzy matches
/// across tabs (all Spaces), Spaces and commands.
pub fn rows(
    query: &str,
    session: &Session,
    settings: &Settings,
    suggestions: &[String],
    history: &[HistoryItem],
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

    let mut out = Vec::new();
    let url = settings.resolve_input(query);
    let search_prefix = settings.search_engine.url.split("%s").next().unwrap_or_default();
    if url.starts_with(search_prefix) {
        out.push(row(Item::Open(url), query, format!("— Search {}", settings.search_engine.name)));
    } else {
        out.push(row(Item::Open(url.clone()), url, "— Open"));
    }
    out.extend(suggestions.iter().take(MAX_SUGGESTIONS).map(|s| Row {
        suggestion: true,
        ..row(Item::Open(settings.resolve_input(s)), s.clone(), "")
    }));
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
    // Stable sort keeps sidebar order among equal scores.
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    // Visited pages come after open tabs and commands: those are one step away.
    let room = MAX_ROWS.saturating_sub(out.len() + MAX_HISTORY.min(history.len()));
    out.extend(scored.into_iter().map(|(_, row)| row).take(room));
    // Visited pages, unless already open as a tab (listed above) or typed.
    let open: Vec<&str> = session
        .spaces()
        .iter()
        .flat_map(|s| session.space_tabs(s.id))
        .map(|t| t.url.as_str())
        .collect();
    let typed = out[0].item.clone();
    out.extend(
        history
            .iter()
            .filter(|h| !open.contains(&h.url.as_str()) && Item::Open(h.url.clone()) != typed)
            .take(MAX_HISTORY)
            .map(history_row),
    );
    out.truncate(MAX_ROWS);
    out
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
                request_suggestions(this, cx);
            }
        }
        Key::Up => bar.selected = bar.selected.saturating_sub(1),
        Key::Down => bar.selected = (bar.selected + 1).min(count.saturating_sub(1)),
        Key::Cancel => this.close_overlays(window, cx),
        Key::Submit => submit(this, None, window, cx),
        Key::Ignored => {}
    }
}

/// History for what is typed (or, in History mode, the latest visits).
pub(crate) fn refresh_history<H: WebHost>(this: &mut Browser<H>) {
    let Some(bar) = &this.bar else { return };
    let query = bar.field.text.trim().to_string();
    let found = match (this.store(), bar.mode) {
        (Some(store), Mode::History) if query.is_empty() => store.recent_history(MAX_ROWS),
        (Some(store), Mode::History) => store.search_history(&query, MAX_ROWS),
        (Some(store), _) if !query.is_empty() => store.search_history(&query, MAX_HISTORY + 4),
        _ => Ok(Vec::new()),
    };
    if let Some(bar) = &mut this.bar {
        bar.history = found.unwrap_or_else(|e| {
            log::error!("reading history: {e}");
            Vec::new()
        });
    }
}

/// Asks the search engine for suggestions once typing pauses; answers for
/// text that has since changed are dropped.
fn request_suggestions<H: WebHost>(this: &mut Browser<H>, cx: &mut Context<Browser<H>>) {
    let profile = this.active_space().map(|s| s.profile);
    let Some(bar) = &mut this.bar else { return };
    bar.seq += 1;
    let seq = bar.seq;
    let query = bar.field.text.trim().to_string();
    let url = (this.settings.search_suggest && !query.is_empty() && !query.contains("://"))
        .then(|| this.settings.suggest_url(&query))
        .flatten();
    let (Some(url), Some(profile)) = (url, profile) else {
        bar.suggestions.clear();
        return;
    };
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(SUGGEST_DELAY).await;
        let async_cx = cx.clone();
        let weak = this.clone();
        let _ = this.update(cx, |browser, _| {
            if browser.bar.as_ref().is_none_or(|bar| bar.seq != seq) {
                return;
            }
            browser.host.fetch(
                profile,
                &url,
                Box::new(move |body| {
                    let found = body
                        .map(|body| parse_suggestions(&body, &query, MAX_SUGGESTIONS))
                        .unwrap_or_default();
                    // Later, not inside whatever update called us back.
                    async_cx
                        .spawn(async move |cx| {
                            let _ = weak.update(cx, |browser, cx| {
                                if let Some(bar) = browser.bar.as_mut().filter(|bar| bar.seq == seq) {
                                    bar.suggestions = found;
                                    cx.notify();
                                }
                            });
                        })
                        .detach();
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
        Item::Space(id, _) => this.activate_space(id, window, cx),
    }
}

impl<H: WebHost> Browser<H> {
    pub(crate) fn run_command(&mut self, c: Command, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(action) = c.action() {
            return window.dispatch_action(action, cx);
        }
        match c {
            Command::NewFolder => self.new_folder(None, window, cx),
            Command::RenameSpace => self.start_rename(window, cx),
            Command::ChangeSpaceColor => self.cycle_space_color(window, cx),
            Command::DeleteSpace => self.delete_active_space(window, cx),
            Command::SwapSidebarSide => self.swap_sidebar_side(cx),
            Command::Extensions => self.open_extensions("chrome://extensions/", window, cx),
            Command::ExtensionStore => {
                self.close_overlays(window, cx);
                self.open_url("https://chromewebstore.google.com/", window, cx)
            }
            Command::TranslatePage => self.translate_page(window, cx),
            Command::SiteSettings => self.toggle_popover(crate::prompts::Popover::Site, window, cx),
            Command::ClearHistory => self.clear_history(cx),
            Command::ImportFromArc => self.import_arc(window, cx),
            Command::SetUpPasswords => self.set_up_passwords(window, cx),
            Command::ChromeSettings(url) => {
                self.open_own_page(url, window, cx);
            }
            // The rest have actions, dispatched above.
            _ => {}
        }
    }
}

/// A shortcut as the mock draws it: one chip per modifier, then the key.
fn chips(binding: &gpui::KeyBinding) -> Vec<SharedString> {
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
                Item::Space(id, _) => div()
                    .w(px(18.))
                    .text_size(px(14.))
                    .child(SharedString::from(
                        self.session.space(*id).map(|s| s.icon.clone()).unwrap_or_default(),
                    ))
                    .into_any_element(),
                Item::Command(c) => icon(c.icon(), 18., icon_color).into_any_element(),
                Item::Open(_) if row.history => icon("history", 18., icon_color).into_any_element(),
                Item::Open(_) if row.suggestion || i == 0 && row.detail.starts_with("— Search") => {
                    icon("search", 18., icon_color).into_any_element()
                }
                Item::Open(_) => icon("globe", 18., icon_color).into_any_element(),
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
                Item::Space(..) => "Switch",
                Item::Command(_) => "Run",
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

    #[test]
    fn empty_query_lists_recent_tabs_then_quick_commands() {
        let (s, gh, ls) = session();
        let r = rows("", &s, &Settings::default(), &[], &[]);
        assert_eq!(r[0].item, Item::Tab(ls), "most recent first");
        assert_eq!(r[1].item, Item::Tab(gh));
        assert_eq!(r[2].item, Item::Command(Command::NewSpace));
        assert_eq!(r.len(), 2 + QUICK_COMMANDS);
    }

    #[test]
    fn typed_search_then_suggestions_then_best_tab() {
        let (s, gh, _) = session();
        let suggestions = ["gh cli".to_string(), "gh".to_string()];
        let r = rows("gh", &s, &Settings::default(), &suggestions, &[]);
        assert_eq!(r[0].item, Item::Open("https://www.google.com/search?q=gh".into()));
        assert_eq!((r[0].label.as_str(), r[0].detail.as_str()), ("gh", "— Search Google"));
        assert!(r[1].suggestion);
        assert_eq!(r[1].item, Item::Open("https://www.google.com/search?q=gh+cli".into()));
        assert!(r.iter().any(|r| r.item == Item::Tab(gh)));
    }

    #[test]
    fn urls_spaces_and_commands() {
        let (s, ..) = session();
        assert_eq!(rows("example.com", &s, &Settings::default(), &[], &[])[0].label, "https://example.com/");
        let r = rows("new space", &s, &Settings::default(), &[], &[]);
        assert!(r.iter().any(|r| r.item == Item::Command(Command::NewSpace)));
        let r = rows("personal", &s, &Settings::default(), &[], &[]);
        assert!(r.iter().any(|r| matches!(r.item, Item::Space(_, 0))));
    }

    #[test]
    fn history_lists_visited_pages_that_are_not_open() {
        let (s, ..) = session();
        let visit = |url: &str, title: &str| HistoryItem { url: url.into(), title: title.into(), visits: 1, last_visit: 0 };
        let history = [visit("https://github.com", "GitHub"), visit("https://docs.rs/gpui", "gpui docs")];
        let r = rows("g", &s, &Settings::default(), &[], &history);
        let from_history: Vec<&str> = r.iter().filter(|r| r.history).map(|r| r.label.as_str()).collect();
        assert_eq!(from_history, ["gpui docs"], "GitHub is open as a tab already");
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
}
