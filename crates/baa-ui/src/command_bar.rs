//! ⌘T / ⌘L command bar (PLAN.md §3): URL or search, fuzzy match over open tabs,
//! "switch to space", and Space commands.

use baa_model::{Session, Settings, SpaceId, TabId};
use baa_web::WebHost;
use gpui::{div, prelude::*, px, relative, Context, SharedString, Window};

use crate::{
    browser::{tab_title, Browser},
    fuzzy,
    input::{Key, TextField},
    motion::Spring,
    theme::Theme,
};

const MAX_ROWS: usize = 8;
const WIDTH: f32 = 600.0;

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
}

const COMMANDS: [(Command, &str); 7] = [
    (Command::NewSpace, "New Space"),
    (Command::NewIncognitoSpace, "New Incognito Space"),
    (Command::RenameSpace, "Rename Space"),
    (Command::ChangeSpaceColor, "Change Space Color"),
    (Command::DeleteSpace, "Delete Space"),
    (Command::SwapSidebarSide, "Move Sidebar to Other Side"),
    (Command::ReopenClosedTab, "Reopen Closed Tab"),
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
        let list = rows.into_iter().enumerate().map(|(i, row)| {
            div()
                .id(("bar-row", i))
                .flex()
                .flex_col()
                .px_3()
                .py_1p5()
                .rounded_md()
                .when(i == bar.selected, |d| d.bg(theme.chrome_hover))
                .hover(|s| s.bg(theme.chrome_hover))
                .on_click(cx.listener(move |this, _, window, cx| submit(this, Some(i), window, cx)))
                .child(div().truncate().child(SharedString::from(row.label)))
                .when(!row.detail.is_empty(), |d| {
                    d.child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(theme.text_dim)
                            .child(SharedString::from(row.detail)),
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
                .pt(relative(0.15))
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
                        .max_h(px(420.))
                        .opacity(t.clamp(0.0, 1.0))
                        .flex()
                        .flex_col()
                        .p_2()
                        .gap_1()
                        .rounded_xl()
                        .bg(theme.chrome_active)
                        .border_1()
                        .border_color(theme.border)
                        .shadow_lg()
                        .text_color(theme.text)
                        .child(div().px_3().py_2().text_lg().child(bar.field.render(
                            placeholder,
                            theme.text,
                            theme.text_dim,
                            theme.chrome_hover,
                        )))
                        .children(list),
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
