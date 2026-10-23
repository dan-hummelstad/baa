//! [`WebHost`] on CEF, through the vendored gpui-cef (PLAN.md §6, §7).

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui::{div, AnyElement, App, AppContext as _, Entity, EventEmitter, FocusHandle, IntoElement, Window};
use gpui_cef::{ProfileOptions, WebviewEvent, WebviewOptions};

use crate::{Callback, ChromeSheet, ChromeTabs, Downloads, ExtensionChange, ExtensionInfo, SiteAccess, Extension, PageEvent, PageHandle, Permission, PermissionState, ProfileId, WebHost};

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
        share_native_messaging_hosts(&root);
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

pub struct CefWebHost {
    root: PathBuf,
    profiles: RefCell<HashMap<ProfileId, gpui_cef::Profile>>,
    next_ephemeral: Cell<u64>,
    downloads: Entity<Downloads>,
    chrome_tabs: Entity<ChromeTabs>,
    /// Per profile, the hidden chrome://extensions page baa's own drives.
    extension_pages: Rc<RefCell<HashMap<ProfileId, gpui_cef::ChromePage>>>,
    /// Per profile, Chrome's settings shown as a sheet: the page, its URL and skin.
    chrome_pages: RefCell<HashMap<ProfileId, ChromeSheetPage>>,
}

impl CefWebHost {
    fn new(root: PathBuf, cx: &mut App) -> Self {
        let chrome_tabs = cx.new(|_| ChromeTabs::default());
        {
            let (tabs, app) = (chrome_tabs.downgrade(), cx.to_async());
            // CEF calls in from its message loop, which may run inside an app
            // update; the push waits for the next turn.
            gpui_cef::on_chrome_tab(move |url| {
                let tabs = tabs.clone();
                app.spawn(async move |cx| {
                    tabs.update(cx, |tabs, cx| {
                        tabs.0.push(url);
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            });
        }
        Self {
            root,
            profiles: RefCell::default(),
            next_ephemeral: Cell::new(u64::MAX),
            downloads: cx.new(|_| Downloads::default()),
            chrome_tabs,
            extension_pages: Rc::default(),
            chrome_pages: RefCell::default(),
        }
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

impl CefWebHost {
    /// Runs `script` (an expression, maybe a promise) on `profile`'s hidden
    /// chrome://extensions page; `done` gets its value, or `None` if it threw
    /// or the page never came up.
    fn on_extensions_page(
        &self,
        profile: ProfileId,
        script: String,
        cx: &mut App,
        done: Callback<Option<serde_json::Value>>,
    ) {
        let script = format!(
            "(async () => {{ if (!chrome.developerPrivate) throw 'loading'; return JSON.stringify((await ({script})) ?? null); }})()"
        );
        let page = ExtensionsPage {
            pages: self.extension_pages.clone(),
            id: profile,
            profile: self.profile(profile),
            script,
        };
        page.run(cx.to_async(), 40, done);
    }

    /// A page; with `init_script`, an extension popup that the script lets act
    /// on the active tab.
    fn open(
        &self,
        profile: ProfileId,
        url: &str,
        init_script: Option<String>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<CefPage> {
        let popup = init_script.is_some();
        let options = WebviewOptions {
            profile: self.profile(profile),
            // The page card's radius in baa-ui; Peek sets its own.
            corner_radius: 8.,
            init_script,
            // Chrome's limits for an extension popup.
            auto_resize: popup.then(|| (gpui::size(gpui::px(25.), gpui::px(25.)), gpui::size(gpui::px(800.), gpui::px(600.)))),
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
                    let new = downloads.update(cx, |list, cx| {
                        let new = list.items.iter().all(|d| d.id != download.id);
                        list.upsert(download);
                        cx.notify();
                        new
                    });
                    if !new {
                        return;
                    }
                    PageEvent::DownloadStarted
                }
                WebviewEvent::PermissionRequested { id, origin, kinds } => {
                    PageEvent::PermissionRequested { id, origin, kinds }
                }
                WebviewEvent::PermissionDismissed(id) => PageEvent::PermissionDismissed(id),
                WebviewEvent::Dialog { kind, message, default_text } => {
                    PageEvent::Dialog { kind, message, default_text }
                }
                WebviewEvent::DialogClosed => PageEvent::DialogClosed,
                WebviewEvent::AudibleChanged(on) => PageEvent::AudibleChanged(on),
                WebviewEvent::PreferredSize(size) => PageEvent::PreferredSize(size),
                // Only the tabs shim posts; see `tabs_shim`.
                WebviewEvent::Message(m) if popup => {
                    let web = |url: &str| url.starts_with("http") || url.starts_with("chrome-extension://");
                    match m.split_once(':') {
                        Some(("load", url)) if web(url) => PageEvent::ActiveTabLoad(Some(url.into())),
                        Some(("open", url)) if web(url) => PageEvent::OpenRequested {
                            url: url.into(),
                            from_user_gesture: true,
                            background: false,
                        },
                        _ if m == "reload" => PageEvent::ActiveTabLoad(None),
                        _ => return,
                    }
                }
                WebviewEvent::Message(_) => return,
            };
            page.update(cx, |_, cx| cx.emit(event));
        })
        .detach();
        page
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
        self.open(profile, url, None, window, cx)
    }

    fn create_popup(
        &self,
        profile: ProfileId,
        url: &str,
        tab_url: &str,
        tab_title: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<CefPage> {
        self.open(profile, url, Some(tabs_shim(tab_url, tab_title)), window, cx)
    }

    fn ephemeral_profile(&self) -> ProfileId {
        let id = self.next_ephemeral.get();
        self.next_ephemeral.set(id - 1);
        ProfileId(id)
    }

    fn extension_details(&self, profile: ProfileId, cx: &mut App, done: Callback<Option<Vec<ExtensionInfo>>>) {
        if is_ephemeral(profile) {
            return done(Some(Vec::new()), cx);
        }
        self.on_extensions_page(profile, EXTENSION_DETAILS.into(), cx, Box::new(|value, cx| {
            done(value.map(|v| parse_extension_details(&v)), cx)
        }));
    }

    fn show_chrome_page(&self, profile: ProfileId, sheet: Option<ChromeSheet>, window: &mut Window) -> bool {
        let mut pages = self.chrome_pages.borrow_mut();
        for (id, ChromeSheetPage { page, .. }) in pages.iter() {
            if sheet.is_none() || *id != profile {
                page.hide();
            }
        }
        let Some(sheet) = sheet else { return true };
        let entry = match pages.entry(profile) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(slot) => {
                let script = skin_loader(&sheet.css);
                let Some(page) = gpui_cef::ChromePage::open(self.profile(profile).as_ref(), &sheet.url, Some(script)) else {
                    return false;
                };
                slot.insert(ChromeSheetPage { page, url: sheet.url.clone(), css: sheet.css.clone(), dark: None })
            }
        };
        let ChromeSheetPage { page, url, css, dark } = entry;
        if *dark != Some(sheet.dark) && page.set_dark(sheet.dark) {
            *dark = Some(sheet.dark);
        }
        if *url != sheet.url {
            page.load_url(&sheet.url);
            *url = sheet.url.clone();
        }
        if *css != sheet.css {
            let css_js = serde_json::to_string(&sheet.css).unwrap_or_default();
            page.eval(&format!("window.__baaSkin && __baaSkin({css_js})"), |_| {});
            *css = sheet.css.clone();
        }
        page.show_over(window, sheet.bounds, sheet.corner_radius)
    }

    fn change_extension(&self, profile: ProfileId, id: &str, change: ExtensionChange, cx: &mut App, done: Callback<bool>) {
        let id = serde_json::to_string(id).unwrap_or_default();
        let script = match change {
            ExtensionChange::Enable(on) => format!("chrome.management.setEnabled({id}, {on})"),
            ExtensionChange::Remove => format!("chrome.management.uninstall({id}, {{ showConfirmDialog: false }})"),
            ExtensionChange::SiteAccess(access) => {
                let access = match access {
                    SiteAccess::OnClick => "ON_CLICK",
                    SiteAccess::OnSpecificSites => "ON_SPECIFIC_SITES",
                    SiteAccess::OnAllSites => "ON_ALL_SITES",
                };
                format!("chrome.developerPrivate.updateExtensionConfiguration({{ extensionId: {id}, hostAccess: '{access}' }})")
            }
        };
        self.on_extensions_page(profile, script, cx, Box::new(|value, cx| done(value.is_some(), cx)));
    }

    fn fetch(&self, profile: ProfileId, url: &str, done: Box<dyn FnOnce(Option<String>)>) {
        gpui_cef::fetch(self.profile(profile).as_ref(), url, done)
    }

    fn downloads(&self) -> Entity<Downloads> {
        self.downloads.clone()
    }

    fn chrome_tabs(&self) -> Entity<ChromeTabs> {
        self.chrome_tabs.clone()
    }

    fn cancel_download(&self, id: u32) {
        gpui_cef::cancel_download(id)
    }

    fn site_permission(&self, profile: ProfileId, url: &str, kind: Permission) -> PermissionState {
        self.profile(profile)
            .map(|p| p.permission(url, kind))
            .unwrap_or(PermissionState::Ask)
    }

    fn set_site_permission(&self, profile: ProfileId, url: &str, kind: Permission, state: PermissionState) {
        if let Some(p) = self.profile(profile) {
            p.set_permission(url, kind, state)
        }
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
    fn answer_permission(&self, id: u64, allow: bool) {
        CefPage::answer_permission(self, id, allow)
    }
    fn answer_dialog(&self, ok: bool, text: &str) {
        CefPage::answer_dialog(self, ok, text)
    }
    fn print(&self) {
        CefPage::print(self)
    }
    fn set_muted(&self, muted: bool) {
        CefPage::set_muted(self, muted)
    }
    fn is_secure(&self) -> bool {
        CefPage::is_secure(self)
    }
    fn eval_with_gesture(&self, script: &str) {
        CefPage::eval_with_gesture(self, script)
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

/// Chrome's settings in the sheet: the page, its address, skin, and the colour
/// scheme it was last told (`None` before it was up).
struct ChromeSheetPage {
    page: gpui_cef::ChromePage,
    url: String,
    css: String,
    dark: Option<bool>,
}

/// Runs before each of Chrome's pages: puts baa's skin over Chrome's styles.
/// A skin changed since (`__baaSkin`) is kept in the page's storage under this
/// run's token, so the next page in the sheet starts in it and a later run
/// starts from `css`.
fn skin_loader(css: &str) -> String {
    let css = serde_json::to_string(css).unwrap_or_default();
    let token = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!(
        r#"(() => {{
  const token = '{token}';
  const apply = css => {{
    // Before the document has a root, wait for it.
    const root = document.head || document.documentElement;
    if (!root) {{
      document.addEventListener('DOMContentLoaded', () => apply(css), {{ once: true }});
      return;
    }}
    let s = document.getElementById('baa-skin');
    if (!s) {{
      s = document.createElement('style');
      s.id = 'baa-skin';
      root.appendChild(s);
    }}
    s.textContent = css;
  }};
  window.__baaSkin = css => {{
    try {{ localStorage.setItem('baa-skin', JSON.stringify({{ token, css }})); }} catch {{}}
    apply(css);
  }};
  let saved = null;
  try {{ saved = JSON.parse(localStorage.getItem('baa-skin')); }} catch {{}}
  apply(saved && saved.token === token ? saved.css : {css});
}})();"#
    )
}

/// One script for the hidden extensions page, retried while the page loads.
struct ExtensionsPage {
    pages: Rc<RefCell<HashMap<ProfileId, gpui_cef::ChromePage>>>,
    id: ProfileId,
    profile: Option<gpui_cef::Profile>,
    script: String,
}

impl ExtensionsPage {
    fn run(self, app: gpui::AsyncApp, tries: u32, done: Callback<Option<serde_json::Value>>) {
        let page = {
            let mut pages = self.pages.borrow_mut();
            match pages.get(&self.id) {
                Some(page) => Some(page.clone()),
                None => gpui_cef::ChromePage::open(self.profile.as_ref(), "chrome://extensions/", None).inspect(|page| {
                    pages.insert(self.id, page.clone());
                }),
            }
        };
        let Some(page) = page else {
            return self.again(app, tries, done);
        };
        let script = self.script.clone();
        page.eval(&script, move |result| {
            let result: Option<serde_json::Value> = result.and_then(|r| serde_json::from_str(&r).ok());
            // A throw (the page still loading, or the call failing) comes back as exceptionDetails.
            let value = result
                .filter(|r| r.get("exceptionDetails").is_none())
                .and_then(|r| r["result"]["value"].as_str().and_then(|v| serde_json::from_str(v).ok()));
            // ponytail: a call that fails for good is retried like a page still loading.
            match value {
                Some(value) => {
                    app.spawn(async move |cx| cx.update(|cx| done(Some(value), cx))).detach();
                }
                None => self.again(app, tries, done),
            }
        });
    }

    fn again(self, app: gpui::AsyncApp, tries: u32, done: Callback<Option<serde_json::Value>>) {
        app.clone()
            .spawn(async move |cx| {
                if tries == 0 {
                    log::error!("the extensions page did not answer");
                    return cx.update(|cx| done(None, cx));
                }
                cx.background_executor().timer(std::time::Duration::from_millis(250)).await;
                self.run(cx.clone(), tries - 1, done);
            })
            .detach();
    }
}

/// Every extension, as chrome://extensions lists them.
const EXTENSION_DETAILS: &str = "chrome.developerPrivate.getExtensionsInfo({ includeDisabled: true, includeTerminated: true })";

fn parse_extension_details(list: &serde_json::Value) -> Vec<ExtensionInfo> {
    let text = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
    let mut out: Vec<ExtensionInfo> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["location"] != "COMPONENT" && e["type"] != "THEME")
        .map(|e| {
            let permissions = &e["permissions"];
            ExtensionInfo {
                id: text(&e["id"]),
                name: text(&e["name"]),
                description: text(&e["description"]),
                version: text(&e["version"]),
                enabled: e["state"] == "ENABLED",
                permissions: permissions["simplePermissions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|p| text(&p["message"]))
                    .filter(|m| !m.is_empty())
                    .collect(),
                site_access: match permissions["runtimeHostPermissions"]["hostAccess"].as_str() {
                    Some("ON_CLICK") => Some(SiteAccess::OnClick),
                    Some("ON_SPECIFIC_SITES") => Some(SiteAccess::OnSpecificSites),
                    Some("ON_ALL_SITES") => Some(SiteAccess::OnAllSites),
                    _ => None,
                },
                options: e["optionsPage"]["url"].as_str().filter(|u| !u.is_empty()).map(String::from),
            }
        })
        .collect();
    out.sort_by_key(|e| e.name.to_lowercase());
    out
}

/// Extensions see only Chrome-style tabs and baa's pages are Alloy-style, so
/// a popup asking for "the active tab" gets nothing. This answers with the
/// tab it was opened over, and passes its reloads and navigations of that tab
/// back as messages.
// ponytail: query, get, reload, update, create and openOptionsPage only; tabs.sendMessage and
// scripting into the tab still fail. Ceiling: Chrome-style tabs, which CEF
// cannot put in a native child view.
fn tabs_shim(url: &str, title: &str) -> String {
    let tab = serde_json::json!({
        "id": -1, "index": 0, "windowId": -1, "active": true, "highlighted": true,
        "pinned": false, "incognito": false, "audible": false, "discarded": false,
        "autoDiscardable": true, "groupId": -1, "status": "complete", "url": url, "title": title,
    });
    format!(
        r#"(() => {{
  const tabs = globalThis.chrome && chrome.tabs;
  if (!tabs) return;
  const tab = {tab};
  const post = m => console.debug({prefix:?} + m);
  const done = (cb, v) => typeof cb === 'function' ? void cb(v) : Promise.resolve(v);
  const real = {{ query: tabs.query, get: tabs.get, reload: tabs.reload, update: tabs.update }};
  tabs.query = function (info, cb) {{
    return info && info.active ? done(cb, [{{ ...tab }}]) : real.query.apply(tabs, arguments);
  }};
  tabs.get = function (id, cb) {{
    return id === tab.id ? done(cb, {{ ...tab }}) : real.get.apply(tabs, arguments);
  }};
  tabs.reload = function (id, props, cb) {{
    if (typeof id === 'number' && id !== tab.id) return real.reload.apply(tabs, arguments);
    post('reload');
    return done([...arguments].find(a => typeof a === 'function'));
  }};
  tabs.update = function (id, props, cb) {{
    if (typeof id !== 'number') [id, props, cb] = [tab.id, id, props];
    if (id !== tab.id) return real.update.apply(tabs, arguments);
    if (props && props.url) post('load:' + new URL(props.url, location.href).href);
    return done(cb, {{ ...tab }});
  }};
  // Chrome would open these in a window of its own; they belong in baa's tabs.
  tabs.create = function (props, cb) {{
    if (props && props.url) post('open:' + new URL(props.url, location.href).href);
    return done(cb, {{ ...tab }});
  }};
  chrome.runtime.openOptionsPage = function (cb) {{
    const m = chrome.runtime.getManifest();
    const page = m.options_page || (m.options_ui && m.options_ui.page);
    if (page) post('open:' + chrome.runtime.getURL(page));
    return done(cb);
  }};
}})();"#,
        prefix = gpui_cef::MESSAGE_PREFIX,
    )
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
            let options = manifest
                .get("options_page")
                .or_else(|| manifest.get("options_ui").and_then(|o| o.get("page")))
                .and_then(|p| p.as_str())
                .map(|p| format!("chrome-extension://{id}/{}", p.trim_start_matches('/')));
            let name = manifest.get("name").and_then(|n| n.as_str()).unwrap_or(&id).to_string();
            let name = localized(&dir, &manifest, &name).unwrap_or(name);
            Some(Extension { id, name, icon, popup, options })
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

/// Password managers (1Password, Bitwarden, ...) talk to their browser
/// extensions through a native messaging host they register with Chrome and
/// its relatives. Chromium reads `<user data>/NativeMessagingHosts`, so those
/// registrations are copied there and the extensions work as in Chrome.
/// (Apple's iCloud Passwords host cannot: macOS only lets Chrome and Edge
/// start it.)
fn share_native_messaging_hosts(root: &Path) {
    #[cfg(target_os = "macos")]
    let sources: Vec<PathBuf> = {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let support = home.join("Library/Application Support");
        [
            "Google/Chrome",
            "Chromium",
            "Microsoft Edge",
            "BraveSoftware/Brave-Browser",
            "Vivaldi",
        ]
        .iter()
        .map(|b| support.join(b).join("NativeMessagingHosts"))
        .chain(
            [
                "/Library/Google/Chrome/NativeMessagingHosts",
                "/Library/Application Support/Chromium/NativeMessagingHosts",
                "/Library/Microsoft/Edge/NativeMessagingHosts",
            ]
            .map(PathBuf::from),
        )
        .collect()
    };
    #[cfg(not(target_os = "macos"))]
    let sources: Vec<PathBuf> = Vec::new();
    let dest = root.join("NativeMessagingHosts");
    // Refreshed every launch (an update can move a host); the first browser listing one wins.
    let mut copied = std::collections::HashSet::new();
    for source in sources {
        let Ok(entries) = std::fs::read_dir(source) else { continue };
        let _ = std::fs::create_dir_all(&dest);
        for entry in entries.flatten() {
            let name = entry.file_name();
            if entry.path().extension().is_some_and(|e| e == "json") && copied.insert(name.clone()) {
                let _ = std::fs::copy(entry.path(), dest.join(name));
            }
        }
    }
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
    fn reads_name_icon_popup_and_options_from_the_newest_version() {
        let root = std::env::temp_dir().join(format!("baa-ext-{}", std::process::id()));
        let dir = root.join("Extensions/abc/1.0_0");
        std::fs::create_dir_all(dir.join("_locales/en")).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"__MSG_extName__","default_locale":"en","action":{"default_icon":{"16":"i16.png","32":"i32.png"},"default_popup":"popup.html"},"options_ui":{"page":"/opts.html"}}"#,
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
                options: Some("chrome-extension://abc/opts.html".into()),
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
        share_native_messaging_hosts(&root);

        assert!(keep.exists());
        assert!(!gone.exists());
        assert!(!root.join(PENDING_DELETES).exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn downloads_update_in_place() {
        let download = |id, received| crate::Download {
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
