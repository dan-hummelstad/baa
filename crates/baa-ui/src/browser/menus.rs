//! Native context menus for the sidebar, tabs, downloads and extensions.

use super::*;

impl<H: WebHost> Browser<H> {
    /// A native menu for `kind` at `at`; its item runs once it closes.
    pub(crate) fn open_menu(&mut self, kind: MenuKind, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let actions = self.sidebar_menu(&kind);
        let items = actions.iter().enumerate().map(|(i, (label, _))| baa_web::MenuItem::action(*label, i)).collect();
        let this = cx.entity().downgrade();
        self.menu_open = true;
        self.host.show_menu(at, items, window, cx, Box::new(move |picked, window, cx| {
            let _ = this.update(cx, |this, cx| {
                this.menu_open = false;
                // Closed first: a rename started by the item must stay open.
                this.close_overlays(window, cx);
                if let Some((_, run)) = picked.and_then(|i| actions.into_iter().nth(i)) {
                    run(this, window, cx);
                }
            });
        }));
    }

    pub(super) fn sidebar_menu(&self, kind: &MenuKind) -> Vec<(&'static str, MenuAction<H>)> {
        let mut items: Vec<(&'static str, MenuAction<H>)> = Vec::new();
        let space = self.session.active_space();
        match *kind {
            MenuKind::Tab(id) => {
                let Some(tab) = self.session.tab(id) else {
                    return items;
                };
                let zone = tab.zone;
                if zone == Zone::Favorite {
                    items.push((
                        "Remove from Favorites",
                        Box::new(move |this, window, cx| {
                            if let Some(space) = space {
                                this.session.move_tab(id, space, Zone::Today, 0);
                                this.sync(window, cx);
                            }
                        }),
                    ));
                } else {
                    items.push((
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
                    items.push((
                        if zone == Zone::Pinned { "Unpin" } else { "Pin" },
                        Box::new(move |this, window, cx| {
                            this.session.toggle_pin(id);
                            this.sync(window, cx);
                        }),
                    ));
                    items.push((
                        "Move to New Folder",
                        Box::new(move |this, window, cx| this.new_folder(Some(id), None, window, cx)),
                    ));
                }
                items.push((
                    "Close Tab",
                    Box::new(move |this, window, cx| this.close_tab_id(id, window, cx)),
                ));
            }
            MenuKind::Download { ref path, ref url } => {
                let (open, reveal, link, forget) = (path.clone(), path.clone(), url.clone(), path.clone());
                items.push(("Open", Box::new(move |_, _, cx| cx.open_with_system(std::path::Path::new(&open)))));
                items.push((
                    "Show in Finder",
                    Box::new(move |_, _, cx| cx.reveal_path(std::path::Path::new(&reveal))),
                ));
                items.push((
                    "Copy Download Link",
                    Box::new(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(link.clone()))),
                ));
                items.push(("Remove from List", Box::new(move |this, _, cx| this.forget_download(&forget, cx))));
            }
            MenuKind::LibraryFilter => {
                for (kind, label) in crate::library::KINDS {
                    items.push((
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
                if self.session.folder(id).is_some_and(|f| f.live.is_some()) {
                    items.push(("Refresh", Box::new(move |this, window, cx| this.refresh_live(Some(id), window, cx))));
                } else {
                    items.push((
                        "New Subfolder",
                        Box::new(move |this, window, cx| this.new_folder(None, Some(id), window, cx)),
                    ));
                }
                items.push((
                    "Rename Folder",
                    Box::new(move |this, window, cx| this.start_folder_rename(id, window, cx)),
                ));
                items.push((
                    "Delete Folder",
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
                items.push((
                    if pinned { "Unpin from Toolbar" } else { "Pin to Toolbar" },
                    Box::new(move |this, _, cx| this.toggle_extension_pin(&pin, cx)),
                ));
                if let Some(options) = ext.options {
                    items.push((
                        "Options",
                        Box::new(move |this, window, cx| this.open_url(&options, window, cx)),
                    ));
                }
                items.push((
                    "Manage Extension",
                    Box::new(move |this, window, cx| {
                        this.open_own_page(&format!("baa://extensions/?id={manage}"), window, cx);
                    }),
                ));
                // The settings tab has the Remove button, which asks first.
                items.push((
                    "Remove…",
                    Box::new(move |this, window, cx| {
                        this.open_own_page(&format!("baa://extensions/?id={remove}"), window, cx);
                    }),
                ));
            }
            MenuKind::Sidebar => {
                items.push((
                    "New Folder",
                    Box::new(|this, window, cx| this.new_folder(None, None, window, cx)),
                ));
                // Arc's presets, written as the GitHub pages they mirror.
                for (label, page) in [
                    ("New Live Folder: My Pull Requests", "https://github.com/pulls"),
                    ("New Live Folder: Review Requests", "https://github.com/pulls/review-requested"),
                    ("New Live Folder: My Issues", "https://github.com/issues"),
                ] {
                    items.push((
                        label,
                        Box::new(move |this, window, cx| {
                            if let Some((name, source)) = baa_model::LiveSource::from_page(page) {
                                this.new_live_folder(&name, source, window, cx);
                            }
                        }),
                    ));
                }
                if let Some((name, source)) = self.live_source_here() {
                    items.push((
                        "New Live Folder from This Page",
                        Box::new(move |this, window, cx| this.new_live_folder(&name, source.clone(), window, cx)),
                    ));
                }
                items.push((
                    "New Tab",
                    Box::new(|this, window, cx| this.open_bar(Mode::NewTab, window, cx)),
                ));
            }
        }
        items
    }
}
