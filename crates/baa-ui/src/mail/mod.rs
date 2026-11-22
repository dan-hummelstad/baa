//! The mail views (PLAN-mail.md §2, §5). So far only what `Browser` holds of them.

use std::sync::Arc;

use baa_mail::Mail;
use baa_web::WebHost;
use gpui::Entity;

/// Mail in one window.
#[allow(dead_code)] // nothing makes or reads it until WP-G
pub(crate) struct MailUi<H: WebHost> {
    /// The process's one `Mail`, shared by its windows.
    pub(crate) mail: Arc<Mail>,
    /// The message and editor pages on screen.
    pub(crate) documents: Vec<Entity<H::Page>>,
}
