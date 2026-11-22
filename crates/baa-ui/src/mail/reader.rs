//! The reading pane (mocks A2/B2, right; WP-I).

use baa_mail::ThreadId;
use baa_web::WebHost;
use gpui::{div, prelude::*, AnyElement, Context, Entity, Window};

use crate::{browser::Browser, theme::Theme};

/// The conversation on screen and its message's page.
pub(crate) struct Reader<H: WebHost> {
    pub(crate) thread: ThreadId,
    pub(crate) page: Option<Entity<H::Page>>,
}

impl<H: WebHost> Browser<H> {
    pub(crate) fn render_reader(&self, _theme: &Theme, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div().into_any_element()
    }

    pub(crate) fn select_thread(&mut self, _thread: ThreadId, _window: &mut Window, _cx: &mut Context<Self>) {}

    pub(crate) fn refresh_reader(&mut self, _cx: &mut Context<Self>) {}

    pub(crate) fn reply_to_selected(&mut self, _all: bool, _window: &mut Window, _cx: &mut Context<Self>) {}

    pub(crate) fn forward_selected(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    pub(crate) fn delete_selected(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}
}
