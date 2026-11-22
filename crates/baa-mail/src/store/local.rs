//! The `local` table: what no server holds, one value a key.

use rusqlite::{params, OptionalExtension};

use super::Store;
use crate::Result;

impl Store {
    pub(crate) fn local(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn().query_row("SELECT value FROM local WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    pub(crate) fn set_local(&self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO local (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}
