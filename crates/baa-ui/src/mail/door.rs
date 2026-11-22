//! The Archive's Mailbox section: the door into mail (mocks A1/B1; WP-H).

use baa_web::WebHost;
use gpui::{div, prelude::*, AnyElement, Context, Window};

use crate::{browser::Browser, theme::Theme};

impl<H: WebHost> Browser<H> {
    pub(crate) fn render_door(&self, _theme: &Theme, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .child(div().id("open-mailbox").on_click(cx.listener(|this, _, window, cx| this.open_mail_from_door(window, cx))).child("Open Mailbox"))
            .into_any_element()
    }
}
