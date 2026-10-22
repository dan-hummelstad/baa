//! CEF host. The UI codes against [`WebHost`]/[`PageHandle`] only (PLAN.md §12);
//! CEF types never cross this crate's boundary.

use gpui::{AnyElement, App, Entity, EventEmitter, Window};

/// Identifies a storage profile = one CEF request context (PLAN.md §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ProfileId(pub u64);

/// What a page reports back to the shell.
#[derive(Debug, Clone, PartialEq)]
pub enum PageEvent {
    UrlChanged(String),
    TitleChanged(String),
    FaviconChanged(Option<String>),
    LoadingChanged(bool),
    HistoryChanged { can_go_back: bool, can_go_forward: bool },
    /// Page asked to open a new window/tab (window.open, target=_blank, middle click).
    OpenRequested { url: String, from_user_gesture: bool },
    /// Page requested a context menu at window-local coordinates.
    ContextMenu { x: f32, y: f32, link_url: Option<String>, selection: Option<String> },
    Closed,
}

/// One live page. Implemented by the CEF backend (and later WKWebView/Android).
pub trait PageHandle: EventEmitter<PageEvent> + 'static {
    fn load_url(&self, url: &str);
    fn reload(&self);
    fn stop(&self);
    fn go_back(&self);
    fn go_forward(&self);
    fn url(&self) -> String;
    fn title(&self) -> String;
    fn is_loading(&self) -> bool;
    fn can_go_back(&self) -> bool;
    fn can_go_forward(&self) -> bool;
    fn eval(&self, script: &str);
    fn set_zoom(&self, level: f64);
    fn find(&self, text: &str, forward: bool);
    fn open_devtools(&self);
    /// Hidden pages keep state but stop painting (background tabs).
    fn set_visible(&self, visible: bool);
    /// Render the page for this frame at the element's laid-out bounds.
    fn render(&self, window: &mut Window, cx: &mut App) -> AnyElement;
}

/// Creates pages inside isolated storage profiles.
pub trait WebHost: 'static {
    type Page: PageHandle;
    fn create_page(&self, profile: ProfileId, url: &str, window: &mut Window, cx: &mut App) -> Entity<Self::Page>;
    /// In-memory profile; nothing persisted (incognito Space).
    fn ephemeral_profile(&self) -> ProfileId;
    /// Drop a profile's on-disk data once all its pages are closed.
    fn delete_profile(&self, profile: ProfileId);
}
