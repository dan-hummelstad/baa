use baa_model::SpaceId;

use crate::{AccountId, MailSpace};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: AccountId,
    /// `None`: an account of every Space.
    pub space: Option<SpaceId>,
    /// The person's name, as the From line carries it.
    pub name: String,
    pub address: String,
    /// Verification codes from its mail are offered to pages (PLAN-mail.md §7).
    pub codes: bool,
}

/// What [`crate::Mail::add_account`] is given. No `Debug`: it holds the password.
// ponytail: an address and a password only. WP-E adds the server (or its preset) and the
// OAuth sign-in.
pub struct NewAccount {
    /// `None`: an account of every Space.
    pub space: Option<MailSpace>,
    pub name: String,
    pub address: String,
    pub password: String,
}
