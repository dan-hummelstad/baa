//! The mail nav in the sidebar's place (mocks A2/B2, left; WP-H).

use baa_web::WebHost;
use gpui::{div, prelude::*, AnyElement, Context, Window};

use crate::{browser::Browser, theme::Theme};

impl<H: WebHost> Browser<H> {
    pub(crate) fn render_nav(&self, _theme: &Theme, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div().size_full().debug_selector(|| "mail-nav".into()).into_any_element()
    }
}
