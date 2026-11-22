//! Google sign-in (PLAN-mail.md §4 "OAuth").

use crate::{Error, Result};

/// What a sign-in gives. No `Debug`: both are secrets.
// ponytail: empty until WP-E's exchange fills it (a refresh and an access token).
pub struct Tokens(());

/// One Google sign-in under way. No `Debug`: it holds the PKCE verifier.
pub struct GoogleSignIn {
    url: String,
}

impl GoogleSignIn {
    /// Starts the loopback listener. [`Error::NotSetUp`] when this build has no Google client.
    pub fn start() -> Result<GoogleSignIn> {
        Err(Error::NotSetUp)
    }

    /// Google's consent page, to open as a tab of the Space.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Blocks until Google sends the browser back, or 5 minutes pass, then exchanges the code
    /// for tokens. Call it off the UI thread.
    pub fn finish(self) -> Result<Tokens> {
        Err(Error::NotSetUp)
    }
}
