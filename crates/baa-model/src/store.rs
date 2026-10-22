use std::path::Path;

use rusqlite::{types::Type, Connection, Error, OptionalExtension, Result};
use serde::Serialize;

use crate::{Session, Settings, SpaceId};

/// One history row per URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryItem {
    pub url: String,
    pub title: String,
    pub visits: u32,
    /// Unix seconds.
    pub last_visit: i64,
}

pub(crate) fn is_web(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

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
        conn.execute("CREATE TABLE IF NOT EXISTS favicons (url TEXT PRIMARY KEY, png BLOB NOT NULL)", [])?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS history (url TEXT PRIMARY KEY, title TEXT NOT NULL, \
             visits INTEGER NOT NULL, last_visit INTEGER NOT NULL)",
            [],
        )?;
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
        let mut s = stored.filter(|s| !s.spaces.is_empty()).unwrap_or_else(Session::first_run);
        // An incognito Space that was active is not stored; open the first one instead.
        if s.active_space.is_none_or(|id| s.space(id).is_none()) {
            s.active_space = s.spaces.first().map(|sp| sp.id);
        }
        Ok(s)
    }

    /// Incognito Spaces and their tabs are left out.
    pub fn save_session(&self, s: &Session) -> Result<()> {
        let incognito: Vec<SpaceId> = s.spaces.iter().filter(|sp| sp.ephemeral).map(|sp| sp.id).collect();
        let mut s = s.clone();
        s.spaces.retain(|sp| !sp.ephemeral);
        s.tabs.retain(|t| !incognito.contains(&t.space));
        s.folders.retain(|f| !incognito.contains(&f.space));
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

    /// Favicons as PNG, by their URL, so restored tabs show theirs at launch.
    // ponytail: never pruned. Ceiling: one row per site ever visited; drop rows
    // no tab references if the file grows.
    pub fn load_favicons(&self) -> Result<Vec<(String, Vec<u8>)>> {
        let mut stmt = self.conn.prepare("SELECT url, png FROM favicons")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    pub fn save_favicon(&self, url: &str, png: &[u8]) -> Result<()> {
        self.conn
            .execute("INSERT OR REPLACE INTO favicons VALUES (?1, ?2)", rusqlite::params![url, png])
            .map(drop)
    }

    /// Non-http(s) URLs are ignored; an empty title keeps the known one.
    pub fn record_visit(&self, url: &str, title: &str, at: i64) -> Result<()> {
        if !is_web(url) {
            return Ok(());
        }
        self.conn
            .execute(
                "INSERT INTO history VALUES (?1, ?2, 1, ?3) ON CONFLICT(url) DO UPDATE SET visits = visits + 1, \
                 last_visit = max(last_visit, excluded.last_visit), \
                 title = CASE excluded.title WHEN '' THEN title ELSE excluded.title END",
                rusqlite::params![url, title, at],
            )
            .map(drop)
    }

    pub fn set_history_title(&self, url: &str, title: &str) -> Result<()> {
        self.conn
            .execute("UPDATE history SET title = ?2 WHERE url = ?1 AND ?2 != ''", [url, title])
            .map(drop)
    }

    /// Every whitespace-separated word must appear in the URL or title.
    // ponytail: SQLite LIKE folds ASCII case only, and it is a table scan. Ceiling: fine for
    // ~100k rows; past that, FTS5.
    pub fn search_history(&self, query: &str, limit: usize) -> Result<Vec<HistoryItem>> {
        let words: Vec<String> = query
            .split_whitespace()
            .map(|w| format!("%{}%", w.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")))
            .collect();
        let mut sql = String::from("SELECT url, title, visits, last_visit FROM history WHERE 1");
        for i in 1..=words.len() {
            sql += &format!(" AND (url LIKE ?{i} ESCAPE '\\' OR title LIKE ?{i} ESCAPE '\\')");
        }
        // Frecency: visits divided by (1 + days since the last one), so a page visited
        // ten times last month ranks with one visited once today.
        sql += &format!(
            " ORDER BY visits / (1.0 + max(0, {} - last_visit) / 86400.0) DESC LIMIT {limit}",
            crate::now()
        );
        self.history_rows(&sql, rusqlite::params_from_iter(words))
    }

    /// Newest first.
    pub fn recent_history(&self, limit: usize) -> Result<Vec<HistoryItem>> {
        self.history_rows(
            &format!("SELECT url, title, visits, last_visit FROM history ORDER BY last_visit DESC LIMIT {limit}"),
            [],
        )
    }

    pub fn remove_history(&self, url: &str) -> Result<()> {
        self.conn.execute("DELETE FROM history WHERE url = ?1", [url]).map(drop)
    }

    pub fn clear_history(&self) -> Result<()> {
        self.conn.execute("DELETE FROM history", []).map(drop)
    }

    /// Merges into what is there: visit counts add up, the newest visit wins, a known
    /// title is kept. Non-http(s) URLs are skipped.
    pub fn import_history(&self, items: &[HistoryItem]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO history VALUES (?1, ?2, ?3, ?4) ON CONFLICT(url) DO UPDATE SET \
                 visits = visits + excluded.visits, last_visit = max(last_visit, excluded.last_visit), \
                 title = CASE title WHEN '' THEN excluded.title ELSE title END",
            )?;
            for h in items.iter().filter(|h| is_web(&h.url)) {
                stmt.execute(rusqlite::params![h.url, h.title, h.visits, h.last_visit])?;
            }
        }
        tx.commit()
    }

    fn history_rows(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<HistoryItem>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params, |r| {
            Ok(HistoryItem { url: r.get(0)?, title: r.get(1)?, visits: r.get(2)?, last_visit: r.get(3)? })
        })?;
        rows.collect()
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
    fn favicons_round_trip() {
        let store = Store::open(":memory:").unwrap();
        store.save_favicon("https://a/f.ico", b"png1").unwrap();
        store.save_favicon("https://a/f.ico", b"png2").unwrap();
        assert_eq!(store.load_favicons().unwrap(), [("https://a/f.ico".to_string(), b"png2".to_vec())]);
    }

    #[test]
    fn history_records_searches_and_merges() {
        let store = Store::open(":memory:").unwrap();
        let now = crate::now() as i64;
        store.record_visit("https://docs.rs/rusqlite", "rusqlite - Rust", now - 30 * 86400).unwrap();
        for _ in 0..3 {
            store.record_visit("https://docs.rs/rusqlite", "", now - 30 * 86400).unwrap();
        }
        store.record_visit("https://docs.rs/serde", "", now).unwrap();
        store.set_history_title("https://docs.rs/serde", "Serde Docs").unwrap();
        store.record_visit("about:blank", "x", now).unwrap();
        store.record_visit("https://100%_sure.example", "", now).unwrap();

        let hit = |q: &str| store.search_history(q, 10).unwrap().into_iter().map(|h| h.url).collect::<Vec<_>>();
        assert_eq!(hit("DOCS rust"), ["https://docs.rs/rusqlite"], "every word, any case, url or title");
        assert_eq!(hit("docs"), ["https://docs.rs/serde", "https://docs.rs/rusqlite"], "recent beats frequent-but-old");
        assert_eq!(hit("%_"), ["https://100%_sure.example"], "wildcards are literal");
        let r = store.recent_history(10).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[2], HistoryItem {
            url: "https://docs.rs/rusqlite".into(),
            title: "rusqlite - Rust".into(),
            visits: 4,
            last_visit: now - 30 * 86400
        });

        store
            .import_history(&[
                HistoryItem { url: "https://docs.rs/serde".into(), title: "old".into(), visits: 5, last_visit: 1 },
                HistoryItem { url: "https://new.example".into(), title: "New".into(), visits: 2, last_visit: 7 },
                HistoryItem { url: "file:///etc".into(), title: String::new(), visits: 1, last_visit: 7 },
            ])
            .unwrap();
        let serde = store.search_history("serde", 1).unwrap().remove(0);
        assert_eq!((serde.title.as_str(), serde.visits, serde.last_visit), ("Serde Docs", 6, now));
        store.remove_history("https://new.example").unwrap();
        assert_eq!(store.recent_history(10).unwrap().len(), 3);
        store.clear_history().unwrap();
        assert!(store.recent_history(10).unwrap().is_empty());
    }

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
        assert_eq!(back.active_space(), Some(sp), "incognito was active, so the first Space opens");
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
