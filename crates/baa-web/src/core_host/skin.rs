//! Chrome's own pages (`baa://…`) in normal tabs: the url mapping and baa's
//! skin script. Colour schemes are the engine's job (`set_color_schemes`).

use super::{
    c_string,
    dispatch::Dispatch,
    ffi::{Profile, Tab},
    PageKind,
};

/// Helium's own scheme is not rewritten by a plain load, so loads go out as `chrome://`.
pub fn to_chrome(url: &str) -> String {
    match url.strip_prefix("baa://") {
        Some(rest) => format!("chrome://{rest}"),
        None => url.to_string(),
    }
}

/// The UI sees only `baa://`.
pub fn to_baa(url: String) -> String {
    match url.strip_prefix("chrome://") {
        Some(rest) => format!("baa://{rest}"),
        None => url,
    }
}

pub fn is_webui(url: &str) -> bool {
    url.starts_with("baa://") || url.starts_with("chrome://")
}

/// Runs on every document of the tab; websites pay for the first line only.
/// The stylesheet is in the script itself, so no flash and nothing stored.
pub fn loader(css: &str) -> String {
    let css = serde_json::to_string(css).unwrap_or_default();
    format!(
        r#"(() => {{
  if (location.protocol !== 'chrome:' && location.protocol !== 'baa:') return;
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
  window.__baaSkin = apply;
  apply({css});
}})();"#
    )
}

/// Re-skins a page that is up: only Chrome's own define `__baaSkin`.
fn restyle(css: &str) -> String {
    let css = serde_json::to_string(css).unwrap_or_default();
    format!("if (location.protocol === 'chrome:' || location.protocol === 'baa:') window.__baaSkin && __baaSkin({css})")
}

impl Dispatch {
    /// The init script of a tab baa takes over. A fresh Chrome page is created
    /// blank so the script is in before its first document; one Chrome made
    /// (`adopted`) is on its way already and gets the skin once.
    pub(super) fn prepare_tab(&self, tab: *mut Tab, url: &str, css: &str, adopted: bool) {
        let script = loader(css);
        (self.core.tab_set_init_script)(tab, c_string(&script).as_ptr());
        if !is_webui(url) {
            return;
        }
        if adopted {
            (self.core.tab_eval)(tab, c_string(&script).as_ptr(), false);
        } else {
            (self.core.tab_load_url)(tab, c_string(&to_chrome(url)).as_ptr());
        }
    }

    /// New css for the tabs of `profile`, for their next documents and the one they show.
    pub(super) fn restyle_profile(&self, profile: *mut Profile, css: &str) {
        let (script, now) = (loader(css), restyle(css));
        for live in self.tabs.borrow().values() {
            let shared = &live.shared;
            if shared.profile == profile && shared.kind == PageKind::Tab {
                shared.run(|core, tab| {
                    (core.tab_set_init_script)(tab, c_string(&script).as_ptr());
                    (core.tab_eval)(tab, c_string(&now).as_ptr(), false);
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_leading_scheme_is_mapped() {
        assert_eq!(to_chrome("baa://settings"), "chrome://settings");
        assert_eq!(to_chrome("https://a.test/?next=baa://x#baa://y"), "https://a.test/?next=baa://x#baa://y");
        assert_eq!(to_chrome("baa://flags/?u=baa://x"), "chrome://flags/?u=baa://x");
        assert_eq!(to_baa("chrome://flags/?u=chrome://x".into()), "baa://flags/?u=chrome://x");
        assert_eq!(to_baa("https://a.test/?next=chrome://x".into()), "https://a.test/?next=chrome://x");
        assert_eq!(to_baa("chrome-extension://abc/p.html".into()), "chrome-extension://abc/p.html");
    }
}
