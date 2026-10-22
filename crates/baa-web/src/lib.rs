//! CEF host. The UI codes against [`WebHost`]/[`PageHandle`] only (PLAN.md §12);
//! CEF types never cross this crate's boundary.

mod cef_host;

use gpui::{AnyElement, App, Entity, EventEmitter, FocusHandle, Window};

pub use baa_model::ProfileId;
pub use cef_host::{CefPage, CefRuntime, CefWebHost, Downloads};
pub use gpui_cef::{Download, DownloadState, Error};

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
    /// `level` is a factor: 1.0 is 100%.
    fn set_zoom(&self, level: f64);
    /// Repeating the same `text` moves to the next match; empty `text` ends the search.
    fn find(&self, text: &str, forward: bool);
    fn open_devtools(&self);
    /// Hidden pages keep state but stop painting (background tabs).
    fn set_visible(&self, visible: bool);
    /// Focus this to send keyboard input to the page (after closing the command bar, say).
    fn focus_handle(&self, cx: &App) -> FocusHandle;
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
