//! The Extensions tab. `chrome://extensions` is drawn here and carried out by
//! a hidden Chrome page (baa-web), since baa's pages cannot run Chrome's.

use baa_model::{Settings, SpaceId};
use baa_web::{ExtensionChange, ExtensionInfo, SiteAccess, WebHost};
use gpui::{div, prelude::*, px, AnyElement, Context, PathPromptOptions, SharedString};

use super::{Item, SettingsView};
use crate::{
    browser::Browser,
    icons::icon,
    theme::{r, surface},
};

/// What the tab shows, kept on the browser: the toolbar reads it too.
#[derive(Default)]
pub(crate) struct ExtensionsPage {
    /// `None` until read, or when reading failed.
    pub list: Option<Vec<ExtensionInfo>>,
    pub loading: bool,
    /// The Space whose extensions `list` holds.
    pub space: Option<SpaceId>,
    /// The extension whose details are open.
    pub open: Option<String>,
    /// The extension Remove was pressed on once.
    pub confirm_remove: Option<String>,
    /// Why the last "Load unpacked" failed.
    pub error: Option<String>,
}

impl<H: WebHost> Browser<H> {
    /// Re-reads `space`'s extensions.
    pub(crate) fn refresh_extensions_for(&mut self, space: SpaceId, cx: &mut Context<Self>) {
        let Some(profile) = self.session.space(space).map(|s| s.profile) else { return };
        if self.extensions_page.space != Some(space) {
            self.extensions_page.list = None;
        }
        self.extensions_page.space = Some(space);
        self.extensions_page.loading = true;
        let this = cx.weak_entity();
        self.host.extension_details(
            profile,
            cx,
            // Deferred: the answer may come straight back, inside this update.
            Box::new(move |list, cx| {
                cx.defer(move |cx| {
                    this.update(cx, |this, cx| {
                        if this.extensions_page.space == Some(space) {
                            this.extensions_page.list = list;
                            this.extensions_page.loading = false;
                        }
                        if let Some(active) = this.active_space() {
                            this.extensions = this.host.extensions(active.profile);
                        }
                        cx.notify();
                    })
                    .ok();
                })
            }),
        );
        cx.notify();
    }

    /// Shown at once; then Chrome's answer replaces it.
    pub(crate) fn change_extension(&mut self, space: SpaceId, id: &str, change: ExtensionChange, cx: &mut Context<Self>) {
        let Some(profile) = self.session.space(space).map(|s| s.profile) else { return };
        if self.extensions_page.space == Some(space) {
            if let Some(list) = &mut self.extensions_page.list {
                match change {
                    ExtensionChange::Remove => list.retain(|e| e.id != id),
                    ExtensionChange::Reload => {}
                    ExtensionChange::Enable(on) => list.iter_mut().filter(|e| e.id == id).for_each(|e| e.enabled = on),
                    ExtensionChange::SiteAccess(a) => list.iter_mut().filter(|e| e.id == id).for_each(|e| e.site_access = Some(a)),
                }
            }
        }
        if change == ExtensionChange::Remove && self.session.active_space() == Some(space) {
            self.extensions.retain(|e| e.id != id);
        }
        let this = cx.weak_entity();
        self.host.change_extension(
            profile,
            id,
            change,
            cx,
            Box::new(move |_, cx| {
                cx.defer(move |cx| {
                    this.update(cx, |this, cx| this.refresh_extensions_for(space, cx)).ok();
                })
            }),
        );
        cx.notify();
    }

    /// Developer mode: an extension straight from a folder.
    pub(crate) fn load_unpacked(&mut self, space: SpaceId, path: &std::path::Path, cx: &mut Context<Self>) {
        let Some(profile) = self.session.space(space).map(|s| s.profile) else { return };
        self.extensions_page.error = None;
        let this = cx.weak_entity();
        self.host.load_unpacked(
            profile,
            path,
            cx,
            Box::new(move |result, cx| {
                cx.defer(move |cx| {
                    this.update(cx, |this, cx| {
                        match result {
                            Ok(_) => this.extensions_page.error = None,
                            Err(e) => this.extensions_page.error = Some(e),
                        }
                        this.refresh_extensions_for(space, cx);
                    })
                    .ok();
                })
            }),
        );
        cx.notify();
    }

    /// Whether the toolbar shows `id`: not when Chrome has it switched off.
    pub(crate) fn extension_enabled(&self, id: &str) -> bool {
        self.extensions_page
            .list
            .as_ref()
            .filter(|_| self.extensions_page.space == self.session.active_space())
            .is_none_or(|list| list.iter().find(|e| e.id == id).is_none_or(|e| e.enabled))
    }
}

impl<H: WebHost> SettingsView<H> {
    pub(super) fn extensions_tab(&self, s: &Settings, cx: &mut Context<Self>) -> Vec<Item> {
        let space = self.space;
        let page = &self.browser.read(cx).extensions_page;
        let (list, loading, error) = (page.list.clone(), page.loading, page.error.clone());
        let mut out = vec![self.space_picker(cx)];
        let mut tools = div().flex().items_center().gap(px(8.));
        if s.web.developer_mode {
            tools = tools.child(self.button("ext-load", "Load unpacked…", false, cx).on_click(cx.listener(|this, _, _, cx| this.choose_unpacked(cx))));
        }
        out.push(self.row(
            "dev_mode",
            "Load extensions from a folder and reload them while you work on them.",
            self.toggle("pref-dev-mode", s.web.developer_mode, |s, v| s.web.developer_mode = v, cx),
        ));
        out.push(self.row(
            "load_unpacked",
            "Pick the folder that holds the extension's manifest.json.",
            tools
                .child(self.button("ext-store", "Chrome Web Store", false, cx).on_click(cx.listener(|this, _, _, cx| {
                    this.in_browser(cx, |b, w, cx| b.open_url("https://chromewebstore.google.com/", w, cx))
                })))
                .into_any_element(),
        ));
        if let Some(error) = error {
            out.push(self.note(format!("Couldn't load it: {error}")));
        }
        out.push(self.row("ext_list", "", div().into_any_element()));
        match list {
            Some(list) if list.is_empty() => out.push(self.note("No extensions yet. Find some in the Chrome Web Store.")),
            Some(list) => {
                for (i, e) in list.iter().enumerate() {
                    out.push((None, self.render_extension(i, e, space, s.web.developer_mode, cx)));
                }
            }
            None if loading => out.push(self.note("Loading…")),
            None => out.push(self.note("Couldn't read your extensions.")),
        }
        out
    }

    /// A folder picker; the folder is loaded into the chosen Space.
    fn choose_unpacked(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Load".into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                if let Some(path) = paths.into_iter().next() {
                    this.update(cx, |this, cx| this.load_unpacked_from(&path, cx)).ok();
                }
            }
        })
        .detach();
    }

    pub(crate) fn load_unpacked_from(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        let space = self.space;
        self.browser.update(cx, |b, cx| b.load_unpacked(space, path, cx));
    }

    fn render_extension(&self, i: usize, e: &ExtensionInfo, space: SpaceId, developer: bool, cx: &mut Context<Self>) -> AnyElement {
        let b = self.browser.read(cx);
        let page = &b.extensions_page;
        let open = page.open.as_deref() == Some(e.id.as_str());
        let confirming = page.confirm_remove.as_deref() == Some(e.id.as_str());
        let pinned = !b.settings.unpinned_extensions.contains(&e.id);
        let icon_path = b.extensions.iter().find(|x| x.id == e.id).and_then(|x| x.icon.clone());
        let n = i as u64;
        let (id, on) = (e.id.clone(), e.enabled);
        let switch = self.switch("ext-on", on).on_click(cx.listener(move |this, _, _, cx| {
            this.browser.update(cx, |b, cx| b.change_extension(space, &id, ExtensionChange::Enable(!on), cx))
        }));
        let mut actions: Vec<AnyElement> = Vec::new();
        let id = e.id.clone();
        actions.push(
            self.button("ext-details", if open { "Hide Details" } else { "Details" }, false, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.browser.update(cx, |b, cx| {
                        let page = &mut b.extensions_page;
                        page.open = (page.open.as_deref() != Some(id.as_str())).then(|| id.clone());
                        cx.notify();
                    })
                }))
                .into_any_element(),
        );
        if let Some(options) = e.options.clone().filter(|_| e.enabled) {
            actions.push(
                self.button("ext-options", "Options", false, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let options = options.clone();
                        this.in_browser(cx, move |b, w, cx| b.open_url(&options, w, cx))
                    }))
                    .into_any_element(),
            );
        }
        let id = e.id.clone();
        actions.push(
            self.button("ext-pin", if pinned { "Unpin from Toolbar" } else { "Pin to Toolbar" }, false, cx)
                .on_click(cx.listener(move |this, _, _, cx| this.browser.update(cx, |b, cx| b.toggle_extension_pin(&id, cx))))
                .into_any_element(),
        );
        if developer && e.unpacked {
            let id = e.id.clone();
            actions.push(
                self.button("ext-reload", "Reload", false, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.browser.update(cx, |b, cx| b.change_extension(space, &id, ExtensionChange::Reload, cx))
                    }))
                    .into_any_element(),
            );
        }
        actions.push(div().flex_1().into_any_element());
        let id = e.id.clone();
        if confirming {
            actions.push(
                self.button("ext-keep", "Cancel", false, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.browser.update(cx, |b, cx| {
                            b.extensions_page.confirm_remove = None;
                            cx.notify();
                        })
                    }))
                    .into_any_element(),
            );
        }
        actions.push(
            self.button("ext-remove", if confirming { "Remove for Good" } else { "Remove" }, confirming, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let id = id.clone();
                    this.browser.update(cx, |b, cx| {
                        if b.extensions_page.confirm_remove.as_deref() == Some(id.as_str()) {
                            b.extensions_page.confirm_remove = None;
                            b.change_extension(space, &id, ExtensionChange::Remove, cx);
                        } else {
                            b.extensions_page.confirm_remove = Some(id);
                            cx.notify();
                        }
                    })
                }))
                .into_any_element(),
        );
        let details = open.then(|| self.render_details(n, e, space, cx));
        let ext_id = e.id.clone();
        div()
            .id(("extension", i))
            .debug_selector(|| format!("ext:{ext_id}"))
            .my(px(5.))
            .p(px(16.))
            .flex()
            .flex_none()
            .flex_col()
            .gap(px(12.))
            .rounded(r(12.))
            .border_1()
            .border_color(surface::line())
            .child(
                div()
                    .flex()
                    .gap(px(14.))
                    .child(
                        div()
                            .size(px(36.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .opacity(if e.enabled { 1. } else { 0.45 })
                            .child(match icon_path {
                                Some(path) => gpui::img(path).size(px(30.)).into_any_element(),
                                None => icon("puzzle", 24., surface::icon().into()).into_any_element(),
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap(px(8.))
                                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(SharedString::from(e.name.clone())))
                                    .child(div().text_size(px(12.)).text_color(surface::muted()).child(SharedString::from(e.version.clone()))),
                            )
                            .child(div().text_size(px(12.5)).text_color(surface::muted()).line_clamp(2).child(SharedString::from(e.description.clone()))),
                    )
                    .child(switch),
            )
            .child(div().flex().flex_wrap().gap(px(6.)).children(actions))
            .children(details)
            .into_any_element()
    }

    fn render_details(&self, n: u64, e: &ExtensionInfo, space: SpaceId, cx: &mut Context<Self>) -> AnyElement {
        let heading = |text: &'static str| div().text_size(px(12.)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(surface::muted()).child(text);
        let permissions: Vec<AnyElement> = if e.permissions.is_empty() {
            vec![div().text_size(px(13.)).child("No special permissions").into_any_element()]
        } else {
            e.permissions.iter().map(|p| div().text_size(px(13.)).child(SharedString::from(format!("•  {p}"))).into_any_element()).collect()
        };
        let access = e.site_access.map(|current| {
            let mut options = vec![(SiteAccess::OnClick, "On Click"), (SiteAccess::OnAllSites, "On All Sites")];
            // Chosen sites are set in Chrome's page; shown only when already in use.
            if current == SiteAccess::OnSpecificSites {
                options.insert(1, (SiteAccess::OnSpecificSites, "On Specific Sites"));
            }
            let _ = n;
            let id = e.id.clone();
            let segments = self.segments_with(
                "ext-access",
                options.into_iter().map(|(v, l)| (v, SharedString::from(l))).collect(),
                current,
                move |this, value, cx| {
                    let id = id.clone();
                    this.browser.update(cx, |b, cx| b.change_extension(space, &id, ExtensionChange::SiteAccess(value), cx))
                },
                cx,
            );
            div().flex().flex_col().gap(px(6.)).child(heading("Site Access")).child(segments)
        });
        div()
            .pt(px(12.))
            .border_t_1()
            .border_color(surface::line())
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(div().flex().flex_col().gap(px(6.)).child(heading("Permissions")).children(permissions))
            .children(access)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(heading("ID"))
                    .child(div().text_size(px(12.)).font_family(crate::theme::MONO).child(SharedString::from(e.id.clone()))),
            )
            .into_any_element()
    }
}
