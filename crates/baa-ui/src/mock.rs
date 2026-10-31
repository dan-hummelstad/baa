//! Test double for `WebHost` so UI tests run without CEF.

use std::cell::{Cell, RefCell};

use baa_web::{Downloads, PageEvent, PageHandle, Permission, PermissionState, ProfileId, WebHost};
use gpui::{div, prelude::*, AnyElement, App, Entity, EventEmitter, FocusHandle, Window};

pub struct MockHost {
    pub created: RefCell<Vec<(ProfileId, String)>>,
    pub downloads: Entity<Downloads>,
    pub permissions: RefCell<Vec<(String, Permission, PermissionState)>>,
    pub deleted: RefCell<Vec<ProfileId>>,
    pub fetched: RefCell<Vec<String>>,
    pub suggest_body: RefCell<Option<String>>,
    pub extensions: RefCell<Vec<baa_web::Extension>>,
    pub chrome_tabs: Entity<baa_web::ChromeTabs>,
    pub extension_details: RefCell<Vec<baa_web::ExtensionInfo>>,
    pub extension_changes: RefCell<Vec<(String, baa_web::ExtensionChange)>>,
    /// What the Chrome settings sheet last showed.
    pub chrome_sheet: RefCell<Option<baa_web::ChromeSheet>>,
    /// (popup url, the tab it was opened for).
    pub popups: RefCell<Vec<(String, String)>>,
    /// The menus shown, and the item id the next one picks.
    pub menus: RefCell<Vec<Vec<baa_web::MenuItem>>>,
    pub menu_pick: Cell<Option<usize>>,
}

pub struct MockPage {
    pub url: RefCell<String>,
    pub visible: Cell<bool>,
    pub zoom: Cell<f64>,
    pub radius: Cell<f32>,
    pub square: Cell<u8>,
    pub moved: Cell<u32>,
    pub floating: Cell<bool>,
    pub muted: Cell<bool>,
    /// What the UI answered the page, in order.
    pub answers: RefCell<Vec<String>>,
    pub focus: FocusHandle,
}

impl EventEmitter<PageEvent> for MockPage {}

impl PageHandle for MockPage {
    fn load_url(&self, url: &str) {
        *self.url.borrow_mut() = url.into();
    }
    fn reload(&self) {}
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
    fn answer_dialog(&self, ok: bool, text: &str) {
        self.answers.borrow_mut().push(format!("dialog {ok} {text}"));
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
        self.visible.set(visible);
    }
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
    fn render(&self, _: &mut Window, _: &mut App) -> AnyElement {
        // Focusable like the CEF element, so actions dispatched while the page has
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
    pub fn new(cx: &mut App) -> Self {
        Self {
            created: RefCell::default(),
            downloads: cx.new(|_| Downloads::default()),
            permissions: RefCell::default(),
            deleted: RefCell::default(),
            fetched: RefCell::default(),
            suggest_body: RefCell::default(),
            extensions: RefCell::default(),
            chrome_tabs: cx.new(|_| baa_web::ChromeTabs::default()),
            extension_details: RefCell::default(),
            extension_changes: RefCell::default(),
            chrome_sheet: RefCell::default(),
            popups: RefCell::default(),
            menus: RefCell::default(),
            menu_pick: Cell::default(),
        }
    }
}

impl WebHost for MockHost {
    type Page = MockPage;
    fn create_devtools(&self, of: &Entity<MockPage>, _inspect_at: Option<(i32, i32)>, _side: &str, window: &mut Window, cx: &mut App) -> Entity<MockPage> {
        let url = format!("devtools://{}", of.read(cx).url.borrow());
        self.create_page(ProfileId(0), &url, window, cx)
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
        _: &mut Window,
        cx: &mut App,
    ) -> Entity<MockPage> {
        self.created.borrow_mut().push((profile, url.into()));
        cx.new(|cx| MockPage {
            url: RefCell::new(url.into()),
            visible: Cell::new(true),
            zoom: Cell::new(1.0),
            radius: Cell::new(8.0),
            square: Cell::new(0),
            moved: Cell::new(0),
            floating: Cell::new(false),
            muted: Cell::new(false),
            answers: RefCell::default(),
            focus: cx.focus_handle(),
        })
    }
    fn create_popup(
        &self,
        profile: ProfileId,
        url: &str,
        tab_url: &str,
        _: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<MockPage> {
        self.popups.borrow_mut().push((url.into(), tab_url.into()));
        self.create_page(profile, url, window, cx)
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
    fn show_chrome_page(&self, _: ProfileId, sheet: Option<baa_web::ChromeSheet>, _: &mut Window) -> bool {
        *self.chrome_sheet.borrow_mut() = sheet;
        true
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
            baa_web::ExtensionChange::Enable(on) => list.iter_mut().filter(|e| e.id == id).for_each(|e| e.enabled = on),
            baa_web::ExtensionChange::SiteAccess(a) => {
                list.iter_mut().filter(|e| e.id == id).for_each(|e| e.site_access = Some(a))
            }
        }
        drop(list);
        done(true, cx)
    }
    fn extensions(&self, _: ProfileId) -> Vec<baa_web::Extension> {
        self.extensions.borrow().clone()
    }
    fn fetch(&self, _: ProfileId, url: &str, done: Box<dyn FnOnce(Option<String>)>) {
        self.fetched.borrow_mut().push(url.into());
        done(self.suggest_body.borrow().clone())
    }
    fn downloads(&self) -> Entity<Downloads> {
        self.downloads.clone()
    }
    fn chrome_tabs(&self) -> Entity<baa_web::ChromeTabs> {
        self.chrome_tabs.clone()
    }
    fn cancel_download(&self, _: u32) {}
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
}
