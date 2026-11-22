//! A draft in the reading pane (mocks A3/B3; WP-J).

use baa_mail::{Draft, DraftId};
use baa_web::WebHost;
use gpui::{div, prelude::*, AnyElement, Context, Entity, Window};

use super::Field;
use crate::{
    browser::Browser,
    input::{Key, TextField},
    theme::Theme,
};

/// One open draft: a tab of the strip.
pub(crate) struct Composer<H: WebHost> {
    pub(crate) id: DraftId,
    /// The body's editor document.
    pub(crate) page: Option<Entity<H::Page>>,
    pub(crate) to: TextField,
    pub(crate) cc: TextField,
    pub(crate) bcc: TextField,
    pub(crate) subject: TextField,
}

impl<H: WebHost> Composer<H> {
    pub(crate) fn field(&self, field: Field) -> Option<&TextField> {
        match field {
            Field::To => Some(&self.to),
            Field::Cc => Some(&self.cc),
            Field::Bcc => Some(&self.bcc),
            Field::Subject => Some(&self.subject),
            Field::Search => None,
        }
    }

    pub(crate) fn field_mut(&mut self, field: Field) -> Option<&mut TextField> {
        match field {
            Field::To => Some(&mut self.to),
            Field::Cc => Some(&mut self.cc),
            Field::Bcc => Some(&mut self.bcc),
            Field::Subject => Some(&mut self.subject),
            Field::Search => None,
        }
    }
}

impl<H: WebHost> Browser<H> {
    pub(crate) fn render_composer(&self, _theme: &Theme, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        div().into_any_element()
    }

    /// ⌘N: a new draft from the Space's first account.
    pub(crate) fn new_message(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    /// Opens `draft` (a new one, a reply, a forward) as a tab in front.
    pub(crate) fn open_draft(&mut self, _draft: Draft, _window: &mut Window, _cx: &mut Context<Self>) {}

    /// ⌘↩.
    pub(crate) fn send_front_draft(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    /// The view state's open drafts, after a Space switch or at launch.
    pub(crate) fn restore_drafts(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    pub(crate) fn on_compose_key(&mut self, _field: Field, _key: Key, _before: &str, _window: &mut Window, _cx: &mut Context<Self>) {}

    pub(crate) fn on_sent(&mut self, _draft: DraftId, _cx: &mut Context<Self>) {}
}
