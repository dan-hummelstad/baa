use std::path::Path;

use rusqlite::{types::Type, Connection, Error, OptionalExtension, Result};
use serde::Serialize;

use crate::{Session, Settings, SpaceId};

/// SQLite file holding the session and settings, each as one JSON value;
/// `serde(default)` makes added fields migration-free.
// ponytail: whole-session rewrite per save; fine for hundreds of tabs. Give tabs their own
// table if saves show up in profiles.
pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Store> {
        Self::init(Connection::open(path)?)
    }

    fn init(conn: Connection) -> Result<Store> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute("CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL)", [])?;
        Ok(Store { conn })
    }

    /// No stored Space → [`Session::first_run`]. An unreadable session is an error, not a
    /// silent reset, so the next save cannot overwrite it.
    pub fn load_session(&self) -> Result<Session> {
        let stored: Option<Session> = self
            .get("session")?
            .map(|v| serde_json::from_str(&v))
            .transpose()
            .map_err(|e| Error::FromSqlConversionFailure(0, Type::Text, e.into()))?;
        Ok(stored.filter(|s| !s.spaces.is_empty()).unwrap_or_else(Session::first_run))
    }

    /// Incognito Spaces and their tabs are left out.
    pub fn save_session(&self, s: &Session) -> Result<()> {
        let incognito: Vec<SpaceId> = s.spaces.iter().filter(|sp| sp.ephemeral).map(|sp| sp.id).collect();
        let mut s = s.clone();
        s.spaces.retain(|sp| !sp.ephemeral);
        s.tabs.retain(|t| !incognito.contains(&t.space));
        s.active_space = s.active_space.filter(|id| !incognito.contains(id));
        self.put("session", &s)
    }

    /// Unreadable settings fall back to defaults rather than failing startup.
    pub fn load_settings(&self) -> Result<Settings> {
        Ok(self.get("settings")?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default())
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.put("settings", settings)
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        self.conn.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0)).optional()
    }

    fn put(&self, key: &str, value: &impl Serialize) -> Result<()> {
        let json = serde_json::to_string(value).expect("model types serialize");
        self.conn.execute("INSERT OR REPLACE INTO kv VALUES (?1, ?2)", [key, &json]).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProfileId, SidebarSide, Zone};

    #[test]
    fn session_round_trip_skips_incognito() {
        let mut s = Session::first_run();
        let sp = s.active_space().unwrap();
        let a = s.open_tab(sp, "https://a", 10);
        s.tab_mut(a).unwrap().favicon = Some("https://a/f.ico".into());
        let b = s.open_tab(sp, "https://b", 11);
        s.toggle_pin(b);
        s.open_tab(sp, "https://c", 12);
        let c = s.tabs(sp, Zone::Today).next().unwrap().id;
        s.close_tab(c, 13);
        let work = s.add_space("Work", "💼", None);
        s.space_mut(work).unwrap().sidebar_side = Some(SidebarSide::Right);
        let incog = s.add_ephemeral_space("Incognito", ProfileId(u64::MAX));
        s.open_tab(incog, "https://secret", 14);

        let store = Store::init(Connection::open_in_memory().unwrap()).unwrap();
        store.save_session(&s).unwrap();
        let back = store.load_session().unwrap();

        assert_eq!(back.spaces().len(), 2);
        assert_eq!(back.space(work), s.space(work));
        assert_eq!(back.space(sp), s.space(sp));
        assert_eq!(back.profiles(), s.profiles());
        assert_eq!(back.tabs(sp, Zone::Pinned).collect::<Vec<_>>(), s.tabs(sp, Zone::Pinned).collect::<Vec<_>>());
        assert_eq!(back.tabs(sp, Zone::Today).collect::<Vec<_>>(), s.tabs(sp, Zone::Today).collect::<Vec<_>>());
        assert_eq!(back.archived(), s.archived());
        assert_eq!(back.active_space(), None, "incognito was active, so nothing is stored");
        assert!(back.tabs.iter().all(|t| t.space != incog));
        assert_eq!(back.last_id, s.last_id);
    }

    #[test]
    fn empty_db_is_first_run_and_settings_persist() {
        let dir = std::env::temp_dir().join(format!("baa-model-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        {
            let store = Store::open(&dir).unwrap();
            assert_eq!(store.load_session().unwrap().spaces().len(), 1);
            assert_eq!(store.load_settings().unwrap(), Settings::default());
            let settings = Settings { reduce_motion: true, ..Settings::default() };
            store.save_settings(&settings).unwrap();
        }
        let store = Store::open(&dir).unwrap();
        assert!(store.load_settings().unwrap().reduce_motion);
        drop(store);
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", dir.display()));
        }
    }
}
