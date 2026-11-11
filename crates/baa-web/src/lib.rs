//! BaaCore host. The UI codes against [`WebHost`]/[`PageHandle`] only; no C ABI
//! types cross this crate's boundary.

#[cfg(target_os = "macos")]
mod core_host;

use gpui::{AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Window};

pub use baa_model::{Permission, PermissionState, ProfileId};
#[cfg(target_os = "macos")]
pub use core_host::{
    chrome_argv,
    profiles::{migrate_cef_profiles, purge_deleted_profiles, share_native_messaging_hosts},
    CorePage, CoreRuntime, CoreWebHost,
};

/// An entry of a native context menu (see [`WebHost::show_menu`]).
#[derive(Debug, Clone, PartialEq)]
pub enum MenuItem {
    /// `id` is what [`WebHost::show_menu`] reports when it is picked.
    Action { label: String, enabled: bool, checked: bool, id: usize },
    Separator,
    Submenu { label: String, items: Vec<MenuItem> },
}

impl MenuItem {
    pub fn action(label: impl Into<String>, id: usize) -> Self {
        Self::Action { label: label.into(), enabled: true, checked: false, id }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt,
    BeforeUnload { is_reload: bool },
    /// HTTP auth: a username and a password field.
    Auth,
    /// A link wants an app (mailto:, zoommtg:).
    ExternalProtocol,
    /// The page stopped responding: OK ends it, Cancel waits.
    Unresponsive,
    /// An extension asks to be added, re-enabled, or for more permissions.
    ExtensionInstall,
}

/// A question the page or Chrome waits on, until [`PageHandle::answer_dialog`]
/// or [`PageEvent::AskClosed`]. JS dialogs leave `title` and the labels empty.
#[derive(Debug, Clone, PartialEq)]
pub struct Dialog {
    pub id: u64,
    pub kind: DialogKind,
    pub message: String,
    pub default_text: String,
    pub title: String,
    pub ok_label: String,
    pub cancel_label: String,
    /// Empty: no checkbox.
    pub check_label: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DialogAnswer {
    pub ok: bool,
    /// A prompt's text, or the username.
    pub text: String,
    pub password: String,
    pub checked: bool,
}

/// One of Chrome's omnibox matches.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub url: String,
    pub text: String,
    pub description: String,
    /// A search or calculator result, not a page.
    pub search: bool,
}

/// A snapshot of one download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    /// Stable across updates of the same download for the life of the process.
    pub id: u64,
    pub url: String,
    /// Empty until the destination is chosen.
    pub path: String,
    pub received_bytes: i64,
    /// `-1` while unknown.
    pub total_bytes: i64,
    pub state: DownloadState,
    /// From an incognito profile: nothing about it may be written to disk.
    pub ephemeral: bool,
    /// Chrome holds the file until [`WebHost::download_validate`] (Keep) or
    /// [`WebHost::download_discard`]; `None` once decided or never risky.
    pub danger: Option<DangerKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DangerKind {
    /// Chrome flagged the file type or content as dangerous.
    Dangerous,
    /// An http file from an https page (mixed content).
    Insecure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadState {
    InProgress,
    Complete,
    Canceled,
}

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
    Closed,
    /// The site waits until [`PageHandle::answer_permission`].
    PermissionRequested { id: u64, origin: String, kinds: Vec<Permission> },
    Dialog(Dialog),
    /// The permission ask or dialog `id` went away unanswered.
    AskClosed(u64),
    /// The page went fullscreen, or left it. While on, the UI shows the page alone.
    Fullscreen(bool),
    /// Chrome's fullscreen / pointer lock / keyboard lock notice: show `origin`
    /// and how to leave, so a page cannot pose as the browser. `None` hides it.
    ExclusiveAccess { origin: String, kind: Option<ExclusiveKind> },
    /// Find in page: the active match (1-based, 0 for none) and the count so far.
    FindResult { active: u32, total: u32 },
    /// The renderer is gone; `reload` brings it back.
    Crashed(bool),
    /// An extension put the tab in group `group` (a key from `set_group`, or a
    /// new one) titled `title`, or took it out (`None`).
    GroupChanged { group: Option<u64>, title: String },
    AudibleChanged(bool),
    /// This page started a download (it lands in the host's [`Downloads`]).
    DownloadStarted,
    /// An extension popup's content size changed.
    PreferredSize(gpui::Size<gpui::Pixels>),
    /// The page's menu asked to inspect the element at (x, y), page coordinates.
    DevToolsRequested { x: i32, y: i32 },
    /// Docked DevTools' own dock-side picker chose this ("right", "bottom",
    /// "left", or "undocked").
    DevToolsDock(String),
    /// The page got keyboard focus (a click on it, say): split view makes it the focused pane.
    Focused,
    /// A console message or an uncaught exception; only after [`PageHandle::watch_console`].
    Console { level: ConsoleLevel, text: String },
    /// The tab's media session changed.
    Media(Media),
    /// The user clicked back to tab on the tab's picture-in-picture window.
    FocusRequested,
    /// The tab's audio was muted or unmuted, by the app or by Chrome.
    Muted(bool),
    /// A two-finger history swipe over the page, for the shell to draw.
    /// `forward`: towards the next entry. `amount` 0..1 is how far across; it
    /// keeps changing while the swipe settles after the fingers lift. The
    /// engine checked the tab can go that way, and navigates itself after `Done`.
    Swipe { phase: SwipePhase, forward: bool, amount: f32 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum SwipePhase {
    /// With the page the swipe goes to, as it looked when it was left; `None`
    /// when the engine has no picture of it.
    Began(Option<SwipePicture>),
    Changed,
    /// Gone through: the tab goes back or forward now.
    Done,
    Cancelled,
}

/// A page's picture: BGRA, premultiplied, top row first, in pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct SwipePicture {
    pub width: u32,
    pub height: u32,
    pub bgra: std::sync::Arc<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleLevel {
    Debug,
    Log,
    Info,
    Warning,
    /// `console.error`, a failed `console.assert`, or an uncaught exception.
    Error,
}

/// A tab's media session, as Chrome's media controls see it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Media {
    /// The page's, else the page title.
    pub title: String,
    pub artist: String,
    pub artwork: Option<String>,
    pub playing: bool,
    /// What [`PageHandle::set_media_action`] can do now.
    pub actions: Vec<MediaAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    Play,
    Pause,
    Next,
    Previous,
    PictureInPicture,
    /// Closes the tab's PiP window if it has one; never in [`Media::actions`].
    ExitPictureInPicture,
}

/// What [`PageHandle::set_emulation`] shows the page as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    /// 393×852 at 3x, mobile, touch, and an iPhone's user agent.
    Phone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExclusiveKind {
    /// "Press Esc to exit full screen".
    Fullscreen,
    /// The mouse is captured, the page is not fullscreen.
    PointerLock,
    /// Esc goes to the page: "Press and hold Esc to exit".
    KeyboardLock,
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

/// One live page. Implemented by the BaaCore backend.
pub trait PageHandle: EventEmitter<PageEvent> + 'static {
    fn load_url(&self, url: &str);
    fn reload(&self, bypass_cache: bool);
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
    /// DevTools in a window of its own; `inspect_at` (page coordinates) selects that element.
    fn open_devtools(&self, inspect_at: Option<(i32, i32)>);
    fn answer_permission(&self, id: u64, allow: bool);
    /// OK (with the prompt's text) or Cancel; for "leave site?", OK leaves.
    fn answer_dialog(&self, id: u64, answer: DialogAnswer);
    /// The user left page fullscreen through the app (window left macOS fullscreen, Space switch).
    fn exit_fullscreen(&self);
    /// Chrome's tab group for this tab, so extensions see its folder; `None`
    /// takes it out. The UI's keys keep the top bit clear.
    fn set_group(&self, group: Option<(u64, &str)>);
    fn print(&self);
    fn set_muted(&self, muted: bool);
    /// Arrived over valid TLS.
    fn is_secure(&self) -> bool;
    /// Runs `script` as if the user had just clicked (Picture in Picture needs that).
    fn eval_with_gesture(&self, script: &str);
    /// Hidden pages keep state but stop painting (background tabs). Only shows
    /// or hides; see [`PageHandle::activate`].
    fn set_visible(&self, visible: bool);
    /// Makes this Chrome's active tab of its window, which permission prompts,
    /// dialogs and extension actions follow. Call it for the one tab the user
    /// is on, after `set_visible`. Popups and DevTools ignore it.
    fn activate(&self);
    /// Runs `script` in the page's main world and gets its result as JSON
    /// (promises awaited), or the exception's text. `done` is dropped if the
    /// page closes first.
    fn eval_value(&self, script: &str, done: Callback<Result<serde_json::Value, String>>);
    /// The page's viewport as a base64 PNG.
    fn screenshot(&self, done: Callback<Result<String, String>>);
    /// A real left click (trusted events) at CSS pixels from the viewport's top left.
    fn click(&self, x: f32, y: f32);
    /// Types `text` into the focused element, as a keyboard or IME would.
    fn insert_text(&self, text: &str);
    /// Presses and releases `key`: a DOM key name ("Enter", "ArrowDown") or one character.
    fn press_key(&self, key: &str);
    /// Shows the page as `device` would see it, or as itself again (`None`).
    fn set_emulation(&self, device: Option<Device>);
    fn set_media_action(&self, action: MediaAction);
    /// From now on the page reports [`PageEvent::Console`]. Off by default, so
    /// normal browsing does not pay for it.
    fn watch_console(&self);
    /// Focus this to send keyboard input to the page (after closing the command bar, say).
    fn focus_handle(&self, cx: &App) -> FocusHandle;
    /// Render the page for this frame at the element's laid-out bounds.
    fn render(&self, window: &mut Window, cx: &mut App) -> AnyElement;
    /// Match the rounded card the page sits in, in logical pixels.
    fn set_corner_radius(&self, radius: f32);
    /// Corners left square where the page meets docked DevTools: top left 1,
    /// top right 2, bottom left 4, bottom right 8.
    fn set_square_corners(&self, square: u8);
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
    /// DevTools for `of` as a page of its own, to dock on `side` ("right",
    /// "bottom", "left"); dropping it closes DevTools.
    fn create_devtools(&self, of: &Entity<Self::Page>, inspect_at: Option<(i32, i32)>, side: &str, window: &mut Window, cx: &mut App) -> Entity<Self::Page>;
    /// A native context menu at `at` (window coordinates); returns at once,
    /// `done` runs when it closes.
    fn show_menu(&self, at: gpui::Point<gpui::Pixels>, items: Vec<MenuItem>, window: &mut Window, cx: &mut App, done: MenuDone);
    /// `user_initiated`: the user typed or picked `url` (command bar, settings,
    /// a tab they had open); it is loaded as given, `baa://` included.
    /// Otherwise a page or extension supplied it ([`PageEvent::OpenRequested`],
    /// [`ChromeTabs`]): Chrome's own tab for it is adopted, and if that is gone
    /// only an `http(s)` URL is opened, anything else shows `about:blank`. A
    /// page-supplied URL is never re-issued as a browser load, so pass `false`.
    fn create_page(&self, profile: ProfileId, url: &str, user_initiated: bool, window: &mut Window, cx: &mut App) -> Entity<Self::Page>;
    /// An extension's toolbar popup, acting on the profile's active tab.
    fn create_popup(&self, profile: ProfileId, url: &str, window: &mut Window, cx: &mut App) -> Entity<Self::Page>;
    /// In-memory profile; nothing persisted (incognito Space).
    fn ephemeral_profile(&self) -> ProfileId;
    /// Drop a profile's on-disk data once all its pages are closed.
    fn delete_profile(&self, profile: ProfileId);
    /// Everything Chrome's extensions page shows about `profile`'s extensions,
    /// for baa's own; `done` gets `None` if it could not be read.
    fn extension_details(&self, profile: ProfileId, cx: &mut App, done: Callback<Option<Vec<ExtensionInfo>>>);
    /// Carries out `change`; `done` once it has, with whether it worked.
    fn change_extension(&self, profile: ProfileId, id: &str, change: ExtensionChange, cx: &mut App, done: Callback<bool>);
    /// Extensions installed in `profile`, for the toolbar.
    fn extensions(&self, profile: ProfileId) -> Vec<Extension>;
    /// Chrome's omnibox matches for `text` in `profile`, best first. `update`
    /// runs each time the list improves; the next call (or empty `text`) ends
    /// the previous one, whose `update` is dropped.
    fn autocomplete(&self, profile: ProfileId, text: &str, update: SuggestUpdate);
    /// The download tray's data; observe it for updates.
    fn downloads(&self) -> Entity<Downloads>;
    /// Pages Chrome itself wanted open (an extension's `chrome.tabs.create`);
    /// observe it and take them as tabs.
    fn chrome_tabs(&self) -> Entity<ChromeTabs>;
    fn cancel_download(&self, id: u64);
    /// The user chose Keep for a download with `danger`: Chrome finishes it.
    fn download_validate(&self, id: u64);
    /// The user chose Discard: the item and its partial file are removed.
    fn download_discard(&self, id: u64);
    /// What `profile` remembers about `kind` for `url`'s site.
    fn site_permission(&self, profile: ProfileId, url: &str, kind: Permission) -> PermissionState;
    fn set_site_permission(&self, profile: ProfileId, url: &str, kind: Permission, state: PermissionState);
    /// baa's global browser settings into every profile, now and as each one
    /// opens. Sends only what changed since the last call.
    fn apply_settings(&self, settings: &baa_model::Settings);
    /// The rules `profile` has for `kind` (a site setting, never `Other`): the
    /// default first, with pattern `"*"`, then each exception by Chrome's pattern
    /// (`"[*.]example.com"`). Empty for a gone profile.
    fn site_rules(&self, profile: ProfileId, kind: Permission) -> Vec<(String, PermissionState)>;
    /// `"*"` sets the default, anything else an exception, as typed: see [`site_pattern`].
    /// `None` removes the rule (for `"*"`, restores Chrome's default). False when Chrome
    /// refused the rule, which then changes nothing.
    fn set_site_rule(&self, profile: ProfileId, kind: Permission, pattern: &str, state: Option<PermissionState>) -> bool;
    /// Deletes what `what` names that `profile` stored since `since` (`None`: ever);
    /// `done` runs on a later turn. Chrome's stores only; baa-model keeps its own records.
    fn clear_data(&self, profile: ProfileId, since: Option<std::time::SystemTime>, what: ClearData, cx: &mut App, done: Callback<()>);
    /// Loads the unpacked extension in folder `path` (developer mode); `done`
    /// gets its id, or Chrome's error text.
    fn load_unpacked(&self, profile: ProfileId, path: &std::path::Path, cx: &mut App, done: Callback<Result<String, String>>);
    /// Installs `from`'s extension `id` into `to` without a prompt (see `baa_profile_copy_extension`).
    /// `Ok` with the id, also when `to` already has it.
    fn copy_extension(&self, to: ProfileId, from: ProfileId, id: &str, cx: &mut App, done: Callback<Result<String, String>>);
    /// Whether baa is the system's default browser for http and https.
    fn is_default_browser(&self) -> bool;
    /// macOS asks the user; check again with `is_default_browser` later.
    fn make_default_browser(&self);
    /// A stylesheet over `profile`'s Chrome pages (`baa://...`); `dark` says
    /// whether its colours are dark, so Chrome's own light/dark rules match.
    fn set_webui_skin(&self, profile: ProfileId, css: String, dark: bool);
}

/// Chrome's content-setting pattern for what a person typed: a bare host or `*.host`
/// covers its subdomains too (`[*.]host`); `*`, `[*.]host` and URLs stay as they are.
pub fn site_pattern(text: &str) -> String {
    let t = text.trim();
    if let Some(host) = t.strip_prefix("*.") {
        return format!("[*.]{host}");
    }
    if t.is_empty() || t.contains(['[', '*', '/']) {
        t.to_string()
    } else {
        format!("[*.]{t}")
    }
}

/// What [`WebHost::clear_data`] deletes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClearData {
    pub history: bool,
    pub downloads: bool,
    /// Cookies, storage, service workers.
    pub site_data: bool,
    pub cache: bool,
}

/// What [`WebHost::autocomplete`] calls with each better list, on the main thread.
pub type SuggestUpdate = Box<dyn FnMut(Vec<Suggestion>, &mut App)>;

/// Called once with the answer, on the main thread.
pub type Callback<T> = Box<dyn FnOnce(T, &mut App)>;

/// What [`WebHost::show_menu`] calls once the menu closes: the picked item's id, or `None`.
pub type MenuDone = Box<dyn FnOnce(Option<usize>, &mut Window, &mut App)>;

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
    /// Loaded from a folder (developer mode); only these reload.
    pub unpacked: bool,
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
    /// Unpacked extensions only.
    Reload,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_update_in_place() {
        let download = |id, received| Download {
            id,
            url: "https://example.com/a.zip".into(),
            path: String::new(),
            received_bytes: received,
            total_bytes: 10,
            state: DownloadState::InProgress,
            ephemeral: false,
            danger: None,
        };
        let mut list = Downloads::default();
        list.upsert(Download { danger: Some(DangerKind::Insecure), ..download(1, 0) });
        list.upsert(download(2, 0));
        list.upsert(download(1, 5));
        assert_eq!(list.items.len(), 2);
        assert_eq!(list.items[0].received_bytes, 5);
        assert_eq!(list.items[0].danger, None, "Keep clears the warning");
    }
}
