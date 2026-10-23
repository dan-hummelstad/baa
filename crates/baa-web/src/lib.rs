//! CEF host. The UI codes against [`WebHost`]/[`PageHandle`] only (PLAN.md §12);
//! CEF types never cross this crate's boundary.

mod cef_host;

use gpui::{AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Window};

pub use baa_model::ProfileId;
pub use cef_host::{CefPage, CefRuntime, CefWebHost};
pub use gpui_cef::{DialogKind, Download, DownloadState, Error, Permission, PermissionState};

/// What a page reports back to the shell.
#[derive(Debug, Clone, PartialEq)]
pub enum PageEvent {
    UrlChanged(String),
    TitleChanged(String),
    FaviconChanged(Option<String>),
    /// The favicon at `url`, as PNG.
    FaviconImage { url: String, png: std::sync::Arc<Vec<u8>> },
    LoadingChanged(bool),
    HistoryChanged { can_go_back: bool, can_go_forward: bool },
    /// Page asked to open a new window/tab (window.open, target=_blank, shift- or
    /// middle-click). `background`: cmd- or middle-click, a tab for later.
    OpenRequested { url: String, from_user_gesture: bool, background: bool },
    /// Page requested a context menu at window-local coordinates.
    ContextMenu { x: f32, y: f32, link_url: Option<String>, selection: Option<String> },
    Closed,
    /// The site waits until [`PageHandle::answer_permission`].
    PermissionRequested { id: u64, origin: String, kinds: Vec<Permission> },
    PermissionDismissed(u64),
    /// The page is paused until [`PageHandle::answer_dialog`].
    Dialog { kind: DialogKind, message: String, default_text: String },
    DialogClosed,
    AudibleChanged(bool),
    /// This page started a download (it lands in the host's [`Downloads`]).
    DownloadStarted,
    /// An extension popup asked to load `url` in the tab it is for; `None` reloads it.
    ActiveTabLoad(Option<String>),
    /// An extension popup's content size changed.
    PreferredSize(gpui::Size<gpui::Pixels>),
}

/// URLs waiting to be opened as tabs, oldest first.
#[derive(Default)]
pub struct ChromeTabs(pub Vec<String>);

/// Every download any page started this session, oldest first.
#[derive(Default)]
pub struct Downloads {
    pub items: Vec<Download>,
}

impl Downloads {
    pub fn upsert(&mut self, download: Download) {
        match self.items.iter_mut().find(|item| item.id == download.id) {
            Some(item) => *item = download,
            None => self.items.push(download),
        }
    }
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
    /// The back/forward list as (title, url), and the current entry's index.
    fn history(&self) -> (Vec<(String, String)>, usize);
    /// Moves through history; negative is back.
    fn go_to_offset(&self, offset: i32);
    fn eval(&self, script: &str);
    /// `level` is a factor: 1.0 is 100%.
    fn set_zoom(&self, level: f64);
    /// Repeating the same `text` moves to the next match; empty `text` ends the search.
    fn find(&self, text: &str, forward: bool);
    fn open_devtools(&self);
    fn answer_permission(&self, id: u64, allow: bool);
    /// OK (with the prompt's text) or Cancel; for "leave site?", OK leaves.
    fn answer_dialog(&self, ok: bool, text: &str);
    fn print(&self);
    fn set_muted(&self, muted: bool);
    /// Arrived over valid TLS.
    fn is_secure(&self) -> bool;
    /// Runs `script` as if the user had just clicked (Picture in Picture needs that).
    fn eval_with_gesture(&self, script: &str);
    /// Hidden pages keep state but stop painting (background tabs).
    fn set_visible(&self, visible: bool);
    /// Focus this to send keyboard input to the page (after closing the command bar, say).
    fn focus_handle(&self, cx: &App) -> FocusHandle;
    /// Render the page for this frame at the element's laid-out bounds.
    fn render(&self, window: &mut Window, cx: &mut App) -> AnyElement;
    /// Match the rounded card the page sits in, in logical pixels.
    fn set_corner_radius(&self, radius: f32);
    /// Drawn over other pages (Peek). Pages are native views, which stack by
    /// creation rather than by the UI's paint order.
    fn set_floating(&self, floating: bool);
    /// Move the live page (state, scroll, session) into another window, e.g.
    /// from Peek or lil-baa into the browser.
    fn move_to_window(&mut self, window: &mut Window, cx: &mut Context<Self>)
    where
        Self: Sized;
}

/// Creates pages inside isolated storage profiles.
pub trait WebHost: 'static {
    type Page: PageHandle;
    fn create_page(&self, profile: ProfileId, url: &str, window: &mut Window, cx: &mut App) -> Entity<Self::Page>;
    /// An extension's toolbar popup, acting on the tab at `tab_url`.
    fn create_popup(
        &self,
        profile: ProfileId,
        url: &str,
        tab_url: &str,
        tab_title: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self::Page>;
    /// In-memory profile; nothing persisted (incognito Space).
    fn ephemeral_profile(&self) -> ProfileId;
    /// Drop a profile's on-disk data once all its pages are closed.
    fn delete_profile(&self, profile: ProfileId);
    /// Everything Chrome's extensions page shows about `profile`'s extensions,
    /// for baa's own; `done` gets `None` if it could not be read.
    fn extension_details(&self, profile: ProfileId, cx: &mut App, done: Callback<Option<Vec<ExtensionInfo>>>);
    /// Chrome's own page (`chrome://settings…`) over part of `window`,
    /// restyled with `sheet.css`; `None` hides it. False while it is not up
    /// yet: call again.
    fn show_chrome_page(&self, profile: ProfileId, sheet: Option<ChromeSheet>, window: &mut Window) -> bool;
    /// Carries out `change`; `done` once it has, with whether it worked.
    fn change_extension(&self, profile: ProfileId, id: &str, change: ExtensionChange, cx: &mut App, done: Callback<bool>);
    /// Extensions installed in `profile`, for the toolbar.
    fn extensions(&self, profile: ProfileId) -> Vec<Extension>;
    /// GETs `url` through `profile`'s network stack (search suggestions);
    /// `done` gets the body, or `None`. Called on the main thread.
    fn fetch(&self, profile: ProfileId, url: &str, done: Box<dyn FnOnce(Option<String>)>);
    /// The download tray's data; observe it for updates.
    fn downloads(&self) -> Entity<Downloads>;
    /// Pages Chrome itself wanted open (an extension's `chrome.tabs.create`);
    /// observe it and take them as tabs.
    fn chrome_tabs(&self) -> Entity<ChromeTabs>;
    fn cancel_download(&self, id: u32);
    /// What `profile` remembers about `kind` for `url`'s site.
    fn site_permission(&self, profile: ProfileId, url: &str, kind: Permission) -> PermissionState;
    fn set_site_permission(&self, profile: ProfileId, url: &str, kind: Permission, state: PermissionState);
}

/// Where and how [`WebHost::show_chrome_page`] shows Chrome's page.
#[derive(Debug, Clone, PartialEq)]
pub struct ChromeSheet {
    pub url: String,
    /// In window coordinates.
    pub bounds: gpui::Bounds<gpui::Pixels>,
    pub corner_radius: f32,
    /// A stylesheet over Chrome's own: baa's skin.
    pub css: String,
    /// Whether the skin is dark, so Chrome's own light/dark rules match it.
    pub dark: bool,
}

/// Called once with the answer, on the main thread.
pub type Callback<T> = Box<dyn FnOnce(T, &mut App)>;

/// An extension as Chrome's extensions page knows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExtensionInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub enabled: bool,
    /// What it may do, in Chrome's words ("Read your browsing history").
    pub permissions: Vec<String>,
    /// Where it may run, when that is the user's choice.
    pub site_access: Option<SiteAccess>,
    pub options: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiteAccess {
    OnClick,
    OnSpecificSites,
    OnAllSites,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionChange {
    Enable(bool),
    SiteAccess(SiteAccess),
    Remove,
}

/// An installed extension, as the toolbar shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Extension {
    pub id: String,
    pub name: String,
    pub icon: Option<std::path::PathBuf>,
    /// Its toolbar popup page, if it has one.
    pub popup: Option<String>,
    /// Its settings page, if it has one.
    pub options: Option<String>,
}
