//! Where the link under the pointer goes: a pill at the bottom-left of that tab's pane.

use super::*;

/// The pill's text: a web URL's host (always whole) and the rest after it, or `None` and
/// any other URL whole, scheme first. The page chose it: it is only ever shown.
pub(crate) fn hover_text(url: &str) -> (Option<String>, String) {
    if let Some(parsed) = url::Url::parse(url).ok().filter(|u| matches!(u.scheme(), "http" | "https")) {
        let host = host_of(url);
        if !host.is_empty() {
            let path = Some(parsed.path()).filter(|p| *p != "/").unwrap_or_default();
            let query = parsed.query().map(|q| format!("?{q}")).unwrap_or_default();
            return (Some(plain_text(&host)), plain_text(&format!("{path}{query}")));
        }
    }
    (None, plain_text(url))
}

impl<H: WebHost> Browser<H> {
    pub(super) fn on_target_url(&mut self, id: TabId, url: &str, cx: &mut Context<Self>) {
        self.hover_url = (!url.is_empty()).then(|| (id, url.to_string()));
        cx.notify();
    }

    pub(super) fn clear_hover_url(&mut self, id: TabId, cx: &mut Context<Self>) {
        if self.hover_url.as_ref().is_some_and(|(tab, _)| *tab == id) {
            self.hover_url = None;
            cx.notify();
        }
    }

    /// A permission, dialog, sign-in question or Claude approval is on screen: nothing
    /// a page or an extension chose may show beside it.
    pub(crate) fn prompt_shown(&self) -> bool {
        let tab_ask = |t: &TabId| self.asks.get(t).is_some_and(|a| a.dialog.is_some() || !a.permissions.is_empty());
        !self.agent.asks.is_empty() || !self.account_asks.is_empty() || self.shown_panes().iter().any(tab_ask)
    }

    /// `content`, a tab's page, with the pill over its bottom-left corner; the pointer
    /// leaving the page clears it.
    pub(super) fn with_hover_url(&self, tab: TabId, content: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(("page-hover", tab.0))
            .relative()
            .size_full()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if !*hovered {
                    this.clear_hover_url(tab, cx);
                }
            }))
            .child(content)
            .children(self.hover_pill(tab))
            .into_any_element()
    }

    fn hover_pill(&self, tab: TabId) -> Option<AnyElement> {
        let url = self.hover_url.as_ref().filter(|(t, _)| *t == tab).map(|(_, u)| u)?;
        if self.prompt_shown() {
            return None;
        }
        let (host, rest) = hover_text(url);
        let line = |d: gpui::Div| d.overflow_hidden().whitespace_nowrap();
        let pill = div()
            .debug_selector(|| "hover-url".into())
            .absolute()
            .left(px(6.))
            .bottom(px(6.))
            .max_w(relative(0.6))
            .flex()
            .px(px(8.))
            .py(px(3.))
            .rounded(r(7.))
            .bg(surface::bg())
            .border_1()
            .border_color(surface::border())
            .shadow(vec![shadow(gpui::rgba(0x18122C1F).into(), 2., 8.)])
            .text_size(px(12.))
            .text_color(surface::ink());
        let pill = match host {
            // The host may only be cut when it alone is wider than the pill, and then on the
            // left: the end of it names the site.
            Some(host) => pill
                .child(cut_left(div().flex_none().max_w_full()).debug_selector(|| "hover-host".into()).child(host))
                .child(line(div().min_w_0().flex_shrink(1.)).text_ellipsis().text_color(surface::muted()).debug_selector(|| "hover-rest".into()).child(rest)),
            None => pill.child(line(div().min_w_0()).text_ellipsis().debug_selector(|| "hover-rest".into()).child(rest)),
        };
        Some(pill.into_any_element())
    }
}

#[cfg(test)]
#[test]
fn hover_text_keeps_the_host_apart_and_strips_what_could_disguise_it() {
    assert_eq!(hover_text("https://www.a.com/"), (Some("a.com".into()), "".into()));
    assert_eq!(hover_text("https://u:p@b.com:8443/x/y?q=1#f"), (Some("b.com:8443".into()), "/x/y?q=1".into()));
    assert_eq!(hover_text("mailto:me@a.com"), (None, "mailto:me@a.com".into()));
    assert_eq!(hover_text("javascript:alert(1)\n\u{202E}gpj.exe"), (None, "javascript:alert(1)gpj.exe".into()));
    assert_eq!(hover_text("data:text/html,x"), (None, "data:text/html,x".into()));
}
