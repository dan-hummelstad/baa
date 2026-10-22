//! ⌘T / ⌘L command bar (PLAN.md §3): URL or search, fuzzy match over open tabs,
//! "switch to space", and Space commands.

use baa_model::{Session, Settings, SpaceId, TabId};
use baa_web::WebHost;
use gpui::{div, prelude::*, px, relative, Context, SharedString, Window};

use crate::{
    browser::{shadow, tab_title, Browser},
    fuzzy,
    icons::icon,
    input::{Key, TextField},
    motion::Spring,
    theme::{surface, Theme},
};

const MAX_ROWS: usize = 8;
const WIDTH: f32 = 660.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// ⌘T: Enter opens a new tab.
    NewTab,
    /// ⌘L: prefilled with the current URL; Enter navigates the current tab.
    Address,
    /// Text becomes the active Space's "icon name".
    RenameSpace,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    NewSpace,
    NewIncognitoSpace,
    RenameSpace,
    ChangeSpaceColor,
    DeleteSpace,
    SwapSidebarSide,
    ReopenClosedTab,
    Extensions,
    ExtensionStore,
}

const COMMANDS: [(Command, &str); 9] = [
    (Command::NewSpace, "New Space"),
    (Command::NewIncognitoSpace, "New Incognito Space"),
    (Command::RenameSpace, "Rename Space"),
    (Command::ChangeSpaceColor, "Change Space Color"),
    (Command::DeleteSpace, "Delete Space"),
    (Command::SwapSidebarSide, "Move Sidebar to Other Side"),
    (Command::ReopenClosedTab, "Reopen Closed Tab"),
    (Command::Extensions, "Extensions"),
    (Command::ExtensionStore, "Chrome Web Store"),
];

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Open(String),
    Tab(TabId),
    Space(SpaceId),
    Command(Command),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub item: Item,
    pub label: String,
    pub detail: String,
}

pub struct Bar {
    pub mode: Mode,
    pub field: TextField,
    pub selected: usize,
    pub scale: Spring,
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
        }
    }

    fn rows(&self, session: &Session, settings: &Settings) -> Vec<Row> {
        match self.mode {
            Mode::RenameSpace => Vec::new(),
            _ => rows(&self.field.text, session, settings),
        }
    }
}

/// What the bar lists for `query`: the typed URL/search first, then the best fuzzy
/// matches across tabs (all Spaces), Spaces and commands.
pub fn rows(query: &str, session: &Session, settings: &Settings) -> Vec<Row> {
    let query = query.trim();
    let mut out = Vec::new();
    let mut scored: Vec<(u32, Row)> = Vec::new();
    for space in session.spaces() {
        for tab in session.space_tabs(space.id) {
            let title = tab_title(tab);
            let score = fuzzy::score(query, title).max(fuzzy::score(query, &tab.url));
            if let Some(score) = score {
                let detail = format!("{} {} · {}", space.icon, space.name, tab.url);
                scored.push((
                    score,
                    Row {
                        item: Item::Tab(tab.id),
                        label: title.into(),
                        detail,
                    },
                ));
            }
        }
    }
    // Typed URL, Spaces and commands only once the user typed something, so ⌘T opens on tabs.
    if !query.is_empty() {
        let url = settings.resolve_input(query);
        let search_prefix = settings
            .search_engine
            .url
            .split("%s")
            .next()
            .unwrap_or_default();
        let label = if url.starts_with(search_prefix) {
            format!("Search “{query}”")
        } else {
            url.clone()
        };
        out.push(Row {
            item: Item::Open(url),
            label,
            detail: String::new(),
        });
        for space in session.spaces() {
            let label = format!("Switch to {} {}", space.icon, space.name);
            if let Some(score) = fuzzy::score(query, &label) {
                scored.push((
                    score,
                    Row {
                        item: Item::Space(space.id),
                        label,
                        detail: "Space".into(),
                    },
                ));
            }
        }
        for (command, label) in COMMANDS {
            if let Some(score) = fuzzy::score(query, label) {
                scored.push((
                    score,
                    Row {
                        item: Item::Command(command),
                        label: label.into(),
                        detail: "Command".into(),
                    },
                ));
            }
        }
    }
    // Stable sort keeps sidebar order among equal scores.
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    out.extend(scored.into_iter().map(|(_, row)| row).take(MAX_ROWS));
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
        Key::Edited => bar.selected = 0,
        Key::Up => bar.selected = bar.selected.saturating_sub(1),
        Key::Down => bar.selected = (bar.selected + 1).min(count.saturating_sub(1)),
        Key::Cancel => this.close_overlays(window, cx),
        Key::Submit => submit(this, None, window, cx),
        Key::Ignored => {}
    }
}

/// Run the selected row (or `row`, when clicked).
fn submit<H: WebHost>(
    this: &mut Browser<H>,
    row: Option<usize>,
    window: &mut Window,
    cx: &mut Context<Browser<H>>,
) {
    let Some(bar) = this.bar.as_ref() else { return };
    let (mode, text) = (bar.mode, bar.field.text.clone());
    if mode == Mode::RenameSpace {
        this.rename_active_space(&text);
        return this.close_overlays(window, cx);
    }
    let rows = bar.rows(&this.session, &this.settings);
    let Some(row) = rows.into_iter().nth(row.unwrap_or(bar.selected)) else {
        return;
    };
    this.close_overlays(window, cx);
    match row.item {
        Item::Open(url) => this.navigate(&url, mode == Mode::Address, window, cx),
        Item::Tab(id) => this.activate_tab(id, window, cx),
        Item::Space(id) => this.activate_space(id, window, cx),
        Item::Command(c) => match c {
            Command::NewSpace => this.new_space(window, cx),
            Command::NewIncognitoSpace => this.new_incognito_space(window, cx),
            Command::RenameSpace => this.open_bar(Mode::RenameSpace, window, cx),
            Command::ChangeSpaceColor => this.cycle_space_color(window, cx),
            Command::DeleteSpace => this.delete_active_space(window, cx),
            Command::SwapSidebarSide => this.swap_sidebar_side(cx),
            Command::ReopenClosedTab => this.reopen_closed(window, cx),
            Command::Extensions => this.open_extensions("chrome://extensions/", window, cx),
            Command::ExtensionStore => {
                this.open_extensions("https://chromewebstore.google.com/", window, cx)
            }
        },
    }
}

impl<H: WebHost> Browser<H> {
    pub(crate) fn render_command_bar(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let bar = self.bar.as_ref()?;
        let t = bar.scale.value();
        let rows = bar.rows(&self.session, &self.settings);
        let placeholder = match bar.mode {
            Mode::NewTab => "Search or enter address…",
            Mode::Address => "Enter address…",
            Mode::RenameSpace => "Space name (start with an emoji to change the icon)",
        };
        let accent = theme.accent;
        let white = gpui::white();
        let list = rows.into_iter().enumerate().map(|(i, row)| {
            let selected = i == bar.selected;
            let (glyph, hint) = match &row.item {
                Item::Open(url) if row.label.starts_with("Search") => ("search", "Search"),
                Item::Open(url) if url.is_empty() => ("search", "Open"),
                Item::Open(_) => ("globe", "Open"),
                Item::Tab(_) => ("globe", "Switch to Tab"),
                Item::Space(_) => ("house", "Switch"),
                Item::Command(Command::NewSpace | Command::NewIncognitoSpace) => ("plus", "Run"),
                Item::Command(_) => ("sliders-horizontal", "Run"),
            };
            let (ink, muted) = if selected {
                (white, gpui::Hsla { a: 0.82, ..white })
            } else {
                (surface::ink().into(), surface::muted().into())
            };
            div()
                .id(("bar-row", i))
                .h(px(42.))
                .px(px(12.))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(12.))
                .rounded(px(9.))
                .when(selected, |d| d.bg(accent))
                .when(!selected, |d| d.hover(|s| s.bg(surface::key())))
                .on_click(cx.listener(move |this, _, window, cx| submit(this, Some(i), window, cx)))
                .child(icon(glyph, 18., if selected { muted } else { surface::icon().into() }))
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
                .when(selected, |d| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .text_size(px(12.5))
                            .text_color(muted)
                            .child(hint)
                            .child(icon("arrow-right", 13., muted)),
                    )
                })
        });

        Some(
            div()
                .id("command-bar-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .pt(relative(0.17))
                .occlude()
                .on_click(cx.listener(|this, _, window, cx| this.close_overlays(window, cx)))
                .child(
                    div()
                        .id("command-bar")
                        .track_focus(&self.input_focus)
                        .key_context("CommandBar")
                        .on_key_down(cx.listener(Self::on_input_key))
                        // Clicks inside must not reach the backdrop's close handler.
                        .on_click(|_, _, cx| cx.stop_propagation())
                        // Spring "scale-in": gpui has no transforms, so width and opacity carry it.
                        .w(px(WIDTH * (0.94 + 0.06 * t)))
                        .h_auto()
                        .max_h(px(440.))
                        .opacity(t.clamp(0.0, 1.0))
                        .flex()
                        .flex_col()
                        .rounded(px(14.))
                        .bg(surface::bg())
                        .border_1()
                        .border_color(surface::border())
                        .shadow(vec![
                            shadow(gpui::rgba(0x18122C52).into(), 30., 80.),
                            shadow(gpui::rgba(0x18122C1F).into(), 8., 24.),
                        ])
                        .text_color(surface::ink())
                        .child(
                            div()
                                .h(px(58.))
                                .px(px(20.))
                                .flex()
                                .items_center()
                                .text_size(px(19.))
                                .when(bar.mode != Mode::RenameSpace, |d| {
                                    d.border_b_1().border_color(surface::line())
                                })
                                .child(bar.field.render(
                                    placeholder,
                                    surface::ink().into(),
                                    surface::muted().into(),
                                    gpui::Hsla { a: 0.25, ..accent },
                                )),
                        )
                        .when(bar.mode != Mode::RenameSpace, |d| {
                            d.child(div().p(px(6.)).flex().flex_col().children(list))
                        }),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> (Session, TabId, TabId) {
        let mut s = Session::first_run();
        let sp = s.active_space().unwrap();
        let gh = s.open_tab(sp, "https://github.com", 0);
        s.tab_mut(gh).unwrap().title = "GitHub".into();
        let ls = s.open_tab(sp, "https://lightshow.example", 0);
        s.tab_mut(ls).unwrap().title = "lightshow".into();
        (s, gh, ls)
    }

    #[test]
    fn empty_query_lists_tabs_only() {
        let (s, ..) = session();
        let r = rows("", &s, &Settings::default());
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|r| matches!(r.item, Item::Tab(_))));
    }

    #[test]
    fn typed_text_comes_first_then_best_tab() {
        let (s, gh, _) = session();
        let r = rows("gh", &s, &Settings::default());
        assert_eq!(r[0].item, Item::Open("https://duckduckgo.com/?q=gh".into()));
        assert_eq!(r[0].label, "Search “gh”");
        assert_eq!(r[1].item, Item::Tab(gh));
    }

    #[test]
    fn urls_and_commands() {
        let (s, ..) = session();
        assert_eq!(
            rows("example.com", &s, &Settings::default())[0].label,
            "https://example.com/"
        );
        let r = rows("new space", &s, &Settings::default());
        assert!(r.iter().any(|r| r.item == Item::Command(Command::NewSpace)));
        let r = rows("personal", &s, &Settings::default());
        assert!(r.iter().any(|r| matches!(r.item, Item::Space(_))));
    }
}
