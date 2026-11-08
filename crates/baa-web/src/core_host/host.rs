//! [`WebHost`] on BaaCore: profiles, pages, extensions, downloads.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::PathBuf,
};

use gpui::{App, AppContext as _, Entity, Pixels, Window};

use super::{
    c_string, extensions, ffi,
    ffi::Profile,
    dispatch::Dispatch,
    menu,
    page::CorePage,
    profiles::{is_ephemeral, queue_delete},
    settings::{self, Write, Writes},
    skin, Core, PageKind,
};
use crate::{
    Callback, ChromeTabs, ClearData, Downloads, Extension, ExtensionChange, ExtensionInfo, MenuDone, MenuItem,
    PageHandle, Permission, PermissionState, ProfileId, WebHost,
};

/// `BAA_PERMISSION_*` bits, in bit order.
const PERMISSION_BITS: [(u32, Permission); 15] = [
    (ffi::PERMISSION_CAMERA, Permission::Camera),
    (ffi::PERMISSION_MICROPHONE, Permission::Microphone),
    (ffi::PERMISSION_LOCATION, Permission::Location),
    (ffi::PERMISSION_NOTIFICATIONS, Permission::Notifications),
    (ffi::PERMISSION_CLIPBOARD, Permission::Clipboard),
    (ffi::PERMISSION_MIDI, Permission::Midi),
    (ffi::PERMISSION_OTHER, Permission::Other),
    (ffi::PERMISSION_POPUPS, Permission::Popups),
    (ffi::PERMISSION_SOUND, Permission::Sound),
    (ffi::PERMISSION_JAVASCRIPT, Permission::JavaScript),
    (ffi::PERMISSION_COOKIES, Permission::Cookies),
    (ffi::PERMISSION_AUTOMATIC_DOWNLOADS, Permission::AutomaticDownloads),
    (ffi::PERMISSION_INSECURE_CONTENT, Permission::InsecureContent),
    (ffi::PERMISSION_SENSORS, Permission::Sensors),
    (ffi::PERMISSION_WINDOW_MANAGEMENT, Permission::WindowManagement),
];

pub(super) fn permissions_of(kinds: u32) -> Vec<Permission> {
    PERMISSION_BITS.iter().filter(|(bit, _)| kinds & bit != 0).map(|(_, kind)| *kind).collect()
}

/// The bit a per-site setting exists for; screen sharing and the rest have none.
pub(super) fn permission_bit(kind: Permission) -> Option<u32> {
    PERMISSION_BITS.iter().find(|(_, k)| *k == kind && *k != Permission::Other).map(|(bit, _)| *bit)
}

pub(super) fn state_code(state: PermissionState) -> i32 {
    match state {
        PermissionState::Ask => ffi::SITE_ASK,
        PermissionState::Allow => ffi::SITE_ALLOW,
        PermissionState::Block => ffi::SITE_BLOCK,
        PermissionState::SessionOnly => ffi::SITE_SESSION_ONLY,
    }
}

fn state_of(code: i32) -> PermissionState {
    match code {
        ffi::SITE_ALLOW => PermissionState::Allow,
        ffi::SITE_BLOCK => PermissionState::Block,
        ffi::SITE_SESSION_ONLY => PermissionState::SessionOnly,
        _ => PermissionState::Ask,
    }
}

pub struct CoreWebHost {
    core: &'static Core,
    dispatch: &'static Dispatch,
    /// `--user-data-dir`: profiles live in `root/space-<id>`.
    root: PathBuf,
    profiles: RefCell<HashMap<ProfileId, *mut Profile>>,
    next_ephemeral: Cell<u64>,
    downloads: Entity<Downloads>,
    chrome_tabs: Entity<ChromeTabs>,
    /// What `apply_settings` last sent; none before the first call.
    applied: RefCell<Option<Writes>>,
    /// Kept as the user chose it until it is sent to the engine.
    page_scheme: Cell<baa_model::PageScheme>,
    /// Per Space, so a profile opened later inherits: the css of its Chrome pages, and whether it is dark.
    skins: RefCell<HashMap<ProfileId, (String, bool)>>,
}

impl CoreWebHost {
    pub(super) fn new(core: &'static Core, dispatch: &'static Dispatch, root: PathBuf, cx: &mut App) -> Self {
        let chrome_tabs = cx.new(|_| ChromeTabs::default());
        *dispatch.chrome_tabs.borrow_mut() = Some(chrome_tabs.downgrade());
        let downloads = cx.new(|_| Downloads::default());
        *dispatch.downloads.borrow_mut() = Some(downloads.downgrade());
        *dispatch.app.borrow_mut() = Some(cx.to_async());
        Self {
            core,
            dispatch,
            root,
            profiles: RefCell::default(),
            next_ephemeral: Cell::new(u64::MAX),
            downloads,
            chrome_tabs,
            applied: RefCell::default(),
            page_scheme: Cell::default(),
            skins: RefCell::default(),
        }
    }

    /// Ephemeral profiles have no extensions.
    fn read_extensions(&self, profile: ProfileId) -> Vec<(Extension, ExtensionInfo)> {
        if is_ephemeral(profile) {
            return Vec::new();
        }
        let handle = self.profile_handle(profile);
        if handle.is_null() {
            return Vec::new();
        }
        extensions::read(self.core, handle)
    }

    fn send_schemes(&self, id: ProfileId, handle: *mut Profile) {
        let website = settings::website_scheme(self.page_scheme.get());
        // Without a skin Chrome's pages keep their own look, in the system's scheme.
        let webui = self.skins.borrow().get(&id).map_or(0, |(_, dark)| settings::scheme_code(*dark));
        (self.core.profile_set_color_schemes)(handle, website, webui);
    }

    pub(super) fn profile_handle(&self, id: ProfileId) -> *mut Profile {
        if let Some(handle) = self.profiles.borrow().get(&id) {
            return *handle;
        }
        let handle = if is_ephemeral(id) {
            (self.core.profile_open_ephemeral)()
        } else {
            (self.core.profile_open)(c_string(&format!("space-{}", id.0)).as_ptr())
        };
        if handle.is_null() {
            log::error!("profile {id:?}: Chrome refused it");
        } else {
            self.profiles.borrow_mut().insert(id, handle);
            // Before the pins below, so the privacy ones always win.
            if let Some(writes) = self.applied.borrow().as_ref() {
                for write in &writes.profile {
                    settings::apply(self.core, handle, write);
                }
            }
            self.send_schemes(id, handle);
            // Most are Helium's defaults already; this pins them if one was flipped in settings.
            for pref in baa_privacy::PREFS_OFF {
                (self.core.pref_set_bool)(handle, c_string(pref).as_ptr(), false);
            }
        }
        handle
    }
}

impl WebHost for CoreWebHost {
    type Page = CorePage;

    fn create_devtools(
        &self,
        of: &Entity<CorePage>,
        inspect_at: Option<(i32, i32)>,
        side: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<CorePage> {
        let (profile, inspected) = of.read_with(cx, |page, _| (page.profile, page.shared.tab.get()));
        let handle = self.profile_handle(profile);
        let (x, y) = inspect_at.unwrap_or_default();
        let side = match side {
            "bottom" => ffi::DOCK_BOTTOM,
            "left" => ffi::DOCK_LEFT,
            _ => ffi::DOCK_RIGHT,
        };
        // 0: Chrome refused, or the inspected page is gone; the page then closes itself.
        let tab = if inspected == 0 {
            0
        } else {
            (self.core.tab_open_devtools)(inspected, inspect_at.is_some(), x, y, side)
        };
        let dispatch = self.dispatch;
        cx.new(|cx| {
            let page = CorePage::new(dispatch, profile, handle, tab, PageKind::DevTools, window, cx);
            if tab == 0 {
                page.close_soon(cx);
            }
            page
        })
    }

    fn show_menu(&self, at: gpui::Point<Pixels>, items: Vec<MenuItem>, window: &mut Window, cx: &mut App, done: MenuDone) {
        menu::show_menu(window, cx, at, items, done);
    }

    fn create_page(&self, profile: ProfileId, url: &str, user_initiated: bool, window: &mut Window, cx: &mut App) -> Entity<CorePage> {
        let handle = self.profile_handle(profile);
        let tab = if handle.is_null() {
            0
        } else {
            // A tab Chrome already made for this (window.open) is taken, not duplicated.
            let url = skin::to_baa(url.to_string());
            let css = self.skins.borrow().get(&profile).map(|(css, _)| css.clone()).unwrap_or_default();
            let pending = if user_initiated { None } else { self.dispatch.take_pending(handle, &url) };
            // ponytail: Chrome's verdict on a page-supplied URL went with its
            // tab, so baa takes no risk: only http(s) is opened again; data:,
            // baa://, file:, chrome-extension:// and the rest show a blank page.
            let url = if user_initiated || pending.is_some() || is_web_or_blank(&url) { url } else { "about:blank".into() };
            // Chrome's own pages start blank: the skin script has to be in before their first document.
            let tab = pending.unwrap_or_else(|| {
                let first = if skin::is_webui(&url) { "about:blank" } else { &url };
                (self.core.tab_create)(handle, c_string(first).as_ptr())
            });
            if tab != 0 {
                self.dispatch.prepare_tab(tab, &url, &css, pending.is_some());
            }
            tab
        };
        let dispatch = self.dispatch;
        cx.new(|cx| CorePage::new(dispatch, profile, handle, tab, PageKind::Tab, window, cx))
    }

    fn create_popup(&self, profile: ProfileId, url: &str, window: &mut Window, cx: &mut App) -> Entity<CorePage> {
        // The action runs on the profile's real active tab (no tabs shim).
        let handle = self.profile_handle(profile);
        let id = extensions::id_of(url).filter(|_| !handle.is_null());
        // NULL: no popup (the click was all the action did); the Peek card closes.
        let tab = id.map_or(0, |id| (self.core.extension_popup)(handle, c_string(id).as_ptr()));
        let dispatch = self.dispatch;
        cx.new(|cx| {
            let page = CorePage::new(dispatch, profile, handle, tab, PageKind::Popup, window, cx);
            page.set_corner_radius(8.);
            if tab == 0 {
                page.close_soon(cx);
            }
            page
        })
    }

    fn ephemeral_profile(&self) -> ProfileId {
        let id = self.next_ephemeral.get();
        self.next_ephemeral.set(id - 1);
        ProfileId(id)
    }

    fn delete_profile(&self, profile: ProfileId) {
        self.skins.borrow_mut().remove(&profile);
        if let Some(handle) = self.profiles.borrow_mut().remove(&profile) {
            (self.core.profile_close)(handle);
        }
        if !is_ephemeral(profile) {
            // Next launch removes the directory, before Chrome opens anything in it.
            queue_delete(&self.root, profile);
        }
    }

    fn extension_details(&self, profile: ProfileId, cx: &mut App, done: Callback<Option<Vec<ExtensionInfo>>>) {
        let mut infos: Vec<ExtensionInfo> = self.read_extensions(profile).into_iter().map(|(_, info)| info).collect();
        infos.sort_by_key(|info| info.name.to_lowercase());
        done(Some(infos), cx);
    }

    fn change_extension(&self, profile: ProfileId, id: &str, change: ExtensionChange, cx: &mut App, done: Callback<bool>) {
        let (change, value) = extensions::change_code(change);
        let handle = if is_ephemeral(profile) { std::ptr::null_mut() } else { self.profile_handle(profile) };
        let done_ok = !handle.is_null() && (self.core.profile_extension_change)(handle, c_string(id).as_ptr(), change, value);
        done(done_ok, cx);
    }

    fn extensions(&self, profile: ProfileId) -> Vec<Extension> {
        let mut list: Vec<Extension> =
            self.read_extensions(profile).into_iter().filter(|(_, info)| info.enabled).map(|(e, _)| e).collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    fn autocomplete(&self, profile: ProfileId, text: &str, update: crate::SuggestUpdate) {
        let handle = self.profile_handle(profile);
        if handle.is_null() {
            return;
        }
        // Replacing drops the old callback; Chrome restarts its controller itself.
        let id = self.dispatch.autocomplete_started(update);
        (self.core.profile_autocomplete)(handle, id, c_string(text).as_ptr());
    }

    fn downloads(&self) -> Entity<Downloads> {
        self.downloads.clone()
    }

    fn chrome_tabs(&self) -> Entity<ChromeTabs> {
        self.chrome_tabs.clone()
    }

    fn cancel_download(&self, id: u64) {
        (self.core.download_cancel)(id);
    }

    fn download_validate(&self, id: u64) {
        (self.core.download_validate)(id);
    }

    fn download_discard(&self, id: u64) {
        (self.core.download_discard)(id);
    }

    fn site_permission(&self, profile: ProfileId, url: &str, kind: Permission) -> PermissionState {
        let handle = self.profile_handle(profile);
        let Some(bit) = permission_bit(kind).filter(|_| !handle.is_null()) else { return PermissionState::Ask };
        state_of((self.core.profile_site_permission)(handle, c_string(url).as_ptr(), bit))
    }

    fn set_site_permission(&self, profile: ProfileId, url: &str, kind: Permission, state: PermissionState) {
        let handle = self.profile_handle(profile);
        let Some(bit) = permission_bit(kind).filter(|_| !handle.is_null()) else { return };
        (self.core.profile_set_site_permission)(handle, c_string(url).as_ptr(), bit, state_code(state));
    }

    fn apply_settings(&self, settings: &baa_model::Settings) {
        let new = settings::writes(settings);
        let old = self.applied.replace(Some(new.clone())).unwrap_or_default();
        for write in new.local.iter().filter(|w| !old.local.contains(w)) {
            settings::apply(self.core, std::ptr::null_mut(), write);
        }
        let changed: Vec<&Write> = new.profile.iter().filter(|w| !old.profile.contains(w)).collect();
        for &handle in self.profiles.borrow().values() {
            for write in &changed {
                settings::apply(self.core, handle, write);
            }
        }
        if self.page_scheme.replace(settings.page_scheme) != settings.page_scheme {
            for (&id, &handle) in self.profiles.borrow().iter() {
                self.send_schemes(id, handle);
            }
        }
    }

    fn site_rules(&self, profile: ProfileId, kind: Permission) -> Vec<(String, PermissionState)> {
        extern "C" fn visit(rules: *mut std::ffi::c_void, pattern: *const std::ffi::c_char, state: i32) {
            let rules = unsafe { &mut *rules.cast::<Vec<(String, PermissionState)>>() };
            rules.push((unsafe { super::from_c(pattern) }, state_of(state)));
        }
        let handle = self.profile_handle(profile);
        let Some(bit) = permission_bit(kind).filter(|_| !handle.is_null()) else { return Vec::new() };
        let mut rules = Vec::new();
        (self.core.profile_site_rules)(handle, bit, visit, (&mut rules as *mut Vec<_>).cast());
        rules
    }

    fn set_site_rule(&self, profile: ProfileId, kind: Permission, pattern: &str, state: Option<PermissionState>) -> bool {
        let handle = self.profile_handle(profile);
        let pattern = crate::site_pattern(pattern);
        let Some(bit) = permission_bit(kind).filter(|_| !handle.is_null() && !pattern.is_empty()) else { return false };
        let code = state.map_or(ffi::SITE_UNSET, state_code);
        let done = (self.core.profile_set_site_rule)(handle, bit, c_string(&pattern).as_ptr(), code);
        if !done {
            log::warn!("Chrome refused the {kind:?} rule {pattern:?}");
        }
        done
    }

    fn clear_data(&self, profile: ProfileId, since: Option<std::time::SystemTime>, what: ClearData, _cx: &mut App, done: Callback<()>) {
        let id = self.dispatch.clears.add(done);
        let handle = self.profile_handle(profile);
        if handle.is_null() {
            return self.dispatch.finish_clear(id);
        }
        let since_ms = since
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as i64);
        let types = [
            (what.history, ffi::CLEAR_HISTORY),
            (what.downloads, ffi::CLEAR_DOWNLOADS),
            (what.site_data, ffi::CLEAR_SITE_DATA),
            (what.cache, ffi::CLEAR_CACHE),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .fold(0, |bits, (_, bit)| bits | bit);
        (self.core.profile_clear_data)(handle, id, since_ms, types);
    }

    fn load_unpacked(&self, profile: ProfileId, path: &std::path::Path, _cx: &mut App, done: Callback<Result<String, String>>) {
        let id = self.dispatch.unpacked.add(done);
        let handle = if is_ephemeral(profile) { std::ptr::null_mut() } else { self.profile_handle(profile) };
        if handle.is_null() {
            return self.dispatch.finish_unpacked(id, Err("This Space cannot have extensions".into()));
        }
        (self.core.profile_load_unpacked)(handle, id, c_string(&path.to_string_lossy()).as_ptr());
    }

    fn copy_extension(&self, to: ProfileId, from: ProfileId, ext: &str, _cx: &mut App, done: Callback<Result<String, String>>) {
        let id = self.dispatch.unpacked.add(done);
        let handle = |p| if is_ephemeral(p) { std::ptr::null_mut() } else { self.profile_handle(p) };
        let (to, from) = (handle(to), handle(from));
        if to.is_null() || from.is_null() {
            return self.dispatch.finish_unpacked(id, Err("This Space cannot have extensions".into()));
        }
        (self.core.profile_copy_extension)(to, id, from, c_string(ext).as_ptr());
    }

    fn is_default_browser(&self) -> bool {
        (self.core.is_default_browser)()
    }

    fn make_default_browser(&self) {
        (self.core.set_default_browser)();
    }

    fn set_webui_skin(&self, profile: ProfileId, css: String, dark: bool) {
        let unchanged = self.skins.borrow_mut().insert(profile, (css.clone(), dark)).is_some_and(|old| old == (css.clone(), dark));
        // A Space that is not open picks the skin up when it opens.
        let handle = self.profiles.borrow().get(&profile).copied();
        if let Some(handle) = handle.filter(|_| !unchanged) {
            self.dispatch.restyle_profile(handle, &css);
            self.send_schemes(profile, handle);
        }
    }
}

fn is_web_or_blank(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://") || url == "about:blank"
}
