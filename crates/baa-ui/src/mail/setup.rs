//! Adding a mail account (WP-K): a provider preset, then an app password or Google sign-in.

use baa_web::WebHost;
use gpui::Context;

use crate::browser::Browser;

impl<H: WebHost> Browser<H> {
    /// Settings › Mail, where accounts are added: the door's "Add an account".
    pub(crate) fn open_mail_settings(&mut self, cx: &mut Context<Self>) {
        self.open_settings_at(None, None, cx);
    }
}
