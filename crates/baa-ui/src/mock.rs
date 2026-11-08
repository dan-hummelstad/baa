//! Test double for `WebHost` so UI tests run without a browser engine.

use std::cell::{Cell, RefCell};

use baa_web::{
    Callback, Device, DialogAnswer, Downloads, MediaAction, PageEvent, PageHandle, Permission, PermissionState, ProfileId,
    Suggestion, WebHost,
};
use gpui::{div, prelude::*, AnyElement, App, Entity, EventEmitter, FocusHandle, Window};

pub type RuleSet = (ProfileId, Permission, String, Option<PermissionState>);

pub struct MockHost {
    pub created: RefCell<Vec<(ProfileId, String)>>,
    /// The URL of each `create_page` a page or extension asked for (`user_initiated: false`).
    pub supplied: RefCell<Vec<String>>,
    /// Ids passed to `download_validate` and `download_discard`.
    pub validated: RefCell<Vec<u64>>,
    pub discarded: RefCell<Vec<u64>>,
    pub downloads: Entity<Downloads>,
    pub permissions: RefCell<Vec<(String, Permission, PermissionState)>>,
    pub deleted: RefCell<Vec<ProfileId>>,
    /// The text of each `autocomplete`; `suggest` runs the last one's `update`.
    pub autocompletes: RefCell<Vec<String>>,
    update: std::rc::Rc<RefCell<Option<baa_web::SuggestUpdate>>>,
    pub extensions: RefCell<Vec<baa_web::Extension>>,
    /// Overrides `extensions` for the profiles listed; `copy_extension` adds to it.
    pub profile_extensions: RefCell<std::collections::HashMap<ProfileId, Vec<baa_web::Extension>>>,
    /// (to, from, id) of each `copy_extension`.
    pub copies: RefCell<Vec<(ProfileId, ProfileId, String)>>,
    pub chrome_tabs: Entity<baa_web::ChromeTabs>,
    pub extension_details: RefCell<Vec<baa_web::ExtensionInfo>>,
    pub extension_changes: RefCell<Vec<(String, baa_web::ExtensionChange)>>,
    /// (popup url, the tab it was opened for).
    pub popups: RefCell<Vec<String>>,
    /// The menus shown, and the item id the next one picks.
    pub menus: RefCell<Vec<Vec<baa_web::MenuItem>>>,
    pub menu_pick: Cell<Option<usize>>,
    /// The last `apply_settings`.
    pub applied: RefCell<Option<baa_model::Settings>>,
    /// What `site_rules` answers, per kind; `set_site_rule` edits it.
    pub rules: RefCell<Vec<(Permission, String, PermissionState)>>,
    /// (profile, kind, pattern, state) of each `set_site_rule`.
    pub rule_sets: RefCell<Vec<RuleSet>>,
    pub clears: RefCell<Vec<(ProfileId, Option<std::time::SystemTime>, baa_web::ClearData)>>,
    pub unpacked: RefCell<Vec<(ProfileId, std::path::PathBuf)>>,
    /// What `load_unpacked` answers.
    pub unpacked_result: RefCell<Result<String, String>>,
    /// Chrome refuses every rule.
    pub refuse_rules: Cell<bool>,
    pub default_browser: Cell<bool>,
    pub made_default: Cell<u32>,
    /// (profile, css, dark) of each `set_webui_skin`.
    pub skins: RefCell<Vec<(ProfileId, String, bool)>>,
}

pub struct MockPage {
    pub url: RefCell<String>,
    pub visible: Cell<bool>,
    /// When it was last shown, counting every page's `set_visible(true)`: Chromium's active tab is the last.
    pub shown_at: Cell<u64>,
    pub zoom: Cell<f64>,
    pub radius: Cell<f32>,
    pub square: Cell<u8>,
    pub moved: Cell<u32>,
    pub floating: Cell<bool>,
    pub muted: Cell<bool>,
    /// What the UI answered the page, in order.
    pub answers: RefCell<Vec<String>>,
    pub answered: RefCell<Vec<(u64, DialogAnswer)>>,
    /// Each `set_group`.
    pub groups: RefCell<Vec<Option<(u64, String)>>>,
    pub exited_fullscreen: Cell<u32>,
    pub reloads: Cell<u32>,
    pub loads: RefCell<Vec<String>>,
    pub focus: FocusHandle,
    pub activations: Cell<u32>,
    /// Each `eval_value` with its `done`, for the test to answer.
    pub evals: RefCell<Vec<(String, Callback<Result<serde_json::Value, String>>)>>,
    pub screenshots: RefCell<Vec<Callback<Result<String, String>>>>,
    /// The other CDP-backed calls, in order: "click 1 2", "insert_text hi",
    /// "press_key Enter", "emulation Some(Phone)", "media Pause", "watch_console".
    pub cdp: RefCell<Vec<String>>,
}

impl EventEmitter<PageEvent> for MockPage {}

impl PageHandle for MockPage {
    fn load_url(&self, url: &str) {
        self.loads.borrow_mut().push(url.into());
        *self.url.borrow_mut() = url.into();
    }
    fn reload(&self, _bypass_cache: bool) {
        self.reloads.set(self.reloads.get() + 1);
    }
    fn stop(&self) {}
    fn go_back(&self) {}
    fn go_forward(&self) {}
    fn url(&self) -> String {
        self.url.borrow().clone()
    }
    fn title(&self) -> String {
        String::new()
    }
    fn is_loading(&self) -> bool {
        false
    }
    fn can_go_back(&self) -> bool {
        false
    }
    fn can_go_forward(&self) -> bool {
        false
    }
    fn history(&self) -> (Vec<(String, String)>, usize) {
        (Vec::new(), 0)
    }
    fn go_to_offset(&self, _: i32) {}
    fn eval(&self, _: &str) {}
    fn set_zoom(&self, level: f64) {
        self.zoom.set(level);
    }
    fn find(&self, _: &str, _: bool) {}
    fn open_devtools(&self, _inspect_at: Option<(i32, i32)>) {}
    fn answer_permission(&self, id: u64, allow: bool) {
        self.answers.borrow_mut().push(format!("permission {id} {allow}"));
    }
    fn answer_dialog(&self, id: u64, answer: DialogAnswer) {
        self.answered.borrow_mut().push((id, answer));
    }
    fn exit_fullscreen(&self) {
        self.exited_fullscreen.set(self.exited_fullscreen.get() + 1);
    }
    fn set_group(&self, group: Option<(u64, &str)>) {
        self.groups.borrow_mut().push(group.map(|(key, title)| (key, title.into())));
    }
    fn print(&self) {}
    fn set_muted(&self, muted: bool) {
        self.muted.set(muted);
    }
    fn is_secure(&self) -> bool {
        self.url.borrow().starts_with("https:")
    }
    fn eval_with_gesture(&self, script: &str) {
        self.answers.borrow_mut().push(format!("gesture {}", script.len()));
    }
    fn set_visible(&self, visible: bool) {
        thread_local!(static SHOWS: Cell<u64> = const { Cell::new(0) });
        self.visible.set(visible);
        if visible {
            self.shown_at.set(SHOWS.with(|n| {
                n.set(n.get() + 1);
                n.get()
            }));
        }
    }
    fn activate(&self) {
        self.activations.set(self.activations.get() + 1);
    }
    fn eval_value(&self, script: &str, done: Callback<Result<serde_json::Value, String>>) {
        self.evals.borrow_mut().push((script.into(), done));
    }
    fn screenshot(&self, done: Callback<Result<String, String>>) {
        self.screenshots.borrow_mut().push(done);
    }
    fn click(&self, x: f32, y: f32) {
        self.cdp.borrow_mut().push(format!("click {x} {y}"));
    }
    fn insert_text(&self, text: &str) {
        self.cdp.borrow_mut().push(format!("insert_text {text}"));
    }
    fn press_key(&self, key: &str) {
        self.cdp.borrow_mut().push(format!("press_key {key}"));
    }
    fn set_emulation(&self, device: Option<Device>) {
        self.cdp.borrow_mut().push(format!("emulation {device:?}"));
    }
    fn set_media_action(&self, action: MediaAction) {
        self.cdp.borrow_mut().push(format!("media {action:?}"));
    }
    fn watch_console(&self) {
        self.cdp.borrow_mut().push("watch_console".into());
    }
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
    fn render(&self, _: &mut Window, _: &mut App) -> AnyElement {
        // Focusable like the real page element, so actions dispatched while the page has
        // focus still bubble up to Browser.
        div().track_focus(&self.focus).into_any_element()
    }
    fn set_corner_radius(&self, radius: f32) {
        self.radius.set(radius);
    }
    fn set_square_corners(&self, square: u8) {
        self.square.set(square);
    }
    fn set_floating(&self, floating: bool) {
        self.floating.set(floating);
    }
    fn move_to_window(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) {
        self.moved.set(self.moved.get() + 1);
    }
}

impl MockHost {
    /// Chrome improves the list of the last `autocomplete`. Deferred: the browser
    /// that owns this host is being updated when a test calls it.
    pub fn suggest(&self, list: Vec<Suggestion>, cx: &mut App) {
        let slot = self.update.clone();
        cx.defer(move |cx| {
            let update = slot.borrow_mut().take();
            if let Some(mut update) = update {
                update(list, cx);
                // Unless `update` itself started a newer query.
                slot.borrow_mut().get_or_insert(update);
            }
        });
    }

    pub fn new(cx: &mut App) -> Self {
        Self {
            created: RefCell::default(),
            supplied: RefCell::default(),
            validated: RefCell::default(),
            discarded: RefCell::default(),
            downloads: cx.new(|_| Downloads::default()),
            permissions: RefCell::default(),
            deleted: RefCell::default(),
            autocompletes: RefCell::default(),
            update: Default::default(),
            extensions: RefCell::default(),
            profile_extensions: RefCell::default(),
            copies: RefCell::default(),
            chrome_tabs: cx.new(|_| baa_web::ChromeTabs::default()),
            extension_details: RefCell::default(),
            extension_changes: RefCell::default(),
            popups: RefCell::default(),
            menus: RefCell::default(),
            menu_pick: Cell::default(),
            applied: RefCell::default(),
            rules: RefCell::default(),
            rule_sets: RefCell::default(),
            clears: RefCell::default(),
            unpacked: RefCell::default(),
            unpacked_result: RefCell::new(Ok("unpacked-id".into())),
            refuse_rules: Cell::default(),
            default_browser: Cell::default(),
            made_default: Cell::default(),
            skins: RefCell::default(),
        }
    }
}

impl WebHost for MockHost {
    type Page = MockPage;
    fn create_devtools(&self, of: &Entity<MockPage>, _inspect_at: Option<(i32, i32)>, _side: &str, window: &mut Window, cx: &mut App) -> Entity<MockPage> {
        let url = format!("devtools://{}", of.read(cx).url.borrow());
        self.create_page(ProfileId(0), &url, true, window, cx)
    }
    fn show_menu(&self, _at: gpui::Point<gpui::Pixels>, items: Vec<baa_web::MenuItem>, window: &mut Window, cx: &mut App, done: baa_web::MenuDone) {
        self.menus.borrow_mut().push(items);
        let pick = self.menu_pick.take();
        window.defer(cx, move |window, cx| done(pick, window, cx));
    }
    fn create_page(
        &self,
        profile: ProfileId,
        url: &str,
        user_initiated: bool,
        _: &mut Window,
        cx: &mut App,
    ) -> Entity<MockPage> {
        self.created.borrow_mut().push((profile, url.into()));
        if !user_initiated {
            self.supplied.borrow_mut().push(url.into());
        }
        cx.new(|cx| MockPage {
            url: RefCell::new(url.into()),
            visible: Cell::new(true),
            shown_at: Cell::new(0),
            zoom: Cell::new(1.0),
            radius: Cell::new(8.0),
            square: Cell::new(0),
            moved: Cell::new(0),
            floating: Cell::new(false),
            muted: Cell::new(false),
            answers: RefCell::default(),
            answered: RefCell::default(),
            groups: RefCell::default(),
            exited_fullscreen: Cell::default(),
            reloads: Cell::default(),
            loads: RefCell::default(),
            focus: cx.focus_handle(),
            activations: Cell::default(),
            evals: RefCell::default(),
            screenshots: RefCell::default(),
            cdp: RefCell::default(),
        })
    }
    fn create_popup(&self, profile: ProfileId, url: &str, window: &mut Window, cx: &mut App) -> Entity<MockPage> {
        self.popups.borrow_mut().push(url.into());
        self.create_page(profile, url, true, window, cx)
    }
    fn ephemeral_profile(&self) -> ProfileId {
        ProfileId(u64::MAX)
    }
    fn delete_profile(&self, profile: ProfileId) {
        self.deleted.borrow_mut().push(profile);
    }
    fn extension_details(
        &self,
        _: ProfileId,
        cx: &mut App,
        done: baa_web::Callback<Option<Vec<baa_web::ExtensionInfo>>>,
    ) {
        done(Some(self.extension_details.borrow().clone()), cx)
    }
    fn change_extension(
        &self,
        _: ProfileId,
        id: &str,
        change: baa_web::ExtensionChange,
        cx: &mut App,
        done: baa_web::Callback<bool>,
    ) {
        self.extension_changes.borrow_mut().push((id.into(), change));
        let mut list = self.extension_details.borrow_mut();
        match change {
            baa_web::ExtensionChange::Remove => list.retain(|e| e.id != id),
            baa_web::ExtensionChange::Reload => {}
            baa_web::ExtensionChange::Enable(on) => list.iter_mut().filter(|e| e.id == id).for_each(|e| e.enabled = on),
            baa_web::ExtensionChange::SiteAccess(a) => {
                list.iter_mut().filter(|e| e.id == id).for_each(|e| e.site_access = Some(a))
            }
        }
        drop(list);
        done(true, cx)
    }
    fn extensions(&self, profile: ProfileId) -> Vec<baa_web::Extension> {
        match self.profile_extensions.borrow().get(&profile) {
            Some(list) => list.clone(),
            None => self.extensions.borrow().clone(),
        }
    }
    fn copy_extension(&self, to: ProfileId, from: ProfileId, id: &str, cx: &mut App, done: baa_web::Callback<Result<String, String>>) {
        self.copies.borrow_mut().push((to, from, id.into()));
        let ext = self.extensions(from).into_iter().find(|e| e.id == id);
        if let Some(ext) = ext {
            self.profile_extensions.borrow_mut().entry(to).or_default().push(ext);
        }
        done(Ok(id.into()), cx)
    }
    fn autocomplete(&self, _: ProfileId, text: &str, update: baa_web::SuggestUpdate) {
        self.autocompletes.borrow_mut().push(text.into());
        *self.update.borrow_mut() = Some(update);
    }
    fn downloads(&self) -> Entity<Downloads> {
        self.downloads.clone()
    }
    fn chrome_tabs(&self) -> Entity<baa_web::ChromeTabs> {
        self.chrome_tabs.clone()
    }
    fn cancel_download(&self, _: u64) {}
    fn download_validate(&self, id: u64) {
        self.validated.borrow_mut().push(id);
    }
    fn download_discard(&self, id: u64) {
        self.discarded.borrow_mut().push(id);
    }
    fn site_permission(&self, _: ProfileId, url: &str, kind: Permission) -> PermissionState {
        self.permissions
            .borrow()
            .iter()
            .rev()
            .find(|(u, k, _)| u == url && *k == kind)
            .map_or(PermissionState::Ask, |(_, _, s)| *s)
    }
    fn set_site_permission(&self, _: ProfileId, url: &str, kind: Permission, state: PermissionState) {
        self.permissions.borrow_mut().push((url.into(), kind, state));
    }
    fn apply_settings(&self, settings: &baa_model::Settings) {
        *self.applied.borrow_mut() = Some(settings.clone());
    }
    fn site_rules(&self, _: ProfileId, kind: Permission) -> Vec<(String, PermissionState)> {
        self.rules.borrow().iter().filter(|(k, ..)| *k == kind).map(|(_, p, s)| (p.clone(), *s)).collect()
    }
    fn set_site_rule(&self, profile: ProfileId, kind: Permission, pattern: &str, state: Option<PermissionState>) -> bool {
        let pattern = baa_web::site_pattern(pattern);
        if pattern.is_empty() || self.refuse_rules.get() {
            return false;
        }
        self.rule_sets.borrow_mut().push((profile, kind, pattern.clone(), state));
        let mut rules = self.rules.borrow_mut();
        rules.retain(|(k, p, _)| !(*k == kind && *p == pattern));
        if let Some(state) = state {
            rules.push((kind, pattern, state));
        }
        true
    }
    fn clear_data(&self, profile: ProfileId, since: Option<std::time::SystemTime>, what: baa_web::ClearData, cx: &mut App, done: baa_web::Callback<()>) {
        self.clears.borrow_mut().push((profile, since, what));
        done((), cx)
    }
    fn load_unpacked(&self, profile: ProfileId, path: &std::path::Path, cx: &mut App, done: baa_web::Callback<Result<String, String>>) {
        self.unpacked.borrow_mut().push((profile, path.into()));
        done(self.unpacked_result.borrow().clone(), cx)
    }
    fn is_default_browser(&self) -> bool {
        self.default_browser.get()
    }
    fn make_default_browser(&self) {
        self.made_default.set(self.made_default.get() + 1);
    }
    fn set_webui_skin(&self, profile: ProfileId, css: String, dark: bool) {
        self.skins.borrow_mut().push((profile, css, dark));
    }
}
