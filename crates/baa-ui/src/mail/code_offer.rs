//! A verification code from mail, offered under a page's one-time-code field (PLAN-mail.md §7; WP-L).

use baa_web::WebHost;
use gpui::Context;

use crate::browser::Browser;

pub(crate) struct Offer;

#[derive(Default)]
pub(crate) struct Offers;

impl<H: WebHost> Browser<H> {
    pub(crate) fn on_codes_changed(&mut self, _cx: &mut Context<Self>) {}

    pub(crate) fn drop_hidden_offers(&mut self, _cx: &mut Context<Self>) {}
}
