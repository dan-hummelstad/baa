//! Developer tools in the window: the responsive view (E2) and localhost console error badges (E4).

use super::*;
use baa_web::{ConsoleLevel, Device};

/// A phone pane's width: `Device::Phone`'s 393 CSS px viewport.
pub(crate) const PHONE_W: f32 = 393.;

/// `localhost`, `127.*` or `[::1]`, from the parsed host (never the URL's text).
pub(crate) fn is_local(url: &str) -> bool {
    match url::Url::parse(url).ok().as_ref().and_then(url::Url::host) {
        Some(url::Host::Domain(d)) => d == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

impl<H: WebHost> Browser<H> {
    /// ⌥⌘M: the active tab's page again beside it, shown as a phone; again closes that pane.
    // ponytail: one device preset; a picker when asked.
    pub(crate) fn responsive_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panes = self.shown_panes();
        if let Some(phone) = panes.iter().find(|id| self.phone_panes.contains(id)) {
            return self.close_tab_id(*phone, window, cx);
        }
        let (Some(space), Some(active)) = (self.session.active_space(), self.active_tab()) else { return };
        // The address the user sees in the active pane: never a page-supplied one still
        // waiting for its page to adopt Chrome's tab.
        if panes.len() >= baa_model::SPLIT_MAX || self.supplied.contains(&active) || !self.pages.contains_key(&active) {
            return;
        }
        let Some(url) = self.session.tab(active).map(|t| t.url.clone()) else { return };
        if !url::Url::parse(&url).is_ok_and(|u| matches!(u.scheme(), "http" | "https")) {
            return;
        }
        // As a typed URL in a new tab, then into the split beside the active pane.
        let phone = self.session.open_tab(space, &url, now());
        self.session.activate_tab(active, now());
        if self.session.add_to_split(space, phone, usize::MAX, now()) != Some(phone) {
            return self.close_tab_id(phone, window, cx);
        }
        self.phone_panes.insert(phone);
        self.sync(window, cx);
        if let Some((page, _)) = self.pages.get(&phone) {
            page.read(cx).set_emulation(Some(Device::Phone));
        }
    }

    /// Run from `sync`: per-tab dev state goes with its page, and a phone tab that left its
    /// split is a plain tab again (on restore every tab is one: the set is not saved).
    pub(super) fn retain_dev(&mut self, cx: &mut Context<Self>) {
        let pages = &self.pages;
        self.console_errors.retain(|id, _| pages.contains_key(id));
        self.console_watched.retain(|id| pages.contains_key(id));
        let session = &self.session;
        let in_split = |id: &TabId| {
            session
                .tab(*id)
                .and_then(|t| session.space(t.space)?.split.as_ref())
                .is_some_and(|s| s.tabs.contains(id))
        };
        let left: Vec<TabId> = self.phone_panes.iter().copied().filter(|id| !in_split(id)).collect();
        for id in left {
            self.phone_panes.remove(&id);
            if let Some((page, _)) = self.pages.get(&id) {
                page.read(cx).set_emulation(None);
            }
        }
    }

    /// The phone pane's strip: what it shows as, and its own reload.
    pub(super) fn phone_strip(&self, id: TabId, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_none()
            .h(px(28.))
            .px(px(10.))
            .flex()
            .items_center()
            .justify_between()
            .bg(surface::bg())
            .border_b_1()
            .border_color(surface::border())
            .text_size(px(11.))
            .text_color(surface::muted())
            .child(format!("Phone · {PHONE_W}"))
            .child(
                div()
                    .id(("phone-reload", id.0))
                    .debug_selector(|| "phone-reload".into())
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(r(5.))
                    .hover(|d| d.bg(surface::key()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some((page, _)) = this.pages.get(&id) {
                            page.read(cx).reload(false);
                        }
                    }))
                    .child(icon("rotate-cw", 13., surface::icon().into())),
            )
    }

    /// A localhost page reports its console from now on (once per page).
    pub(super) fn watch_if_local(&mut self, id: TabId, url: &str, cx: &mut Context<Self>) {
        if !self.settings.console_badge || !is_local(url) || !self.console_watched.insert(id) {
            return;
        }
        if let Some((page, _)) = self.pages.get(&id) {
            page.read(cx).watch_console();
        }
    }

    /// Every page event passes here first: a load or navigation starts the count over,
    /// and an error on a localhost page adds one.
    pub(super) fn console_event(&mut self, id: TabId, event: &PageEvent, cx: &mut Context<Self>) {
        match event {
            PageEvent::UrlChanged(url) => {
                self.console_errors.remove(&id);
                self.watch_if_local(id, url, cx);
            }
            PageEvent::LoadingChanged(true) => {
                self.console_errors.remove(&id);
            }
            PageEvent::Console { level: ConsoleLevel::Error, .. }
                if self.settings.console_badge && self.session.tab(id).is_some_and(|t| is_local(&t.url)) =>
            {
                *self.console_errors.entry(id).or_default() += 1;
                cx.notify();
            }
            _ => {}
        }
    }

    /// The badge's click: the tab, with DevTools open beside it.
    pub(crate) fn open_console(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        self.activate_tab(id, window, cx);
        if !self.devtools.contains_key(&id) {
            self.toggle_devtools(None, window, cx);
        }
    }

    /// The red error count on a tab's row, `ids` summed (a folder's tabs), if any.
    pub(crate) fn console_badge(&self, key: (&'static str, u64), ids: &[TabId], cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.settings.console_badge {
            return None;
        }
        let (count, first) = ids.iter().fold((0, None), |(n, first), id| match self.console_errors.get(id) {
            Some(c) => (n + c, first.or(Some(*id))),
            None => (n, first),
        });
        let first = first.filter(|_| count > 0)?;
        Some(
            div()
                .id(key)
                .debug_selector(|| "console-badge".into())
                .flex_none()
                .h(px(16.))
                .min_w(px(16.))
                .px(px(4.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgb(0xE5484D))
                .text_color(gpui::white())
                .text_size(px(10.))
                .font_weight(gpui::FontWeight::BOLD)
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.open_console(first, window, cx);
                }))
                .child(if count > 99 { "99+".to_string() } else { count.to_string() })
                .into_any_element(),
        )
    }
}

#[cfg(test)]
#[test]
fn only_loopback_hosts_are_local() {
    for yes in ["http://localhost:3000/", "http://127.0.0.1:8080/x", "http://127.4.5.6/", "http://[::1]:5173/"] {
        assert!(is_local(yes), "{yes}");
    }
    for no in ["https://127.evil.com/", "http://localhost.evil.com/", "http://a.com@evil.com/", "https://example.com/", "file:///tmp/a", "not a url"] {
        assert!(!is_local(no), "{no}");
    }
}
