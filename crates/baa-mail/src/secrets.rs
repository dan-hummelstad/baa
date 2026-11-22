use std::{collections::HashMap, sync::Mutex};

use crate::AccountId;

/// Which secret of an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Secret {
    /// A password or app password.
    Password,
    /// An OAuth refresh token.
    RefreshToken,
}

/// Where mail secrets are kept: the Keychain in the app, memory in tests. Never mail.db,
/// baa.db, a log or argv (PLAN-mail.md §9).
pub trait Secrets: Send + Sync {
    fn get(&self, account: AccountId, kind: Secret) -> Option<String>;
    fn set(&self, account: AccountId, kind: Secret, value: &str);
    /// Removes every secret of `account`.
    fn delete(&self, account: AccountId);
}

/// Secrets that are gone when it is dropped, for tests.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<(AccountId, Secret), String>>);

impl Secrets for MemorySecrets {
    fn get(&self, account: AccountId, kind: Secret) -> Option<String> {
        self.0.lock().unwrap().get(&(account, kind)).cloned()
    }

    fn set(&self, account: AccountId, kind: Secret, value: &str) {
        self.0.lock().unwrap().insert((account, kind), value.into());
    }

    fn delete(&self, account: AccountId) {
        self.0.lock().unwrap().retain(|(of, _), _| *of != account);
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
        secrets.set(a, Secret::Password, "hunter2");
        secrets.set(a, Secret::RefreshToken, "refresh");
        secrets.set(b, Secret::Password, "other");
        secrets.set(a, Secret::Password, "changed");
        assert_eq!(secrets.get(a, Secret::Password).as_deref(), Some("changed"));
        assert_eq!(secrets.get(a, Secret::RefreshToken).as_deref(), Some("refresh"));
        secrets.delete(a);
        assert_eq!((secrets.get(a, Secret::Password), secrets.get(a, Secret::RefreshToken)), (None, None));
        assert_eq!(secrets.get(b, Secret::Password).as_deref(), Some("other"), "another account keeps its own");
    }
}
