//! A tab shown by a WKWebView instead of Chrome, for streaming sites the user allowed
//! (`WebPrefs::uses_webkit`): WebKit plays FairPlay, which needs no licence from the app.
//!
//! WebKit's own checks stay (TLS, mixed content, popups). Stricter than Chrome here, since
//! baa draws no WebKit prompts: no extensions, downloads, file pickers or JS dialogs
//! (WebKit answers them as Cancel), camera and microphone always denied, and no scheme
//! but web ones loads, so a page cannot open an app.

use std::{cell::RefCell, collections::HashMap, ffi::CStr};

use block2::Block;
use objc2::{
    define_class, msg_send,
    rc::{Allocated, Retained},
    runtime::{AnyObject, Bool, NSObject},
    sel, class, ClassType,
};

use super::{native::{CGPoint, CGRect, CGSize}, profiles::is_ephemeral, Snapshot};
use crate::{ClearData, ProfileId};

#[link(name = "WebKit", kind = "framework")]
extern "C" {}

const CANCEL: isize = 0;
const ALLOW: isize = 1;
const PERMISSION_DENY: isize = 2;
const NAVIGATION_LINK_ACTIVATED: isize = 0;

/// What a WebKit page may load in any frame. Anything else (mailto:, file:, app schemes) is
/// cancelled: baa has no external-protocol prompt for WebKit.
pub(super) fn loads(scheme: &str) -> bool {
    matches!(scheme.to_ascii_lowercase().as_str(), "https" | "http" | "about" | "blob" | "data")
}

#[derive(Default)]
pub(super) struct Engine {
    prefs: RefCell<baa_model::WebPrefs>,
    stores: RefCell<HashMap<ProfileId, Retained<AnyObject>>>,
}

impl Engine {
    pub(super) fn set_prefs(&self, web: &baa_model::WebPrefs) {
        *self.prefs.borrow_mut() = web.clone();
    }

    pub(super) fn uses(&self, url: &str) -> bool {
        self.prefs.borrow().uses_webkit(url)
    }

    /// `profile`'s cookies and storage, apart from every other Space's and from Chrome's.
    /// In memory for an incognito Space, and when cookies are cleared on quit. `None` before
    /// macOS 14, which has no per-identifier stores: the page stays in Chrome.
    pub(super) fn store(&self, profile: ProfileId) -> Option<Retained<AnyObject>> {
        if let Some(store) = self.stores.borrow().get(&profile) {
            return Some(store.clone());
        }
        let store: Option<Retained<AnyObject>> = if is_ephemeral(profile) || self.prefs.borrow().clear_cookies_on_quit {
            unsafe { msg_send![class!(WKWebsiteDataStore), nonPersistentDataStore] }
        } else {
            let id = uuid(profile)?;
            unsafe { msg_send![class!(WKWebsiteDataStore), dataStoreForIdentifier: &*id] }
        };
        let store = store?;
        self.stores.borrow_mut().insert(profile, store.clone());
        Some(store)
    }

    /// Whether WebKit may hold data of `profile`. Asking WebKit itself would start its
    /// processes for people who never use it.
    // ponytail: data a Space stored before the switch was turned off stays until it is on
    // again. Upgrade: `fetchAllDataStoreIdentifiers:` once at startup.
    fn may_have_data(&self, profile: ProfileId) -> bool {
        self.prefs.borrow().webkit_drm || self.stores.borrow().contains_key(&profile)
    }

    /// The Space is gone and its pages are closed: its WebKit data goes too.
    pub(super) fn forget(&self, profile: ProfileId) {
        let had = self.may_have_data(profile);
        self.stores.borrow_mut().remove(&profile);
        if !had || is_ephemeral(profile) {
            return;
        }
        if let Some(id) = uuid(profile) {
            let done = block2::RcBlock::new(|_: *mut AnyObject| {});
            unsafe {
                let _: () = msg_send![class!(WKWebsiteDataStore), removeDataStoreForIdentifier: &*id, completionHandler: &*done];
            }
        }
    }

    /// Clear browsing data reaches WebKit's copy too. History and downloads are baa's.
    pub(super) fn clear(&self, profile: ProfileId, since: Option<std::time::SystemTime>, what: ClearData) {
        if !(what.site_data || what.cache) || !self.may_have_data(profile) {
            return;
        }
        let Some(store) = self.store(profile) else { return };
        let since = since.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0., |d| d.as_secs_f64());
        let done = block2::RcBlock::new(|| {});
        unsafe {
            let types: Retained<AnyObject> = if what.site_data {
                msg_send![class!(WKWebsiteDataStore), allWebsiteDataTypes]
            } else {
                let names = ["WKWebsiteDataTypeDiskCache", "WKWebsiteDataTypeMemoryCache", "WKWebsiteDataTypeFetchCache"].map(ns_string);
                let refs: Vec<*const AnyObject> = names.iter().map(Retained::as_ptr).collect();
                msg_send![class!(NSSet), setWithObjects: refs.as_ptr(), count: refs.len()]
            };
            let date: Retained<AnyObject> = msg_send![class!(NSDate), dateWithTimeIntervalSince1970: since];
            let _: () = msg_send![&*store, removeDataOfTypes: &*types, modifiedSince: &*date, completionHandler: &*done];
        }
    }
}

/// The same identifier for a Space on every launch.
fn uuid(profile: ProfileId) -> Option<Retained<AnyObject>> {
    let has: Bool = unsafe { msg_send![class!(WKWebsiteDataStore), respondsToSelector: sel!(dataStoreForIdentifier:)] };
    if !has.as_bool() {
        return None;
    }
    let mut bytes = *b"baa-webkit\0\0\0\0\0\0";
    bytes[8..].copy_from_slice(&profile.0.to_be_bytes());
    unsafe {
        let id: Allocated<AnyObject> = msg_send![class!(NSUUID), alloc];
        msg_send![id, initWithUUIDBytes: bytes.as_ptr()]
    }
}

pub(super) struct View {
    web: Retained<AnyObject>,
}

impl View {
    pub(super) fn new(store: &AnyObject) -> Self {
        unsafe {
            let config: Retained<AnyObject> = msg_send![class!(WKWebViewConfiguration), new];
            let _: () = msg_send![&*config, setWebsiteDataStore: store];
            let prefs: Retained<AnyObject> = msg_send![&*config, preferences];
            let fullscreen: Bool = msg_send![&*prefs, respondsToSelector: sel!(setElementFullscreenEnabled:)];
            if fullscreen.as_bool() {
                let _: () = msg_send![&*prefs, setElementFullscreenEnabled: Bool::YES];
            }
            // Streaming sites offer FairPlay only to Safari; WebKit's own tokens stay as they are.
            let _: () = msg_send![&*config, setApplicationNameForUserAgent: &*ns_string(&safari_tokens())];
            let web: Allocated<AnyObject> = msg_send![class!(WKWebView), alloc];
            let frame = CGRect { origin: CGPoint { x: 0., y: 0. }, size: CGSize { width: 1., height: 1. } };
            let web: Retained<AnyObject> = msg_send![web, initWithFrame: frame, configuration: &*config];
            DELEGATE.with(|delegate| {
                let _: () = msg_send![&*web, setNavigationDelegate: &**delegate];
                let _: () = msg_send![&*web, setUIDelegate: &**delegate];
            });
            Self { web }
        }
    }

    pub(super) fn as_ptr(&self) -> *mut AnyObject {
        Retained::as_ptr(&self.web) as *mut AnyObject
    }

    /// `url` is one the user typed or picked (`PageHandle::load_url`, `create_page`).
    pub(super) fn load(&self, url: &str) {
        unsafe {
            let url: Option<Retained<AnyObject>> = msg_send![class!(NSURL), URLWithString: &*ns_string(url)];
            let Some(url) = url else { return };
            let request: Retained<AnyObject> = msg_send![class!(NSURLRequest), requestWithURL: &*url];
            let _: *mut AnyObject = msg_send![&*self.web, loadRequest: &*request];
        }
    }

    pub(super) fn snapshot(&self) -> Snapshot {
        unsafe {
            let url: Option<Retained<AnyObject>> = msg_send![&*self.web, URL];
            let url = url.and_then(|u| -> Option<Retained<AnyObject>> { msg_send![&*u, absoluteString] });
            let title: Option<Retained<AnyObject>> = msg_send![&*self.web, title];
            let flag = |b: Bool| b.as_bool();
            Snapshot {
                url: url.as_deref().map(string).unwrap_or_default(),
                title: title.as_deref().map(string).unwrap_or_default(),
                view: self.as_ptr(),
                loading: flag(msg_send![&*self.web, isLoading]),
                can_go_back: flag(msg_send![&*self.web, canGoBack]),
                can_go_forward: flag(msg_send![&*self.web, canGoForward]),
                secure: flag(msg_send![&*self.web, hasOnlySecureContent]),
                // ponytail: no committed origin, so nothing that needs one (the agent) acts on
                // a WebKit page. WebKit's URL changes before a navigation commits.
                ..Snapshot::default()
            }
        }
    }

    pub(super) fn reload(&self, bypass_cache: bool) {
        unsafe {
            let _: *mut AnyObject =
                if bypass_cache { msg_send![&*self.web, reloadFromOrigin] } else { msg_send![&*self.web, reload] };
        }
    }

    pub(super) fn stop(&self) {
        unsafe { msg_send![&*self.web, stopLoading] }
    }

    pub(super) fn go_to_offset(&self, offset: i32) {
        unsafe {
            let list: Retained<AnyObject> = msg_send![&*self.web, backForwardList];
            let item: Option<Retained<AnyObject>> = msg_send![&*list, itemAtIndex: offset as isize];
            if let Some(item) = item {
                let _: *mut AnyObject = msg_send![&*self.web, goToBackForwardListItem: &*item];
            }
        }
    }

    /// As (title, url), oldest first, and the current entry's index.
    pub(super) fn history(&self) -> (Vec<(String, String)>, usize) {
        unsafe {
            let list: Retained<AnyObject> = msg_send![&*self.web, backForwardList];
            let back: Retained<AnyObject> = msg_send![&*list, backList];
            let current: Option<Retained<AnyObject>> = msg_send![&*list, currentItem];
            let forward: Retained<AnyObject> = msg_send![&*list, forwardList];
            let mut entries = items(&back);
            let at = entries.len();
            entries.extend(current.iter().map(|item| entry(item)));
            entries.extend(items(&forward));
            (entries, at)
        }
    }

    pub(super) fn eval(&self, script: &str) {
        unsafe {
            let none: *const Block<dyn Fn(*mut AnyObject, *mut AnyObject)> = std::ptr::null();
            let _: () = msg_send![&*self.web, evaluateJavaScript: &*ns_string(script), completionHandler: none];
        }
    }

    pub(super) fn set_zoom(&self, level: f64) {
        unsafe { msg_send![&*self.web, setPageZoom: level] }
    }

    pub(super) fn focus(&self) {
        super::native::make_first_responder(self.as_ptr());
    }
}

impl Drop for View {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![&*self.web, stopLoading];
            let none: *const AnyObject = std::ptr::null();
            let _: () = msg_send![&*self.web, setNavigationDelegate: none];
            let _: () = msg_send![&*self.web, setUIDelegate: none];
            let _: () = msg_send![&*self.web, removeFromSuperview];
        }
    }
}

unsafe fn items(array: &AnyObject) -> Vec<(String, String)> {
    let count: usize = msg_send![array, count];
    (0..count)
        .map(|i| {
            let item: Retained<AnyObject> = msg_send![array, objectAtIndex: i];
            entry(&item)
        })
        .collect()
}

unsafe fn entry(item: &AnyObject) -> (String, String) {
    let title: Option<Retained<AnyObject>> = msg_send![item, title];
    let url: Retained<AnyObject> = msg_send![item, URL];
    let url: Retained<AnyObject> = msg_send![&*url, absoluteString];
    (title.as_deref().map(string).unwrap_or_default(), string(&url))
}

/// "Version/x Safari/605.1.15" with the installed Safari's version.
fn safari_tokens() -> String {
    let version = unsafe {
        let bundle: Option<Retained<AnyObject>> = msg_send![class!(NSBundle), bundleWithPath: &*ns_string("/Applications/Safari.app")];
        bundle.and_then(|bundle| -> Option<Retained<AnyObject>> {
            msg_send![&*bundle, objectForInfoDictionaryKey: &*ns_string("CFBundleShortVersionString")]
        })
    };
    // ponytail: a fixed fallback for a Mac without Safari.app; sites may call it outdated.
    let version = version.as_deref().map(string).unwrap_or_else(|| "26.0".into());
    format!("Version/{version} Safari/605.1.15")
}

fn ns_string(text: &str) -> Retained<AnyObject> {
    unsafe {
        let s: Allocated<AnyObject> = msg_send![class!(NSString), alloc];
        msg_send![s, initWithBytes: text.as_ptr().cast::<std::ffi::c_void>(), length: text.len(), encoding: 4usize]
    }
}

fn string(s: &AnyObject) -> String {
    unsafe {
        let utf8: *const std::ffi::c_char = msg_send![s, UTF8String];
        if utf8.is_null() { String::new() } else { CStr::from_ptr(utf8).to_string_lossy().into_owned() }
    }
}

unsafe fn scheme_of(action: &AnyObject) -> String {
    let request: Retained<AnyObject> = msg_send![action, request];
    let url: Option<Retained<AnyObject>> = msg_send![&*request, URL];
    let scheme: Option<Retained<AnyObject>> = url.and_then(|u| msg_send![&*u, scheme]);
    scheme.as_deref().map(string).unwrap_or_default()
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this class has no Drop.
    #[unsafe(super(NSObject))]
    #[name = "BaaWebKitDelegate"]
    struct Delegate;

    impl Delegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide_action(&self, _web: &AnyObject, action: &AnyObject, decide: &Block<dyn Fn(isize)>) {
            let download: Bool = unsafe {
                let has: Bool = msg_send![action, respondsToSelector: sel!(shouldPerformDownload)];
                if has.as_bool() { msg_send![action, shouldPerformDownload] } else { Bool::NO }
            };
            let ok = !download.as_bool() && loads(&unsafe { scheme_of(action) });
            decide.call((if ok { ALLOW } else { CANCEL },));
        }

        /// What WebKit cannot show would be a download: never.
        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn decide_response(&self, _web: &AnyObject, response: &AnyObject, decide: &Block<dyn Fn(isize)>) {
            let shows: Bool = unsafe { msg_send![response, canShowMIMEType] };
            decide.call((if shows.as_bool() { ALLOW } else { CANCEL },));
        }

        /// No new windows. A link the user clicked with a target opens in this tab, as a plain
        /// link would; scripted popups are dropped.
        #[unsafe(method(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(&self, web: &AnyObject, _config: &AnyObject, action: &AnyObject, _features: &AnyObject) -> *mut AnyObject {
            unsafe {
                let kind: isize = msg_send![action, navigationType];
                if kind == NAVIGATION_LINK_ACTIVATED && matches!(scheme_of(action).as_str(), "https" | "http") {
                    let request: Retained<AnyObject> = msg_send![action, request];
                    let _: *mut AnyObject = msg_send![web, loadRequest: &*request];
                }
            }
            std::ptr::null_mut()
        }

        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn media_capture(&self, _web: &AnyObject, _origin: &AnyObject, _frame: &AnyObject, _kind: isize, decide: &Block<dyn Fn(isize)>) {
            decide.call((PERMISSION_DENY,));
        }
    }
);

thread_local! {
    /// WebKit holds its delegates weakly; this one lives for the process.
    static DELEGATE: Retained<Delegate> = unsafe { msg_send![Delegate::class(), new] };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_schemes_load() {
        for ok in ["https", "HTTP", "about", "blob", "data"] {
            assert!(loads(ok), "{ok}");
        }
        for no in ["", "mailto", "file", "javascript", "zoommtg", "x-apple.systempreferences", "ftp", "chrome-extension"] {
            assert!(!loads(no), "{no}");
        }
    }

    #[test]
    fn each_space_has_its_own_lasting_identifier() {
        let Some(a) = uuid(ProfileId(1)) else { return };
        let string_of = |id: &AnyObject| -> String {
            let s: Retained<AnyObject> = unsafe { msg_send![id, UUIDString] };
            string(&s)
        };
        assert_eq!(string_of(&a), string_of(&uuid(ProfileId(1)).unwrap()));
        assert_ne!(string_of(&a), string_of(&uuid(ProfileId(2)).unwrap()));
    }
}
