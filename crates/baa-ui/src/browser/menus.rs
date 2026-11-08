//! Native context menus for the sidebar, tabs, downloads and extensions.

use super::*;

pub(crate) enum MenuEntry<H: WebHost> {
    Item { label: String, checked: bool, enabled: bool, run: MenuAction<H> },
    Separator,
    Submenu(String, Vec<MenuEntry<H>>),
}

impl<H: WebHost> MenuEntry<H> {
    pub(super) fn item(label: impl Into<String>, run: MenuAction<H>) -> Self {
        MenuEntry::Item { label: label.into(), checked: false, enabled: true, run }
    }

    fn check(label: impl Into<String>, checked: bool, run: MenuAction<H>) -> Self {
        MenuEntry::Item { label: label.into(), checked, enabled: true, run }
    }

    /// Greyed-out text.
    fn note(label: impl Into<String>) -> Self {
        MenuEntry::Item { label: label.into(), checked: false, enabled: false, run: Box::new(|_, _, _| {}) }
    }
}

/// The native menu's rows; each action's id is its index in `actions`.
fn flatten<H: WebHost>(entries: Vec<MenuEntry<H>>, actions: &mut Vec<MenuAction<H>>) -> Vec<baa_web::MenuItem> {
    entries
        .into_iter()
        .map(|e| match e {
            MenuEntry::Item { label, checked, enabled, run } => {
                actions.push(run);
                baa_web::MenuItem::Action { label, enabled, checked, id: actions.len() - 1 }
            }
            MenuEntry::Separator => baa_web::MenuItem::Separator,
            MenuEntry::Submenu(label, items) => baa_web::MenuItem::Submenu { label, items: flatten(items, actions) },
        })
        .collect()
}

/// "just now", "3 min ago", "2 h ago".
fn ago(then: u64, now: u64) -> String {
    match now.saturating_sub(then) / 60 {
        0 => "just now".into(),
        m @ 1..60 => format!("{m} min ago"),
        m => format!("{} h ago", m / 60),
    }
}

impl<H: WebHost> Browser<H> {
    /// A native menu for `kind` at `at`; its item runs once it closes.
    pub(crate) fn open_menu(&mut self, kind: MenuKind, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let mut actions = Vec::new();
        let items = flatten(self.sidebar_menu(&kind), &mut actions);
        let this = cx.entity().downgrade();
        self.menu_open = true;
        self.host.show_menu(at, items, window, cx, Box::new(move |picked, window, cx| {
            let _ = this.update(cx, |this, cx| {
                this.menu_open = false;
                // Closed first: a rename started by the item must stay open.
                this.close_overlays(window, cx);
                if let Some(run) = picked.and_then(|i| actions.into_iter().nth(i)) {
                    run(this, window, cx);
                }
            });
        }));
    }

    pub(super) fn sidebar_menu(&self, kind: &MenuKind) -> Vec<MenuEntry<H>> {
        let mut items: Vec<MenuEntry<H>> = Vec::new();
        let space = self.session.active_space();
        match *kind {
            MenuKind::Tab(id) => {
                let Some(tab) = self.session.tab(id) else {
                    return items;
                };
                let zone = tab.zone;
                if zone == Zone::Favorite {
                    items.push(MenuEntry::item(
                        "Remove from Favorites",
                        Box::new(move |this, window, cx| {
                            if let Some(space) = space {
                                this.session.move_tab(id, space, Zone::Today, 0);
                                this.sync(window, cx);
                            }
                        }),
                    ));
                } else {
                    items.push(MenuEntry::item(
                        "Add to Favorites",
                        // The Space argument is ignored: a favourite keeps its own.
                        Box::new(move |this, window, cx| {
                            let space = this.session.tab(id).map(|t| t.space);
                            if let Some(space) = space {
                                this.session.move_tab(id, space, Zone::Favorite, usize::MAX);
                                this.sync(window, cx);
                            }
                        }),
                    ));
                    items.push(MenuEntry::item(
                        if zone == Zone::Pinned { "Unpin" } else { "Pin" },
                        Box::new(move |this, window, cx| {
                            this.session.toggle_pin(id);
                            this.sync(window, cx);
                        }),
                    ));
                    items.push(MenuEntry::item(
                        "Move to New Folder",
                        Box::new(move |this, window, cx| this.new_folder(Some(id), None, window, cx)),
                    ));
                }
                items.push(MenuEntry::item(
                    "Close Tab",
                    Box::new(move |this, window, cx| this.close_tab_id(id, window, cx)),
                ));
            }
            MenuKind::Download { ref path, ref url } => {
                let (open, reveal, link, forget) = (path.clone(), path.clone(), url.clone(), path.clone());
                items.push(MenuEntry::item("Open", Box::new(move |_, _, cx| cx.open_with_system(std::path::Path::new(&open)))));
                items.push(MenuEntry::item(
                    "Show in Finder",
                    Box::new(move |_, _, cx| cx.reveal_path(std::path::Path::new(&reveal))),
                ));
                items.push(MenuEntry::item(
                    "Copy Download Link",
                    Box::new(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(link.clone()))),
                ));
                items.push(MenuEntry::item("Remove from List", Box::new(move |this, _, cx| this.forget_download(&forget, cx))));
            }
            MenuKind::LibraryFilter => {
                for (kind, label) in crate::library::KINDS {
                    items.push(MenuEntry::item(
                        label,
                        Box::new(move |this, _, cx| {
                            if let Some(library) = &mut this.library {
                                library.filter = kind;
                            }
                            cx.notify();
                        }),
                    ));
                }
            }
            MenuKind::Folder(id) => {
                let Some(folder) = self.session.folder(id) else { return items };
                let live = folder.live.is_some();
                match &folder.live {
                    Some(source) => {
                        items.push(MenuEntry::Submenu(folder.name.clone(), self.live_menu(id, source)));
                        items.push(MenuEntry::Separator);
                    }
                    None => items.push(MenuEntry::item(
                        "New Subfolder",
                        Box::new(move |this, window, cx| this.new_folder(None, Some(id), window, cx)),
                    )),
                }
                items.push(MenuEntry::item(
                    "Rename Folder",
                    Box::new(move |this, window, cx| this.start_folder_rename(id, window, cx)),
                ));
                items.push(MenuEntry::item(
                    if live { "Delete Live Folder" } else { "Delete Folder" },
                    Box::new(move |this, window, cx| {
                        this.session.remove_folder(id);
                        this.sync(window, cx);
                    }),
                ));
            }
            MenuKind::Extension(ref id) => {
                let Some(ext) = self.extensions.iter().find(|e| &e.id == id).cloned() else {
                    return items;
                };
                let pinned = !self.settings.unpinned_extensions.contains(id);
                let (pin, manage, remove) = (id.clone(), id.clone(), id.clone());
                items.push(MenuEntry::item(
                    if pinned { "Unpin from Toolbar" } else { "Pin to Toolbar" },
                    Box::new(move |this, _, cx| this.toggle_extension_pin(&pin, cx)),
                ));
                if let Some(options) = ext.options {
                    items.push(MenuEntry::item(
                        "Options",
                        Box::new(move |this, window, cx| this.open_url(&options, window, cx)),
                    ));
                }
                items.push(MenuEntry::item(
                    "Manage Extension",
                    Box::new(move |this, window, cx| {
                        this.open_own_page(&format!("baa://extensions/?id={manage}"), window, cx);
                    }),
                ));
                // The settings tab has the Remove button, which asks first.
                items.push(MenuEntry::item(
                    "Remove…",
                    Box::new(move |this, window, cx| {
                        this.open_own_page(&format!("baa://extensions/?id={remove}"), window, cx);
                    }),
                ));
            }
            MenuKind::Space(id) => {
                let Some(space) = self.session.space(id) else { return items };
                if !space.ephemeral {
                    items.push(MenuEntry::item("Rename Space", Box::new(|this, window, cx| this.start_rename(window, cx))));
                    let colors = PALETTE
                        .iter()
                        .zip(PALETTE_NAMES)
                        .enumerate()
                        .map(|(i, ((top, _), name))| {
                            MenuEntry::check(name, *top == space.accent, Box::new(move |this, window, cx| this.set_color_of(id, i, window, cx)))
                        })
                        .collect();
                    items.push(MenuEntry::Submenu("Theme Colour".into(), colors));
                    // ponytail: a fixed emoji set; any other emoji goes in through Rename ("🦊 Work").
                    let icons = ICONS
                        .iter()
                        .chain(&["🏠", "💼", "🎨", "📚", "🎮", "🎵", "✈️", "🧪"])
                        .map(|&icon| {
                            MenuEntry::check(icon, icon == space.icon, Box::new(move |this, _, cx| {
                                this.set_icon_of(id, icon);
                                cx.notify();
                            }))
                        })
                        .collect();
                    items.push(MenuEntry::Submenu("Icon".into(), icons));
                    items.push(MenuEntry::Separator);
                }
                items.push(MenuEntry::item(
                    "Space Settings…",
                    Box::new(|this, _, cx| this.open_settings_at(Some(crate::settings::Tab::Spaces), Some("spaces"), cx)),
                ));
                if self.session.spaces().len() > 1 {
                    items.push(MenuEntry::item("Delete Space…", Box::new(move |this, _, cx| this.ask_delete_space(id, cx))));
                }
            }
            MenuKind::Sidebar => {
                items.push(MenuEntry::item(
                    "New Folder",
                    Box::new(|this, window, cx| this.new_folder(None, None, window, cx)),
                ));
                // GitHub pages pick their own kind of folder (see LiveSource::from_page).
                if let Some((name, source)) = self.live_source_here() {
                    items.push(MenuEntry::item(
                        "New Live Folder from This Page",
                        Box::new(move |this, window, cx| this.new_live_folder(&name, source.clone(), window, cx)),
                    ));
                }
                items.push(MenuEntry::item(
                    "New Tab",
                    Box::new(|this, window, cx| this.open_bar(Mode::NewTab, window, cx)),
                ));
            }
        }
        items
    }

    /// Arc's live folder submenu: last fetch, Refresh, and for GitHub the filters and repos.
    fn live_menu(&self, id: FolderId, source: &baa_model::LiveSource) -> Vec<MenuEntry<H>> {
        let mut items = vec![
            MenuEntry::note(match (self.live_errors.get(&id), self.live_fetched.get(&id)) {
                (Some(e), _) => format!("Last fetch failed: {e}"),
                (None, Some(at)) => format!("Last fetch: {}", ago(*at, now())),
                (None, None) => "Fetching…".into(),
            }),
            MenuEntry::item("Refresh", Box::new(move |this, window, cx| this.refresh_live(Some(id), window, cx))),
        ];
        let baa_model::LiveSource::GitHub(gh) = source else { return items };
        items.push(MenuEntry::Separator);
        for &filter in baa_model::live::GitHubFilter::for_kind(gh.kind) {
            items.push(MenuEntry::check(
                filter.label(),
                gh.filters.contains(&filter),
                Box::new(move |this, window, cx| this.toggle_live_filter(id, filter, window, cx)),
            ));
        }
        items.push(MenuEntry::Separator);
        let repos: Vec<MenuEntry<H>> = match self.live_repos(id) {
            repos if repos.is_empty() => vec![MenuEntry::note("None yet")],
            repos => repos
                .into_iter()
                .map(|repo| {
                    let shown = !gh.hidden_repos.contains(&repo);
                    let label = repo.clone();
                    MenuEntry::check(
                        label,
                        shown,
                        Box::new(move |this, window, cx| this.set_live_repo_hidden(id, &repo, shown, window, cx)),
                    )
                })
                .collect(),
        };
        items.push(MenuEntry::Submenu("Repositories".into(), repos));
        items
    }
}
