//! [`WebHost`] on CEF, through the vendored gpui-cef (PLAN.md §6, §7).

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui::{div, AnyElement, App, AppContext as _, Entity, EventEmitter, FocusHandle, IntoElement, Window};
use gpui_cef::{Download, ProfileOptions, WebviewEvent, WebviewOptions};

use crate::{PageEvent, PageHandle, ProfileId, WebHost};

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
            // Matches the page card's rounded_lg in baa-ui's browser.rs.
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
                } => PageEvent::OpenRequested {
                    url,
                    from_user_gesture,
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
