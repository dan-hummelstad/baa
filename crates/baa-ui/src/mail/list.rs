//! The message list and the status line under it (mocks A2/B2, middle; WP-H).

use baa_web::WebHost;
use gpui::{div, prelude::*, AnyElement, Context, Window};

use crate::{browser::Browser, input::Key, theme::Theme};

impl<H: WebHost> Browser<H> {
    pub(crate) fn render_list(&self, _theme: &Theme, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div().w(gpui::px(380.)).h_full().into_any_element()
    }

    /// A key in the search field; `before` is its text before the key.
    pub(crate) fn on_search_key(&mut self, _key: Key, _before: &str, _window: &mut Window, _cx: &mut Context<Self>) {}
}
