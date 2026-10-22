//! Page tools and one-off jobs run from the command bar or settings: view
//! source, reader mode, Picture in Picture, translate, history import and
//! clearing, Arc import, password manager setup. Results show as a toast.

use baa_model::import;
use baa_web::{PageHandle, WebHost};
use gpui::{div, prelude::*, px, Context, SharedString, Window};

use crate::{
    browser::{shadow, Browser, PALETTE},
    motion::Feel,
};

/// How long a toast stays before it fades.
const TOAST_FOR: std::time::Duration = std::time::Duration::from_millis(2600);

/// The page's main text in a clean column; run again (or reload) to go back.
// ponytail: a "most paragraph text wins" heuristic, not Readability.js. Pages
// that build their article from divs without <p> fall back to <main>/<article>.
pub(crate) const READER_SCRIPT: &str = r#"(() => {
  const root = document.documentElement;
  if (root.dataset.baaReader) { location.reload(); return; }
  const score = el => [...el.querySelectorAll('p')].reduce((n, p) => n + p.innerText.length, 0);
  const candidates = [...document.querySelectorAll('article, main, [role=main], section, div')];
  let best = document.querySelector('article') || document.body, top = 0;
  for (const el of candidates) {
    const s = score(el) - el.querySelectorAll('a').length * 20;
    if (s > top * 1.25 || (s > top && el.contains(best))) { top = s; best = el; }
  }
  const article = best.cloneNode(true);
  article.querySelectorAll('script, style, nav, aside, footer, form, iframe, button, svg, [aria-hidden=true]').forEach(e => e.remove());
  const title = (document.querySelector('h1') || {}).innerText || document.title;
  const style = document.createElement('style');
  style.textContent = ':root{color-scheme:light dark}body{margin:0;background:#FBFAF7;color:#1d1b20;font:20px/1.65 ui-serif,Georgia,serif}' +
    '@media (prefers-color-scheme:dark){body{background:#1c1b20;color:#e7e4ec}a{color:#b8a8ff}}' +
    'main{max-width:680px;margin:0 auto;padding:64px 24px 120px}h1{font:700 2em/1.2 ui-sans-serif,system-ui;margin:0 0 .8em}' +
    'img,video,figure{max-width:100%;height:auto}pre{overflow:auto;font-size:.8em}a{color:#5b45d6}table{display:none}';
  const main = document.createElement('main');
  const h1 = document.createElement('h1');
  h1.textContent = title;
  main.append(h1, ...article.childNodes);
  // The page's own styles and scripts' markup go; what is left is the text.
  document.head.replaceChildren(style);
  document.body.replaceChildren(main);
  for (const el of [root, document.body]) for (const a of [...el.attributes]) el.removeAttribute(a.name);
  document.title = title;
  root.dataset.baaReader = '1';
  scrollTo(0, 0);
})()"#;

/// The page's largest video goes into (or comes back from) Picture in Picture.
pub(crate) const PIP_SCRIPT: &str = r#"(() => {
  if (document.pictureInPictureElement) return document.exitPictureInPicture();
  const videos = [...document.querySelectorAll('video')].filter(v => v.readyState > 0);
  videos.sort((a, b) => b.clientWidth * b.clientHeight - a.clientWidth * a.clientHeight);
  if (videos[0]) videos[0].requestPictureInPicture();
})()"#;

/// Percent-encodes everything but RFC 3986's unreserved characters.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            b => format!("%{b:02X}"),
        })
        .collect()
}

/// The user's language for translation, e.g. "en" from `LANG=en_GB.UTF-8`.
// ponytail: LANG only; a GUI app launched from Finder often has none, so
// English. Ceiling: read the system's preferred languages.
fn language() -> String {
    std::env::var("LANG")
        .ok()
        .and_then(|l| l.split(['_', '.']).next().map(str::to_string))
        .filter(|l| l.len() == 2)
        .unwrap_or_else(|| "en".into())
}

impl<H: WebHost> Browser<H> {
    pub(crate) fn show_toast(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast = Some(text.into());
        self.toasts += 1;
        let n = self.toasts;
        self.motion.from(("toast", 0), Feel::Glide, 0.0, 1.0);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_FOR).await;
            let _ = this.update(cx, |this, cx| {
                if this.toasts == n {
                    this.motion.to(("toast", 0), Feel::Glide, 0.0);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn render_toast(&self) -> Option<impl IntoElement> {
        let t = self.motion.get(("toast", 0), Feel::Glide);
        let text = self.toast.clone().filter(|_| t > 0.01)?;
        Some(
            div()
                .absolute()
                .bottom(px(28.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .occlude()
                        .mb(px(-16. * (1. - t)))
                        .opacity(t.clamp(0., 1.))
                        .px(px(14.))
                        .py(px(9.))
                        .rounded(px(10.))
                        .bg(gpui::rgba(0x1E1B26F0))
                        .shadow(vec![shadow(gpui::rgba(0x0A08143D).into(), 8., 24.)])
                        .text_size(px(13.))
                        .text_color(gpui::white())
                        .child(text),
                ),
        )
    }

    pub(crate) fn view_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(url) = self.active_page().map(|p| p.read(cx).url()) else { return };
        if !url.starts_with("view-source:") {
            self.open_url(&format!("view-source:{url}"), window, cx);
        }
    }

    // ponytail: Google Translate's page proxy, opened on the user's request;
    // Chromium's built-in translate talks to Google in the background and is off.
    pub(crate) fn translate_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(url) = self.active_page().map(|p| p.read(cx).url()) else { return };
        if !url.starts_with("http") {
            return;
        }
        let target = format!(
            "https://translate.google.com/translate?sl=auto&tl={}&u={}",
            language(),
            encode(&url)
        );
        self.navigate(&target, true, window, cx);
    }

    pub(crate) fn clear_history(&mut self, cx: &mut Context<Self>) {
        match self.store().map(|s| s.clear_history()) {
            Some(Err(e)) => log::error!("clearing history: {e}"),
            _ => self.show_toast("Browsing history cleared", cx),
        }
    }

    /// Arc's Spaces, pinned tabs, folders and favourites, as new Spaces.
    pub(crate) fn import_arc(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let arc = import::arc_sidebar_path().ok_or_else(|| "Arc's data was not found".into()).and_then(|p| import::read_arc_sidebar(&p));
        let arc = match arc {
            Ok(arc) => arc,
            Err(e) => {
                log::error!("importing from Arc: {e}");
                return self.show_toast(format!("Couldn't import from Arc: {e}"), cx);
            }
        };
        let before: Vec<_> = self.session.spaces().iter().map(|s| s.id).collect();
        let tabs = self.session.import_arc(&arc);
        let added: Vec<_> = self.session.spaces().iter().map(|s| s.id).filter(|id| !before.contains(id)).collect();
        for (i, id) in added.iter().enumerate() {
            if let Some(space) = self.session.space_mut(*id) {
                let (top, bottom) = PALETTE[(before.len() + i) % PALETTE.len()];
                (space.accent, space.accent_to) = (top, Some(bottom));
            }
        }
        self.sync(window, cx);
        let spaces = added.len();
        self.show_toast(
            format!(
                "Imported {spaces} Space{} and {tabs} tab{} from Arc",
                if spaces == 1 { "" } else { "s" },
                if tabs == 1 { "" } else { "s" }
            ),
            cx,
        );
    }

    /// Another browser's history, read off the main thread.
    pub(crate) fn import_history(&mut self, source: import::Source, cx: &mut Context<Self>) {
        let name = source.name;
        let read = cx.background_spawn(async move { import::read_history(&source).map_err(|e| e.to_string()) });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                let saved = result.and_then(|items| {
                    let n = items.len();
                    this.store()
                        .map(|s| s.import_history(&items))
                        .transpose()
                        .map(|_| n)
                        .map_err(|e| e.to_string())
                });
                let message = match saved {
                    Ok(n) => format!("Imported {n} pages of history from {name}"),
                    Err(e) => {
                        log::error!("importing history from {name}: {e}");
                        format!("Couldn't import history from {name}")
                    }
                };
                this.show_toast(message, cx);
            });
        })
        .detach();
    }

    /// Password manager extensions; baa-web connects them to their Mac apps.
    pub(crate) fn set_up_passwords(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_extensions("https://chromewebstore.google.com/search/password%20manager", window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_encoded_for_a_query() {
        assert_eq!(encode("https://a.example/?q=1 2"), "https%3A%2F%2Fa.example%2F%3Fq%3D1%202");
    }

}
