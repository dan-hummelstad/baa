//! General, Appearance, Search, Passwords, Spaces, Keyboard and System.

use baa_model::{
    ChromeStyle, DevToolsDock, ExternalLinks, PageScheme, SearchEngine, Settings, SidebarSide, SpaceId, SEARCH_ENGINES,
};
use baa_web::WebHost;
use gpui::{div, prelude::*, px, AnyElement, Context, PathPromptOptions, SharedString};

use super::{Field, Item, SettingsView};
use crate::{
    actions,
    icons::icon,
    theme::{r, surface},
};

const PASSWORDS_APP: &str = "/System/Applications/Passwords.app";

/// Archive choices, in hours (0 = never).
const ARCHIVE_AFTER: [(u32, &str); 4] = [(12, "12 hours"), (24, "A day"), (24 * 7, "A week"), (0, "Never")];

/// Where `keymap.json` lives, set by the app shell.
pub struct KeymapPath(pub std::path::PathBuf);

impl gpui::Global for KeymapPath {}

fn labels<T: Copy>(options: &[(T, &str)]) -> Vec<(T, SharedString)> {
    options.iter().map(|(v, l)| (*v, SharedString::from(l.to_string()))).collect()
}

/// The built-in engines, then the ones the user added.
fn all_engines(s: &Settings) -> Vec<SearchEngine> {
    SEARCH_ENGINES
        .iter()
        .map(|(name, url)| SearchEngine { name: (*name).into(), url: (*url).into() })
        .chain(s.search_engines.iter().cloned())
        .collect()
}

impl<H: WebHost> SettingsView<H> {
    pub(super) fn general(&self, s: &Settings, cx: &mut Context<Self>) -> Vec<Item> {
        let default = self.default_browser;
        let default_control: AnyElement = if default {
            div().text_size(px(12.5)).text_color(surface::muted()).child("baa is your default browser").into_any_element()
        } else {
            self.button("make-default", "Make Default", true, cx)
                // macOS asks; the answer is read when this window is next in front.
                .on_click(cx.listener(|this, _, _, cx| this.browser.read(cx).host.make_default_browser()))
                .into_any_element()
        };
        let dir = s.web.download_dir.as_ref().map(|p| p.display().to_string());
        let dir_control = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().max_w(px(220.)).truncate().text_size(px(12.5)).text_color(surface::muted()).child(dir.clone().unwrap_or_else(|| "Downloads".into())))
            .child(self.button("download-change", "Change…", false, cx).on_click(cx.listener(|this, _, _, cx| this.choose_download_dir(cx))))
            .children(dir.map(|_| {
                self.button("download-default", "Default", false, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.change(cx, |s| s.web.download_dir = None)))
            }))
            .into_any_element();
        let arc = baa_model::import::arc_sidebar_path().is_some();
        let mut imports = div().flex().flex_wrap().justify_end().gap(px(6.));
        if arc {
            imports = imports.child(
                self.button("import-arc", "Spaces from Arc", false, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.in_browser(cx, false, |b, w, cx| b.import_arc(w, cx)))),
            );
        }
        for (i, source) in baa_model::import::sources().into_iter().enumerate() {
            let label = format!("History from {}", source.name);
            let id: &'static str = ["import-history-0", "import-history-1", "import-history-2", "import-history-3"][i.min(3)];
            imports = imports.child(self.button(id, label, false, cx).on_click(cx.listener(move |this, _, _, cx| {
                let source = source.clone();
                this.browser.update(cx, |b, cx| b.import_history(source, cx));
            })));
        }
        vec![
            self.section("Browser"),
            self.row("default_browser", "", default_control),
            self.row(
                "on_launch",
                "Pinned tabs and favourites are always restored. Pinned only also keeps each Space's open tab.",
                self.segments("pref-launch", labels(&[(true, "Restore Today tabs"), (false, "Pinned only")]), s.restore_today, |v, s| s.restore_today = v, cx),
            ),
            self.row(
                "archive",
                "Pinned tabs and favourites are never archived.",
                self.segments("pref-archive", labels(&ARCHIVE_AFTER), s.archive_after_hours, |v, s| s.archive_after_hours = v, cx),
            ),
            self.section("Links"),
            self.row(
                "external_links",
                "",
                self.segments("pref-external", labels(&[(ExternalLinks::LilBaa, "Little baa"), (ExternalLinks::Tab, "New tab")]), s.external_links, |v, s| s.external_links = v, cx),
            ),
            self.row(
                "peek_links",
                "",
                self.segments("pref-peek", labels(&[(true, "Peek"), (false, "New tab")]), s.peek_links, |v, s| s.peek_links = v, cx),
            ),
            self.section("Downloads"),
            self.row("download_dir", "", dir_control),
            self.row("ask_save", "", self.toggle("pref-ask-save", s.web.ask_where_to_save, |s, v| s.web.ask_where_to_save = v, cx)),
            self.section("Languages"),
            self.row("languages", "Comma-separated, like en-US, sv.", self.field(Field::Languages, "languages-field", "System default", 220., cx)),
            self.row("spellcheck", "", self.toggle("pref-spell", s.web.spellcheck, |s, v| s.web.spellcheck = v, cx)),
            self.row(
                "more_languages",
                "",
                self.button("more-languages", "Open", false, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.in_browser(cx, true, |b, w, cx| b.open_url("baa://settings/languages", w, cx))))
                    .into_any_element(),
            ),
            self.section("Import"),
            self.row("import", "Each Space from Arc comes in as a new one.", imports.into_any_element()),
        ]
    }

    /// A folder picker; the folder becomes the downloads folder.
    fn choose_download_dir(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Choose".into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                if let Some(path) = paths.into_iter().next() {
                    this.update(cx, |this, cx| this.change(cx, |s| s.web.download_dir = Some(path))).ok();
                }
            }
        })
        .detach();
    }

    pub(super) fn appearance(&self, s: &Settings, cx: &mut Context<Self>) -> Vec<Item> {
        vec![
            self.section("Window"),
            self.row(
                "chrome",
                "ASCII draws the window in a monospace terminal frame.",
                self.segments("pref-chrome", labels(&[(ChromeStyle::Arc, "Arc"), (ChromeStyle::Ascii, "ASCII")]), s.chrome, |v, s| s.chrome = v, cx),
            ),
            self.row(
                "side",
                "",
                self.segments("pref-side", labels(&[(SidebarSide::Left, "Left"), (SidebarSide::Right, "Right")]), s.sidebar_side, |v, s| s.sidebar_side = v, cx),
            ),
            self.section("Websites"),
            self.row(
                "page_scheme",
                "What sites see as your light or dark preference.",
                self.segments("pref-scheme", labels(&[(PageScheme::System, "System"), (PageScheme::Light, "Light"), (PageScheme::Dark, "Dark")]), s.page_scheme, |v, s| s.page_scheme = v, cx),
            ),
            self.row(
                "zoom",
                "",
                self.segments(
                    "pref-zoom",
                    labels(&[(0.75, "75%"), (0.9, "90%"), (1.0, "100%"), (1.1, "110%"), (1.25, "125%"), (1.5, "150%"), (1.75, "175%")]),
                    s.web.default_zoom,
                    |v, s| s.web.default_zoom = v,
                    cx,
                ),
            ),
            self.row(
                "font_size",
                "",
                self.segments("pref-font", labels(&[(12, "Small"), (16, "Medium"), (20, "Large"), (24, "Very large")]), s.web.font_size, |v, s| s.web.font_size = v, cx),
            ),
            self.row(
                "min_font_size",
                "",
                self.segments("pref-min-font", labels(&[(0, "Off"), (9, "9"), (12, "12"), (16, "16")]), s.web.min_font_size, |v, s| s.web.min_font_size = v, cx),
            ),
            self.section("Accessibility"),
            self.row("reduce_motion", "Also follows the system setting.", self.toggle("pref-motion", s.reduce_motion, |s, v| s.reduce_motion = v, cx)),
            self.row("focus_highlight", "", self.toggle("pref-focus", s.web.focus_highlight, |s, v| s.web.focus_highlight = v, cx)),
            self.row("caret_browsing", "Move through pages with a text cursor.", self.toggle("pref-caret", s.web.caret_browsing, |s, v| s.web.caret_browsing = v, cx)),
        ]
    }

    pub(super) fn search_tab(&self, s: &Settings, cx: &mut Context<Self>) -> Vec<Item> {
        let engines = all_engines(s);
        let on = engines.iter().position(|e| e.url == s.search_engine.url).unwrap_or(usize::MAX);
        let options: Vec<(usize, SharedString)> = engines.iter().enumerate().map(|(i, e)| (i, SharedString::from(e.name.clone()))).collect();
        let bad_url = {
            let url = self.text(Field::EngineUrl);
            !url.is_empty() && !url.contains("%s")
        };
        let mut out = vec![
            self.section("Default"),
            self.row(
                "engine",
                "Used by the command bar and by Chrome's own search.",
                self.segments(
                    "pref-engine",
                    options,
                    on,
                    |i, s| {
                        if let Some(e) = all_engines(s).get(i) {
                            s.search_engine = e.clone();
                        }
                    },
                    cx,
                ),
            ),
        ];
        for (i, e) in s.search_engines.iter().enumerate() {
            let remove = div()
                .id(("engine-remove", i))
                .debug_selector(|| format!("engine-remove:{i}"))
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(r(6.))
                .hover(|d| d.bg(surface::key()))
                .on_click(cx.listener(move |this, _, _, cx| this.remove_engine(i, cx)))
                .child(icon("x", 13., surface::icon().into()))
                .into_any_element();
            out.push(self.sub_row(format!("engine:{i}"), SharedString::from(format!("{}  {}", e.name, e.url)).into_any_element(), remove));
        }
        out.push(self.row(
            "add_engine",
            if bad_url { "The address needs %s where the search goes." } else { "Use %s where the search goes." },
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(self.field(Field::EngineName, "engine-name", "Name", 90., cx))
                .child(self.field(Field::EngineUrl, "engine-url", "https://…/search?q=%s", 190., cx))
                .child(self.button("engine-add", "Add", true, cx).on_click(cx.listener(|this, _, _, cx| this.add_engine(cx))))
                .into_any_element(),
        ));
        out.push(self.section("Typing"));
        out.push(self.row(
            "suggest",
            "Sends what you type in the command bar to the search engine.",
            self.toggle("pref-suggest", s.search_suggest, |s, v| s.search_suggest = v, cx),
        ));
        out
    }

    pub(super) fn add_engine(&mut self, cx: &mut Context<Self>) {
        let (name, url) = (self.text(Field::EngineName).trim().to_string(), self.text(Field::EngineUrl).trim().to_string());
        if name.is_empty() || !url.contains("%s") {
            return;
        }
        for f in [Field::EngineName, Field::EngineUrl] {
            self.field_mut(f).text.clear();
        }
        self.change(cx, |s| {
            s.search_engines.retain(|e| e.name != name);
            s.search_engines.push(SearchEngine { name, url });
        });
    }

    fn remove_engine(&mut self, i: usize, cx: &mut Context<Self>) {
        self.change(cx, |s| {
            if i < s.search_engines.len() {
                let gone = s.search_engines.remove(i);
                if gone.url == s.search_engine.url {
                    s.search_engine = SearchEngine::default();
                }
            }
        });
    }

    pub(super) fn passwords(&self, cx: &mut Context<Self>) -> Vec<Item> {
        vec![
            self.row(
                "pw_managers",
                "1Password, Bitwarden and others fill logins and passkeys through their Chrome extension; baa connects it to their Mac app.",
                self.button("pref-extensions", "Get Extension", false, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.in_browser(cx, true, |b, w, cx| b.set_up_passwords(w, cx))))
                    .into_any_element(),
            ),
            self.row(
                "pw_apple",
                "macOS lets only Chrome and Edge use Apple's iCloud Passwords extension. Logins are a click away in the Passwords app.",
                self.button("pref-passwords", "Open Passwords", false, cx)
                    .on_click(cx.listener(|_, _, _, cx| cx.open_with_system(std::path::Path::new(PASSWORDS_APP))))
                    .into_any_element(),
            ),
            self.note("Chrome's built-in password manager is off in baa."),
        ]
    }

    pub(super) fn spaces_tab(&self, cx: &mut Context<Self>) -> Vec<Item> {
        let spaces: Vec<(SpaceId, String, String, u32, bool)> = self
            .browser
            .read(cx)
            .session
            .spaces()
            .iter()
            .map(|s| (s.id, s.icon.clone(), s.name.clone(), s.accent, s.ephemeral))
            .collect();
        let many = spaces.len() > 1;
        let mut out = vec![self.row("spaces", "Each Space has its own cookies, logins and extensions.", div().into_any_element())];
        for (id, icon_glyph, name, accent, incognito) in spaces {
            let renaming = self.renaming && self.active == Field::SpaceName(id);
            let left: AnyElement = if renaming {
                self.field(Field::SpaceName(id), "space-name", "Name", 180., cx)
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().size(px(12.)).rounded(r(6.)).bg(gpui::rgb(accent)))
                    .child(SharedString::from(format!("{icon_glyph} {name}")))
                    .children(incognito.then(|| div().text_size(px(11.)).text_color(surface::muted()).child("incognito")))
                    .into_any_element()
            };
            let confirming = self.confirm_delete == Some(id);
            let mut buttons = div().flex().gap(px(6.));
            if !incognito {
                buttons = buttons
                    .child(
                        self.button("space-rename", if renaming { "Done" } else { "Rename" }, false, cx)
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle_rename(id, cx))),
                    )
                    .child(self.button("space-color", "Colour", false, cx).on_click(cx.listener(move |this, _, _, cx| {
                        this.in_browser(cx, false, move |b, w, cx| b.cycle_color_of(id, w, cx))
                    })));
            }
            if many {
                buttons = buttons.child(
                    self.button("space-delete", if confirming { "Delete for Good" } else { "Delete" }, confirming, cx).on_click(cx.listener(move |this, _, _, cx| {
                        if this.confirm_delete == Some(id) {
                            this.confirm_delete = None;
                            this.in_browser(cx, false, move |b, w, cx| b.delete_space(id, w, cx));
                        } else {
                            this.confirm_delete = Some(id);
                            cx.notify();
                        }
                    })),
                );
            }
            out.push(self.sub_row(format!("space:{}", id.0), left, buttons.into_any_element()));
        }
        out.push((
            None,
            div()
                .pt(px(12.))
                .px(px(10.))
                .flex()
                .gap(px(8.))
                .child(self.button("space-new", "New Space", false, cx).on_click(cx.listener(|this, _, _, cx| this.in_browser(cx, true, |b, w, cx| b.new_space(w, cx)))))
                .child(self.button("space-new-incognito", "New Incognito Space", false, cx).on_click(cx.listener(|this, _, _, cx| {
                    this.in_browser(cx, true, |b, w, cx| b.new_incognito_space(w, cx))
                })))
                .into_any_element(),
        ));
        out
    }

    fn toggle_rename(&mut self, id: SpaceId, cx: &mut Context<Self>) {
        if self.renaming && self.active == Field::SpaceName(id) {
            return self.finish_rename(id, cx);
        }
        let current = self.browser.read(cx).session.space(id).map(|s| format!("{} {}", s.icon, s.name)).unwrap_or_default();
        *self.field_mut(Field::SpaceName(id)) = crate::input::TextField::selected(current);
        self.renaming = true;
        self.focus_field(Field::SpaceName(id), cx);
    }

    pub(super) fn finish_rename(&mut self, id: SpaceId, cx: &mut Context<Self>) {
        let text = self.text(Field::SpaceName(id)).to_string();
        self.renaming = false;
        self.focus_field(Field::Search, cx);
        self.browser.update(cx, |b, cx| {
            b.rename_space(id, &text);
            cx.notify();
        });
    }

    pub(super) fn keyboard(&self, cx: &mut Context<Self>) -> Vec<Item> {
        let mut out = vec![self.row("shortcuts", "What each key does in the browser window.", div().into_any_element())];
        let map = cx.key_bindings();
        let map = map.borrow();
        for (label, action) in shortcuts() {
            let keys: Vec<SharedString> = map.bindings_for_action(&*action).next_back().map(crate::command_bar::chips).unwrap_or_default();
            let chips = keys.into_iter().map(|k| {
                div().h(px(20.)).min_w(px(20.)).px(px(6.)).flex().items_center().justify_center().rounded(r(5.)).bg(surface::key()).text_size(px(11.5)).text_color(surface::icon()).child(k)
            });
            out.push(self.sub_row(format!("shortcut:{label}"), SharedString::from(label).into_any_element(), div().flex().gap(px(4.)).children(chips).into_any_element()));
        }
        drop(map);
        out.push(self.section("Customise"));
        out.push(self.row(
            "keymap",
            "Rebind in keymap.json. Changes apply the next time baa starts.",
            self.button("edit-keymap", "Edit keymap.json…", false, cx).on_click(cx.listener(|this, _, _, cx| this.edit_keymap(cx))).into_any_element(),
        ));
        out
    }

    /// Opens the user's keymap in the default editor, creating an empty one first.
    pub(crate) fn edit_keymap(&mut self, cx: &mut Context<Self>) {
        let Some(path) = cx.try_global::<KeymapPath>().map(|p| p.0.clone()) else { return };
        match ensure_keymap(&path) {
            Ok(()) => cx.open_with_system(&path),
            Err(e) => log::error!("creating {}: {e}", path.display()),
        }
    }

    pub(super) fn system(&self, s: &Settings, cx: &mut Context<Self>) -> Vec<Item> {
        const PAGES: [(&str, &str, &str); 7] = [
            ("page-flags", "Flags", "baa://flags"),
            ("page-about", "About", "baa://version"),
            ("page-certs", "Certificates", "baa://settings/certificates"),
            ("page-gpu", "GPU", "baa://gpu"),
            ("page-net", "Net internals", "baa://net-internals"),
            ("page-inspect", "Inspect devices", "baa://inspect"),
            ("page-all", "All Chrome pages", "baa://chrome-urls"),
        ];
        let pages = div().flex().flex_wrap().justify_end().gap(px(6.)).children(PAGES.into_iter().map(|(id, label, url)| {
            self.button(id, label, false, cx).on_click(cx.listener(move |this, _, _, cx| this.in_browser(cx, true, move |b, w, cx| b.open_url(url, w, cx))))
        }));
        let reset: AnyElement = match self.reset {
            None => self
                .button("reset", "Reset…", false, cx)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.reset = Some(false);
                    cx.notify();
                }))
                .into_any_element(),
            Some(sites) => div()
                .flex()
                .flex_col()
                .items_end()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().text_size(px(12.5)).child("Also clear site exceptions in every Space"))
                        .child(self.switch("reset-sites", sites).on_click(cx.listener(move |this, _, _, cx| {
                            this.reset = Some(!sites);
                            cx.notify();
                        }))),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(self.button("reset-cancel", "Cancel", false, cx).on_click(cx.listener(|this, _, _, cx| {
                            this.reset = None;
                            cx.notify();
                        })))
                        .child(self.button("reset-confirm", "Reset Settings", true, cx).on_click(cx.listener(move |this, _, _, cx| this.do_reset(sites, cx)))),
                )
                .into_any_element(),
        };
        vec![
            self.section("Performance"),
            self.row("hw_accel", "Applies when baa restarts.", self.toggle("pref-hw", s.web.hardware_acceleration, |s, v| s.web.hardware_acceleration = v, cx)),
            self.row("memory_saver", "Frees memory from tabs you have not used for a while.", self.toggle("pref-memory", s.web.memory_saver, |s, v| s.web.memory_saver = v, cx)),
            self.section("Developer"),
            self.row(
                "devtools",
                "Where Inspect and ⌥⌘I open DevTools. DevTools' own Dock side menu changes it too.",
                self.segments(
                    "pref-devtools",
                    labels(&[(DevToolsDock::Right, "Right"), (DevToolsDock::Bottom, "Bottom"), (DevToolsDock::Left, "Left"), (DevToolsDock::Window, "Window")]),
                    s.devtools_dock,
                    |v, s| s.devtools_dock = v,
                    cx,
                ),
            ),
            self.row("chrome_pages", "Chrome's own pages open in a tab.", pages.into_any_element()),
            self.section("Reset"),
            self.row("reset", "Puts every setting back to its default. Search engines you added and your toolbar choices are kept.", reset),
        ]
    }

    fn do_reset(&mut self, sites: bool, cx: &mut Context<Self>) {
        self.reset = None;
        self.browser.update(cx, |b, cx| b.reset_settings(sites, cx));
        // Switches that were flipped are not where the defaults put them.
        self.motion = crate::motion::Motion::default();
        let i = self.tab.index() as f32;
        self.motion.from(("settings-pill", 0), crate::motion::Feel::Glide, i, i);
        self.motion.from(("settings-tab", 0), crate::motion::Feel::Glide, 1., 1.);
        self.seed_fields(cx);
        cx.notify();
    }
}

/// `[]` where there is no keymap yet, so the editor opens on something valid.
pub(crate) fn ensure_keymap(path: &std::path::Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, "[]\n")
}

/// Every action with a default key, labelled for the Keyboard tab.
fn shortcuts() -> Vec<(&'static str, Box<dyn gpui::Action>)> {
    use actions::*;
    vec![
        ("New tab or search", Box::new(CommandBar)),
        ("Focus address", Box::new(FocusAddress)),
        ("Toggle sidebar", Box::new(ToggleSidebar)),
        ("Close tab", Box::new(CloseTab)),
        ("Reopen closed tab", Box::new(ReopenClosedTab)),
        ("Next tab", Box::new(NextTab)),
        ("Previous tab", Box::new(PrevTab)),
        ("Last tab", Box::new(LastTab)),
        ("Next Space", Box::new(NextSpace)),
        ("Previous Space", Box::new(PrevSpace)),
        ("Pin or unpin tab", Box::new(TogglePin)),
        ("Back", Box::new(Back)),
        ("Forward", Box::new(Forward)),
        ("Reload", Box::new(Reload)),
        ("Find in page", Box::new(Find)),
        ("Developer tools", Box::new(DevTools)),
        ("Zoom in", Box::new(ZoomIn)),
        ("Zoom out", Box::new(ZoomOut)),
        ("Actual size", Box::new(ZoomReset)),
        ("New incognito Space", Box::new(NewIncognitoSpace)),
        ("Open peek in a tab", Box::new(ExpandPeek)),
        ("Copy URL", Box::new(CopyUrl)),
        ("Print", Box::new(Print)),
        ("View source", Box::new(ViewSource)),
        ("Reader mode", Box::new(ReaderMode)),
        ("Picture in picture", Box::new(PictureInPicture)),
        ("Mute tab", Box::new(MuteTab)),
        ("Downloads", Box::new(ShowDownloads)),
        ("History", Box::new(ShowHistory)),
        ("Settings", Box::new(OpenSettings)),
    ]
}
