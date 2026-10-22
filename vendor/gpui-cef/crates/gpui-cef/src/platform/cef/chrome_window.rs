//! A native Chrome-style window on a profile. Alloy-style OSR pages run
//! extensions but cannot host Chrome's own UI (Web Store install prompt,
//! extension popups, chrome://extensions), so those open here.

use cef::*;

use super::Profile;

wrap_client! {
    struct PlainClient;
    impl Client {}
}

wrap_browser_view_delegate! {
    struct ChromeViewDelegate {}
    impl ViewDelegate {}
    impl BrowserViewDelegate {
        fn browser_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::CHROME
        }
        fn chrome_toolbar_type(&self, _browser_view: Option<&mut BrowserView>) -> ChromeToolbarType {
            ChromeToolbarType::NORMAL
        }
    }
}

wrap_window_delegate! {
    struct ChromeWindowDelegate {
        view: BrowserView,
    }
    impl ViewDelegate {}
    impl PanelDelegate {}
    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let Some(window) = window else { return };
            window.add_child_view(Some(&mut View::from(&self.view)));
            window.center_window(Some(&Size { width: 1100, height: 800 }));
            window.show();
        }
        fn window_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::CHROME
        }
        fn can_close(&self, _window: Option<&mut Window>) -> ::std::os::raw::c_int {
            1
        }
    }
}

/// Opens `url` in a separate Chrome-style window sharing `profile`'s storage
/// and extensions. Call after [`crate::Runtime::start`]. Returns false while
/// the profile's context is still initialising; retry shortly after.
pub fn open_chrome_window(profile: Option<&Profile>, url: &str) -> bool {
    let mut client = PlainClient::new();
    let mut delegate = ChromeViewDelegate::new();
    let mut context = profile.map(|p| p.context.clone());
    let Some(view) = browser_view_create(
        Some(&mut client),
        Some(&url.into()),
        Some(&BrowserSettings::default()),
        None,
        context.as_mut(),
        Some(&mut delegate),
    ) else {
        return false;
    };
    let mut window = ChromeWindowDelegate::new(view);
    window_create_top_level(Some(&mut window)).is_some()
}
