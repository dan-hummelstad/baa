//! The command row over the list and reading pane: the ribbon's Home set in ASCII, pills in
//! Arc, from one table (mocks A2/B2; WP-I).

use baa_web::WebHost;
use gpui::{div, prelude::*, AnyElement, Context, Window};

use crate::{browser::Browser, theme::Theme};

impl<H: WebHost> Browser<H> {
    pub(crate) fn render_commands(&self, _theme: &Theme, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div().into_any_element()
    }
}
