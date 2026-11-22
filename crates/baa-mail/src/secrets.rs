use std::{collections::HashMap, sync::Arc, sync::Mutex};

use crate::{AccountId, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Secret {
    Password,
    RefreshToken,
}

/// Where mail secrets are kept: the Keychain in the app, memory in tests. Never mail.db,
/// baa.db, a log or argv (PLAN-mail.md §9).
pub trait Secrets: Send + Sync {
    fn get(&self, account: AccountId, kind: Secret) -> Option<String>;
    fn set(&self, account: AccountId, kind: Secret, value: &str) -> Result<()>;
    /// Removes every secret of `account`.
    fn delete(&self, account: AccountId);
}

/// So [`crate::Mail::open`] can give the store and the connector the same secrets.
impl<S: Secrets + ?Sized> Secrets for Arc<S> {
    fn get(&self, account: AccountId, kind: Secret) -> Option<String> {
        (**self).get(account, kind)
    }

    fn set(&self, account: AccountId, kind: Secret, value: &str) -> Result<()> {
        (**self).set(account, kind, value)
    }

    fn delete(&self, account: AccountId) {
        (**self).delete(account)
    }
}

/// Secrets that are gone when it is dropped, for tests.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<(AccountId, Secret), String>>);

impl Secrets for MemorySecrets {
    fn get(&self, account: AccountId, kind: Secret) -> Option<String> {
        self.0.lock().unwrap().get(&(account, kind)).cloned()
    }

    fn set(&self, account: AccountId, kind: Secret, value: &str) -> Result<()> {
        self.0.lock().unwrap().insert((account, kind), value.into());
        Ok(())
    }

    fn delete(&self, account: AccountId) {
        self.0.lock().unwrap().retain(|(of, _), _| *of != account);
    }
}

/// The macOS Keychain: one generic-password item an account and kind, under the service the
/// app names (a dev build names its own, so it never reads a release's items).
#[cfg(target_os = "macos")]
pub struct Keychain {
    service: String,
}

#[cfg(target_os = "macos")]
impl Keychain {
    pub fn new(service: &str) -> Keychain {
        Keychain { service: service.into() }
    }

    fn item(account: AccountId, kind: Secret) -> String {
        let kind = match kind {
            Secret::Password => "password",
            Secret::RefreshToken => "refresh-token",
        };
        format!("{}.{kind}", account.0)
    }
}

#[cfg(target_os = "macos")]
impl Secrets for Keychain {
    fn get(&self, account: AccountId, kind: Secret) -> Option<String> {
        let bytes = security_framework::passwords::get_generic_password(&self.service, &Self::item(account, kind)).ok()?;
        String::from_utf8(bytes).ok()
    }

    fn set(&self, account: AccountId, kind: Secret, value: &str) -> Result<()> {
        security_framework::passwords::set_generic_password(&self.service, &Self::item(account, kind), value.as_bytes())
            .map_err(|e| crate::Error::Other(format!("The Keychain did not keep the sign-in ({})", e.code())))
    }

    fn delete(&self, account: AccountId) {
        for kind in [Secret::Password, Secret::RefreshToken] {
            let _ = security_framework::passwords::delete_generic_password(&self.service, &Self::item(account, kind));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_secrets_round_trip_and_delete_removes_every_kind_of_one_account() {
        let secrets = MemorySecrets::default();
        let (a, b) = (AccountId(1), AccountId(2));
        assert_eq!(secrets.get(a, Secret::Password), None);
        secrets.set(a, Secret::Password, "hunter2").unwrap();
        secrets.set(a, Secret::RefreshToken, "refresh").unwrap();
        secrets.set(b, Secret::Password, "other").unwrap();
        secrets.set(a, Secret::Password, "changed").unwrap();
        assert_eq!(secrets.get(a, Secret::Password).as_deref(), Some("changed"));
        assert_eq!(secrets.get(a, Secret::RefreshToken).as_deref(), Some("refresh"));
        secrets.delete(a);
        assert_eq!((secrets.get(a, Secret::Password), secrets.get(a, Secret::RefreshToken)), (None, None));
        assert_eq!(secrets.get(b, Secret::Password).as_deref(), Some("other"), "another account keeps its own");
    }

    /// Compiled, never run: a test must not touch the user's Keychain.
    #[cfg(target_os = "macos")]
    #[allow(dead_code)]
    fn the_keychain_is_a_secrets_store() -> Box<dyn Secrets> {
        Box::new(Keychain::new("me.baa.mail.test"))
    }
}
