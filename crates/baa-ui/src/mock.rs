//! Test double for `WebHost` so UI tests run without CEF.

use std::cell::{Cell, RefCell};

use baa_web::{PageEvent, PageHandle, ProfileId, WebHost};
use gpui::{div, prelude::*, AnyElement, App, Entity, EventEmitter, FocusHandle, Window};

#[derive(Default)]
pub struct MockHost {
    pub created: RefCell<Vec<(ProfileId, String)>>,
    pub deleted: RefCell<Vec<ProfileId>>,
}

pub struct MockPage {
    pub url: RefCell<String>,
    pub visible: Cell<bool>,
    pub zoom: Cell<f64>,
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
    fn eval(&self, _: &str) {}
    fn set_zoom(&self, level: f64) {
        self.zoom.set(level);
    }
    fn find(&self, _: &str, _: bool) {}
    fn open_devtools(&self) {}
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
}

impl WebHost for MockHost {
    type Page = MockPage;
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
            focus: cx.focus_handle(),
        })
    }
    fn ephemeral_profile(&self) -> ProfileId {
        ProfileId(u64::MAX)
    }
    fn delete_profile(&self, profile: ProfileId) {
        self.deleted.borrow_mut().push(profile);
    }
}
