//! Chrome's own pages (`baa://…`) in normal tabs: the url mapping, baa's skin
//! script, and the colour scheme a page gets.

use super::{
    c_string, dispatch::Dispatch, ffi::Profile,
};

/// Helium's own scheme is not rewritten by a plain load, so loads go out as `chrome://`.
pub fn to_chrome(url: &str) -> String {
    url.replacen("baa://", "chrome://", 1)
}

/// The UI sees only `baa://`.
pub fn to_baa(url: String) -> String {
    if url.starts_with("chrome://") { url.replacen("chrome://", "baa://", 1) } else { url }
}

pub fn is_webui(url: &str) -> bool {
    url.starts_with("baa://") || url.starts_with("chrome://")
}

/// `baa_tab_set_color_scheme` value for a Chrome page: the skin decides.
fn webui_scheme(dark: bool) -> i32 {
    if dark { 2 } else { 1 }
}

/// Runs on every document of the tab; websites pay for the first line only.
pub fn loader(css: &str) -> String {
    let css = serde_json::to_string(css).unwrap_or_default();
    let token = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!(
        r#"(() => {{
  if (location.protocol !== 'chrome:' && location.protocol !== 'baa:') return;
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

impl Dispatch {
    /// The scheme for a page of `profile`: Chrome's own follow the skin, websites the user's choice.
    pub(super) fn scheme_for(&self, profile: *mut Profile, webui: bool) -> i32 {
        if !webui {
            return self.page_scheme.get();
        }
        webui_scheme(self.skins.borrow().get(&profile).is_some_and(|(_, dark)| *dark))
    }

    /// The init script and scheme of a tab baa takes over. A fresh Chrome page
    /// is created blank so the script is in before its first document; one
    /// Chrome made (`adopted`) is on its way already and gets the skin once.
    pub(super) fn prepare_tab(&self, tab: *mut super::ffi::Tab, profile: *mut Profile, url: &str, adopted: bool) {
        let webui = is_webui(url);
        let css = self.skins.borrow().get(&profile).map(|(css, _)| css.clone()).unwrap_or_default();
        let script = loader(&css);
        (self.core.tab_set_init_script)(tab, c_string(&script).as_ptr());
        (self.core.tab_set_color_scheme)(tab, self.scheme_for(profile, webui));
        if !webui {
            return;
        }
        if adopted {
            (self.core.tab_eval)(tab, c_string(&script).as_ptr(), false);
        } else {
            (self.core.tab_load_url)(tab, c_string(&to_chrome(url)).as_ptr());
        }
    }

    /// New css for `profile`'s Chrome pages, live ones and later ones.
    pub(super) fn set_skin(&self, profile: *mut Profile, css: String, dark: bool) {
        if self.skins.borrow_mut().insert(profile, (css.clone(), dark)).is_some_and(|old| old == (css.clone(), dark)) {
            return;
        }
        let script = loader(&css);
        let apply = format!("window.__baaSkin && __baaSkin({})", serde_json::to_string(&css).unwrap_or_default());
        for live in self.tabs.borrow().values() {
            let shared = &live.shared;
            if shared.profile != profile || shared.kind != super::PageKind::Tab {
                continue;
            }
            shared.run(|core, tab| {
                (core.tab_set_init_script)(tab, c_string(&script).as_ptr());
                if shared.webui.get() {
                    (core.tab_eval)(tab, c_string(&apply).as_ptr(), false);
                    (core.tab_set_color_scheme)(tab, webui_scheme(dark));
                }
            });
        }
    }

    /// What websites see as prefers-color-scheme: 0 system, 1 light, 2 dark.
    pub(super) fn set_page_scheme(&self, scheme: i32) {
        if self.page_scheme.replace(scheme) == scheme {
            return;
        }
        for live in self.tabs.borrow().values() {
            let shared = &live.shared;
            if shared.kind == super::PageKind::Tab && !shared.webui.get() {
                shared.run(|core, tab| (core.tab_set_color_scheme)(tab, scheme));
            }
        }
    }
}
