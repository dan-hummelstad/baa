//! [`WebHost`] on CEF, through the vendored gpui-cef (PLAN.md §6, §7).

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui::{div, AnyElement, App, AppContext as _, Entity, EventEmitter, FocusHandle, IntoElement, Window};
use gpui_cef::{Download, ProfileOptions, WebviewEvent, WebviewOptions};

use crate::{Extension, PageEvent, PageHandle, ProfileId, WebHost};

/// A page. gpui-cef's webview implements [`PageHandle`] directly, so the UI
/// holds the same entity CEF paints into.
pub use gpui_cef::Webview as CefPage;

/// Process-wide CEF. [`CefRuntime::init`] at the top of `main`,
/// [`CefRuntime::start`] inside `Application::run`, [`CefRuntime::shutdown`]
/// after it returns. Clone it into the run closure; CEF shuts down when the
/// last clone drops.
#[derive(Clone)]
pub struct CefRuntime {
    runtime: gpui_cef::Runtime,
    root: PathBuf,
}

impl CefRuntime {
    /// `root` is CEF's `root_cache_path`; profiles live in `root/space-<id>`.
    /// When this process is a CEF child process, it exits in here.
    pub fn init(root: PathBuf) -> Result<Self, gpui_cef::Error> {
        purge_deleted_profiles(&root);
        let mut switches: Vec<(String, Option<String>)> = baa_privacy::SWITCHES
            .iter()
            .map(|s| match s.split_once('=') {
                Some((k, v)) => (k.to_string(), Some(v.to_string())),
                None => (s.to_string(), None),
            })
            .collect();
        // gpui-cef merges this with any value already on the command line.
        switches.push((
            "disable-features".into(),
            Some(baa_privacy::merge_disabled_features("")),
        ));
        let runtime = gpui_cef::init(gpui_cef::RuntimeOptions {
            root_cache_path: Some(root.clone()),
            switches,
            log_file: std::env::var_os("BAA_CEF_LOG").map(Into::into),
            ..Default::default()
        })?;
        Ok(Self { runtime, root })
    }

    /// Initialises CEF. Call once, before the first page.
    pub fn start(&self, cx: &mut App) -> Result<CefWebHost, gpui_cef::Error> {
        self.runtime.start(cx)?;
        Ok(CefWebHost::new(self.root.clone(), cx))
    }

    pub fn shutdown(self) {
        self.runtime.shutdown();
    }
}

/// Every download any page started this session, oldest first.
#[derive(Default)]
pub struct Downloads {
    pub items: Vec<Download>,
}

impl Downloads {
    fn upsert(&mut self, download: Download) {
        match self.items.iter_mut().find(|item| item.id == download.id) {
            Some(item) => *item = download,
            None => self.items.push(download),
        }
    }
}

pub struct CefWebHost {
    root: PathBuf,
    profiles: RefCell<HashMap<ProfileId, gpui_cef::Profile>>,
    next_ephemeral: Cell<u64>,
    downloads: Entity<Downloads>,
}

impl CefWebHost {
    fn new(root: PathBuf, cx: &mut App) -> Self {
        Self {
            root,
            profiles: RefCell::default(),
            next_ephemeral: Cell::new(u64::MAX),
            downloads: cx.new(|_| Downloads::default()),
        }
    }

    /// The download tray's data; observe it for updates.
    pub fn downloads(&self) -> Entity<Downloads> {
        self.downloads.clone()
    }

    fn profile(&self, id: ProfileId) -> Option<gpui_cef::Profile> {
        if let Some(profile) = self.profiles.borrow().get(&id) {
            return Some(profile.clone());
        }
        let filter: gpui_cef::FilterHook = Arc::new(|request| {
            baa_privacy::block_browser_request(
                request.url,
                request.resource_type,
                request.page_initiated,
            )
        });
        let options = ProfileOptions {
            cache_path: (!is_ephemeral(id)).then(|| profile_dir(&self.root, id)),
            filter,
            prefs_off: baa_privacy::PREFS_OFF,
        };
        match gpui_cef::Profile::new(options) {
            Ok(profile) => {
                self.profiles.borrow_mut().insert(id, profile.clone());
                Some(profile)
            }
            Err(err) => {
                // Falls back to the global in-memory context: the page still
                // loads, nothing of it persists.
                log::error!("profile {id:?}: {err}");
                None
            }
        }
    }
}

impl WebHost for CefWebHost {
    type Page = CefPage;

    fn create_page(
        &self,
        profile: ProfileId,
        url: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<CefPage> {
        let options = WebviewOptions {
            profile: self.profile(profile),
            // The page card's radius in baa-ui; Peek sets its own.
            corner_radius: 8.,
            ..WebviewOptions::url(url)
        };
        let page = cx.new(|cx| CefPage::new(window, cx, options));
        let downloads = self.downloads.clone();
        cx.subscribe(&page, move |page, event: &WebviewEvent, cx| {
            let event = match event.clone() {
                WebviewEvent::UrlChanged(url) => PageEvent::UrlChanged(url),
                WebviewEvent::TitleChanged(title) => PageEvent::TitleChanged(title),
                WebviewEvent::FaviconChanged(url) => PageEvent::FaviconChanged(url),
                WebviewEvent::FaviconImage { url, png } => PageEvent::FaviconImage { url, png },
                WebviewEvent::LoadingChanged(loading) => PageEvent::LoadingChanged(loading),
                WebviewEvent::HistoryChanged {
                    can_go_back,
                    can_go_forward,
                } => PageEvent::HistoryChanged {
                    can_go_back,
                    can_go_forward,
                },
                WebviewEvent::OpenRequested {
                    url,
                    from_user_gesture,
                    background,
                } => PageEvent::OpenRequested {
                    url,
                    from_user_gesture,
                    background,
                },
                WebviewEvent::ContextMenu {
                    x,
                    y,
                    link_url,
                    selection,
                } => PageEvent::ContextMenu {
                    x,
                    y,
                    link_url,
                    selection,
                },
                WebviewEvent::Closed => PageEvent::Closed,
                WebviewEvent::Download(download) => {
                    downloads.update(cx, |list, cx| {
                        list.upsert(download);
                        cx.notify();
                    });
                    return;
                }
                WebviewEvent::Message(_) => return,
            };
            page.update(cx, |_, cx| cx.emit(event));
        })
        .detach();
        page
    }

    fn ephemeral_profile(&self) -> ProfileId {
        let id = self.next_ephemeral.get();
        self.next_ephemeral.set(id - 1);
        ProfileId(id)
    }

    #[cfg(target_os = "macos")]
    fn open_extensions(&self, profile: ProfileId, url: &str, cx: &mut App) {
        let (profile, url) = (self.profile(profile), url.to_string());
        // A profile created moments ago is not usable for a native browser yet.
        cx.spawn(async move |cx| {
            for _ in 0..20 {
                if gpui_cef::open_chrome_window(profile.as_ref(), &url) {
                    return;
                }
                cx.background_executor().timer(std::time::Duration::from_millis(250)).await;
            }
            log::error!("could not open the extensions window");
        })
        .detach();
    }

    // ponytail: Windows still runs on WebView2, which has no Chrome extension UI.
    #[cfg(not(target_os = "macos"))]
    fn open_extensions(&self, _profile: ProfileId, _url: &str, _cx: &mut App) {
        log::warn!("extensions are macOS-only for now");
    }

    fn fetch(&self, profile: ProfileId, url: &str, done: Box<dyn FnOnce(Option<String>)>) {
        gpui_cef::fetch(self.profile(profile).as_ref(), url, done)
    }

    fn extensions(&self, profile: ProfileId) -> Vec<Extension> {
        if is_ephemeral(profile) {
            return Vec::new();
        }
        installed_extensions(&profile_dir(&self.root, profile))
    }

    fn delete_profile(&self, profile: ProfileId) {
        self.profiles.borrow_mut().remove(&profile);
        if is_ephemeral(profile) {
            return;
        }
        // CEF keeps files open until the profile's last browser has closed, and
        // closing is asynchronous: removal can fail (Windows) or race a late
        // write, so it is also queued for the next launch.
        queue_delete(&self.root, profile);
        let _ = std::fs::remove_dir_all(profile_dir(&self.root, profile));
    }
}

impl EventEmitter<PageEvent> for CefPage {}

impl PageHandle for CefPage {
    fn load_url(&self, url: &str) {
        CefPage::load_url(self, url)
    }
    fn reload(&self) {
        CefPage::reload(self)
    }
    fn stop(&self) {
        CefPage::stop(self)
    }
    fn go_back(&self) {
        CefPage::go_back(self)
    }
    fn go_forward(&self) {
        CefPage::go_forward(self)
    }
    fn url(&self) -> String {
        CefPage::url(self)
    }
    fn title(&self) -> String {
        CefPage::title(self)
    }
    fn is_loading(&self) -> bool {
        CefPage::is_loading(self)
    }
    fn can_go_back(&self) -> bool {
        CefPage::can_go_back(self)
    }
    fn can_go_forward(&self) -> bool {
        CefPage::can_go_forward(self)
    }
    fn history(&self) -> (Vec<(String, String)>, usize) {
        let (entries, current) = CefPage::history(self);
        (entries.into_iter().map(|e| (e.title, e.url)).collect(), current)
    }
    fn go_to_offset(&self, offset: i32) {
        CefPage::go_to_offset(self, offset)
    }
    fn eval(&self, script: &str) {
        CefPage::eval(self, script)
    }
    fn set_zoom(&self, level: f64) {
        CefPage::set_zoom(self, level)
    }
    fn find(&self, text: &str, forward: bool) {
        CefPage::find(self, text, forward)
    }
    fn open_devtools(&self) {
        self.show_dev_tools()
    }
    fn set_visible(&self, visible: bool) {
        CefPage::set_visible(self, visible)
    }
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        gpui::Focusable::focus_handle(self, cx)
    }
    fn render(&self, _window: &mut Window, _cx: &mut App) -> AnyElement {
        match self.weak_entity().upgrade() {
            Some(page) => page.into_any_element(),
            None => div().into_any_element(),
        }
    }
    fn set_corner_radius(&self, radius: f32) {
        CefPage::set_corner_radius(self, radius)
    }
    fn set_floating(&self, floating: bool) {
        CefPage::set_floating(self, floating)
    }
    #[cfg(target_os = "macos")]
    fn move_to_window(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        CefPage::move_to_window(self, window, cx)
    }
    // ponytail: WebView2 children cannot change windows; the page stays put.
    #[cfg(not(target_os = "macos"))]
    fn move_to_window(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) {}
}

/// Chrome keeps each extension in `Extensions/<id>/<version>/`; this reads the
/// newest version's manifest (and `_locales` for a `__MSG_name__` name).
// ponytail: lists what is on disk, so an extension disabled in
// chrome://extensions still shows. Ceiling: read Preferences for its state.
fn installed_extensions(profile: &Path) -> Vec<Extension> {
    let Ok(ids) = std::fs::read_dir(profile.join("Extensions")) else {
        return Vec::new();
    };
    let mut out: Vec<Extension> = ids
        .flatten()
        .filter_map(|id| {
            let mut versions: Vec<PathBuf> = std::fs::read_dir(id.path())
                .ok()?
                .flatten()
                .map(|v| v.path())
                .filter(|v| v.join("manifest.json").exists())
                .collect();
            versions.sort();
            let dir = versions.pop()?;
            let manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).ok()?).ok()?;
            let id = id.file_name().to_string_lossy().into_owned();
            let action = manifest.get("action").or_else(|| manifest.get("browser_action"));
            let icon = action
                .and_then(|a| a.get("default_icon"))
                .or_else(|| manifest.get("icons"))
                .and_then(|icons| match icons {
                    serde_json::Value::String(path) => Some(path.clone()),
                    serde_json::Value::Object(sizes) => ["32", "48", "38", "64", "16", "128"]
                        .iter()
                        .find_map(|s| sizes.get(*s))
                        .or_else(|| sizes.values().next())
                        .and_then(|v| v.as_str().map(String::from)),
                    _ => None,
                })
                .map(|path| dir.join(path.trim_start_matches('/')));
            let popup = action
                .and_then(|a| a.get("default_popup"))
                .and_then(|p| p.as_str())
                .map(|p| format!("chrome-extension://{id}/{}", p.trim_start_matches('/')));
            let name = manifest.get("name").and_then(|n| n.as_str()).unwrap_or(&id).to_string();
            let name = localized(&dir, &manifest, &name).unwrap_or(name);
            Some(Extension { id, name, icon, popup })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn localized(dir: &Path, manifest: &serde_json::Value, name: &str) -> Option<String> {
    let key = name.strip_prefix("__MSG_")?.strip_suffix("__")?;
    let locale = manifest.get("default_locale")?.as_str()?;
    let messages: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.join("_locales").join(locale).join("messages.json")).ok()?,
    )
    .ok()?;
    // Chrome matches message keys case-insensitively.
    let map = messages.as_object()?;
    let entry = map.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))?.1;
    Some(entry.get("message")?.as_str()?.to_string())
}

// Ids count down from u64::MAX; baa-model's count up from 1.
fn is_ephemeral(id: ProfileId) -> bool {
    id.0 > u64::MAX / 2
}


fn profile_dir(root: &Path, id: ProfileId) -> PathBuf {
    // Chrome rejects nested profile paths ("Cannot create profile"), so no spaces/ subdir.
    root.join(format!("space-{}", id.0))
}

const PENDING_DELETES: &str = "pending-deletes";

fn queue_delete(root: &Path, id: ProfileId) {
    use std::io::Write as _;
    let line = format!("{}\n", id.0);
    let written = std::fs::create_dir_all(root).and_then(|()| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join(PENDING_DELETES))?
            .write_all(line.as_bytes())
    });
    if let Err(err) = written {
        log::warn!("could not queue profile {id:?} for deletion: {err}");
    }
}

/// Runs before CEF opens anything in `root`, so nothing holds the files.
fn purge_deleted_profiles(root: &Path) {
    let list = root.join(PENDING_DELETES);
    let Ok(text) = std::fs::read_to_string(&list) else {
        return;
    };
    // Parsing as a number keeps a tampered file from naming another path.
    for id in text.lines().filter_map(|line| line.trim().parse().ok()) {
        let dir = profile_dir(root, ProfileId(id));
        if let Err(err) = std::fs::remove_dir_all(&dir) {
            if dir.exists() {
                log::warn!("could not delete {}: {err}", dir.display());
            }
        }
    }
    let _ = std::fs::remove_file(list);
}

#[cfg(test)]
mod extension_tests {
    use super::*;

    #[test]
    fn reads_name_icon_and_popup_from_the_newest_version() {
        let root = std::env::temp_dir().join(format!("baa-ext-{}", std::process::id()));
        let dir = root.join("Extensions/abc/1.0_0");
        std::fs::create_dir_all(dir.join("_locales/en")).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"__MSG_extName__","default_locale":"en","action":{"default_icon":{"16":"i16.png","32":"i32.png"},"default_popup":"popup.html"}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("_locales/en/messages.json"), r#"{"extname":{"message":"Blocker"}}"#).unwrap();
        let found = installed_extensions(&root);
        assert_eq!(
            found,
            [Extension {
                id: "abc".into(),
                name: "Blocker".into(),
                icon: Some(dir.join("i32.png")),
                popup: Some("chrome-extension://abc/popup.html".into()),
            }]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ephemeral_ids_never_meet_model_ids() {
        assert!(is_ephemeral(ProfileId(u64::MAX)));
        assert!(!is_ephemeral(ProfileId(1)));
        assert!(!is_ephemeral(ProfileId(1 << 40)));
    }

    #[test]
    fn queued_deletes_run_at_next_launch() {
        let root = std::env::temp_dir().join(format!("baa-web-purge-{}", std::process::id()));
        let keep = profile_dir(&root, ProfileId(1));
        let gone = profile_dir(&root, ProfileId(2));
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::create_dir_all(gone.join("Cache")).unwrap();

        queue_delete(&root, ProfileId(2));
        std::fs::write(
            root.join(PENDING_DELETES),
            std::fs::read_to_string(root.join(PENDING_DELETES)).unwrap() + "../../etc\n",
        )
        .unwrap();
        purge_deleted_profiles(&root);

        assert!(keep.exists());
        assert!(!gone.exists());
        assert!(!root.join(PENDING_DELETES).exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn downloads_update_in_place() {
        let download = |id, received| Download {
            id,
            url: "https://example.com/a.zip".into(),
            path: String::new(),
            received_bytes: received,
            total_bytes: 10,
            state: crate::DownloadState::InProgress,
        };
        let mut list = Downloads::default();
        list.upsert(download(1, 0));
        list.upsert(download(2, 0));
        list.upsert(download(1, 5));
        assert_eq!(list.items.len(), 2);
        assert_eq!(list.items[0].received_bytes, 5);
    }

}
