//! Chrome's own page (`chrome://settings`) over part of the window, in baa's
//! skin. A real tab of the profile (WebUI needs its Browser), shown as an
//! overlay on the UI.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use objc2::runtime::AnyObject;
use serde_json::to_string;

use super::{c_string, ffi::TabState, native, Dispatch, PageKind, Shared, Snapshot};
use crate::{cef_host::skin_loader, ChromeSheet};

pub struct Sheet {
    dispatch: &'static Dispatch,
    shared: Rc<Shared>,
    /// The gpui view it sits over now.
    over: Cell<*mut AnyObject>,
    url: RefCell<String>,
    css: RefCell<String>,
    dark: Cell<bool>,
    shown: Cell<bool>,
}

/// Helium's own scheme is not rewritten by a plain load.
fn chrome_url(url: &str) -> String {
    url.replacen("baa://", "chrome://", 1)
}

impl Sheet {
    /// `None` when the profile is gone.
    pub fn open(dispatch: &'static Dispatch, profile: *mut super::ffi::Profile, sheet: &ChromeSheet) -> Option<Self> {
        if profile.is_null() {
            return None;
        }
        let core = dispatch.core;
        // Blank first: the skin script has to be in before the first real document.
        let tab = (core.tab_create)(profile, c"about:blank".as_ptr());
        if tab.is_null() {
            return None;
        }
        let mut raw: TabState = unsafe { std::mem::zeroed() };
        let view = if (core.tab_get_state)(tab, &mut raw) { Snapshot::from_c(&raw).view } else { std::ptr::null_mut() };
        let container = Rc::new(native::Container::new(std::ptr::null_mut(), view, sheet.corner_radius));
        let shared = Rc::new(Shared {
            core,
            kind: PageKind::Tab,
            profile,
            container,
            tab: Cell::new(tab),
            view: Cell::new(view),
            dialog: Cell::new(None),
            drag: Cell::new(None),
            inside: Cell::new(false),
        });
        dispatch.bare.borrow_mut().insert(tab, shared.clone());
        (core.tab_set_init_script)(tab, c_string(&skin_loader(&sheet.css)).as_ptr());
        (core.tab_set_color_scheme)(tab, scheme(sheet.dark));
        (core.tab_load_url)(tab, c_string(&chrome_url(&sheet.url)).as_ptr());
        Some(Self {
            dispatch,
            shared,
            over: Cell::new(std::ptr::null_mut()),
            url: RefCell::new(sheet.url.clone()),
            css: RefCell::new(sheet.css.clone()),
            dark: Cell::new(sheet.dark),
            shown: Cell::new(false),
        })
    }

    /// Puts the page over `sheet.bounds` of `window`, after applying what changed.
    pub fn show(&mut self, sheet: &ChromeSheet, window: &mut gpui::Window) -> bool {
        if self.dark.replace(sheet.dark) != sheet.dark {
            self.shared.run(|core, tab| (core.tab_set_color_scheme)(tab, scheme(sheet.dark)));
        }
        if *self.url.borrow() != sheet.url {
            self.url.replace(sheet.url.clone());
            self.shared.run(|core, tab| (core.tab_load_url)(tab, c_string(&chrome_url(&sheet.url)).as_ptr()));
        }
        if *self.css.borrow() != sheet.css {
            self.css.replace(sheet.css.clone());
            let css = to_string(&sheet.css).unwrap_or_default();
            let script = format!("window.__baaSkin && __baaSkin({css})");
            self.shared.run(|core, tab| {
                (core.tab_eval)(tab, c_string(&script).as_ptr(), false);
                // Pages the sheet opens next start in the new skin too.
                (core.tab_set_init_script)(tab, c_string(&skin_loader(&sheet.css)).as_ptr());
            });
        }
        let gpui_view = native::gpui_view(window);
        let container = &self.shared.container;
        // A view pointer can be reused by another window: the superview says if it is still ours.
        if self.over.replace(gpui_view) != gpui_view || !container.is_beside(gpui_view) {
            container.move_above(gpui_view);
        }
        container.set_corner_radius(sheet.corner_radius);
        container.set_visible(true, self.shared.view.get());
        if container.place(self.shared.view.get(), sheet.bounds, sheet.bounds) {
            window.request_animation_frame();
        }
        if !self.shown.replace(true) {
            // The page takes the keyboard when the sheet comes up (gpui's own focus
            // cannot be dropped from here: Window::blur needs an App).
            self.shared.run(|core, tab| (core.tab_focus)(tab));
        }
        true
    }

    pub fn hide(&self) {
        // Keys must not go on into a hidden page.
        if self.shown.replace(false) && native::has_first_responder(self.shared.view.get()) {
            native::make_first_responder(self.over.get());
        }
        self.shared.container.set_visible(false, self.shared.view.get());
    }

    /// Chrome closed its tab.
    pub fn is_gone(&self) -> bool {
        self.shared.tab.get().is_null()
    }
}

fn scheme(dark: bool) -> i32 {
    if dark { 2 } else { 1 }
}

impl Drop for Sheet {
    fn drop(&mut self) {
        let tab = self.shared.tab.get();
        self.hide();
        self.shared.release();
        if !tab.is_null() {
            self.dispatch.bare.borrow_mut().remove(&tab);
            (self.dispatch.core.tab_close)(tab);
        }
    }
}
