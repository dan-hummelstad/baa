//! The tab strip over the mail view: Inbox, then one tab a draft (mocks A2/B2, A3/B3; WP-J).

use baa_web::WebHost;
use gpui::{div, prelude::*, AnyElement, Context, Window};

use crate::{browser::Browser, theme::Theme};

impl<H: WebHost> Browser<H> {
    pub(crate) fn render_mail_tabs(&self, _theme: &Theme, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div().into_any_element()
    }
}
