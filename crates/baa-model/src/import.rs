//! Read history and Arc's sidebar from other browsers' files on this machine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::Value;

use crate::store::is_web;
use crate::HistoryItem;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

// ponytail: newest 20k rows per browser; older history stays behind.
const HISTORY_CAP: usize = 20_000;
/// Seconds between 1601-01-01 (Chromium's epoch) and 1970-01-01.
const CHROMIUM_EPOCH: i64 = 11_644_473_600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Chromium,
    Firefox,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub name: &'static str,
    pub history: PathBuf,
    pub kind: SourceKind,
}

/// Browsers whose history file exists here. Only the default profile of each.
pub fn sources() -> Vec<Source> {
    use SourceKind::*;
    let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from) else {
        return Vec::new();
    };
    #[cfg(not(windows))]
    let (chromium, gecko, root) = (
        [
            ("Arc", "Arc/User Data/Default/History"),
            ("Google Chrome", "Google/Chrome/Default/History"),
            ("Helium", "net.imput.helium/Default/History"),
            ("Microsoft Edge", "Microsoft Edge/Default/History"),
            ("Brave", "BraveSoftware/Brave-Browser/Default/History"),
            ("Vivaldi", "Vivaldi/Default/History"),
        ],
        [("Firefox", "Firefox/Profiles"), ("Zen", "zen/Profiles")],
        home.join("Library/Application Support"),
    );
    // ponytail: Windows paths are from the browsers' docs, untested; Arc and Helium on
    // Windows are left out.
    #[cfg(windows)]
    let (chromium, gecko, root) = (
        [
            ("Google Chrome", "Local/Google/Chrome/User Data/Default/History"),
            ("Microsoft Edge", "Local/Microsoft/Edge/User Data/Default/History"),
            ("Brave", "Local/BraveSoftware/Brave-Browser/User Data/Default/History"),
            ("Vivaldi", "Local/Vivaldi/User Data/Default/History"),
        ],
        [("Firefox", "Roaming/Mozilla/Firefox/Profiles"), ("Zen", "Roaming/zen/Profiles")],
        home.join("AppData"),
    );
    let mut out: Vec<Source> = chromium
        .into_iter()
        .map(|(name, rel)| Source { name, history: root.join(rel), kind: Chromium })
        .filter(|s| s.history.is_file())
        .collect();
    out.extend(gecko.into_iter().filter_map(|(name, rel)| {
        Some(Source { name, history: newest_places(&root.join(rel))?, kind: Firefox })
    }));
    out
}

/// The profile whose places.sqlite was written last: the one in use.
fn newest_places(profiles: &Path) -> Option<PathBuf> {
    std::fs::read_dir(profiles)
        .ok()?
        .flatten()
        .map(|e| e.path().join("places.sqlite"))
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .max()
        .map(|(_, p)| p)
}

/// Newest first. Works while the browser runs: it reads a copy of the database.
pub fn read_history(source: &Source) -> Result<Vec<HistoryItem>, Error> {
    let dir = std::env::temp_dir().join(format!("baa-import-{}-{}", std::process::id(), crate::now()));
    std::fs::create_dir_all(&dir)?;
    let result = read_copy(source, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn read_copy(source: &Source, dir: &Path) -> Result<Vec<HistoryItem>, Error> {
    let db = dir.join("history.sqlite");
    std::fs::copy(&source.history, &db)?;
    let mut wal = source.history.clone().into_os_string();
    wal.push("-wal");
    if Path::new(&wal).is_file() {
        std::fs::copy(&wal, dir.join("history.sqlite-wal"))?;
    }
    // Opened writable, not read-only: SQLite must write the -shm to read a copied -wal.
    // It is our throwaway copy.
    let conn = Connection::open(&db)?;
    let sql = match source.kind {
        SourceKind::Chromium => format!(
            "SELECT url, title, visit_count, last_visit_time / 1000000 - {CHROMIUM_EPOCH} FROM urls \
             WHERE hidden = 0 AND visit_count > 0 AND (url LIKE 'http://%' OR url LIKE 'https://%') \
             ORDER BY last_visit_time DESC LIMIT {HISTORY_CAP}"
        ),
        SourceKind::Firefox => format!(
            "SELECT url, title, visit_count, last_visit_date / 1000000 FROM moz_places \
             WHERE hidden = 0 AND visit_count > 0 AND last_visit_date IS NOT NULL \
             AND (url LIKE 'http://%' OR url LIKE 'https://%') \
             ORDER BY last_visit_date DESC LIMIT {HISTORY_CAP}"
        ),
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| {
        Ok(HistoryItem {
            url: r.get(0)?,
            title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            visits: r.get(2)?,
            last_visit: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArcTab {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArcItem {
    Tab(ArcTab),
    /// Nested folders are flattened into this one; baa folders do not nest.
    Folder { name: String, tabs: Vec<ArcTab> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArcSpace {
    pub name: String,
    pub emoji: Option<String>,
    /// The pinned section only; Arc's unpinned tabs are ephemeral.
    pub pinned: Vec<ArcItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArcSidebar {
    pub spaces: Vec<ArcSpace>,
    pub favorites: Vec<ArcTab>,
}

/// Arc's default sidebar file (macOS).
pub fn arc_sidebar_path() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Arc/StorableSidebar.json");
    p.is_file().then_some(p)
}

/// Parses Arc's `StorableSidebar.json`. Its lists alternate `id, object`; items point
/// at their children by id.
pub fn read_arc_sidebar(path: &Path) -> Result<ArcSidebar, Error> {
    let root: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let containers = root["sidebar"]["containers"].as_array().ok_or("no sidebar.containers")?;
    let c = containers.iter().find(|c| c.get("spaces").is_some()).ok_or("no spaces container")?;
    let objects = |key: &str| c[key].as_array().into_iter().flatten().filter(|v| v.is_object());
    let items: HashMap<&str, &Value> = objects("items").filter_map(|i| Some((i["id"].as_str()?, i))).collect();
    let children = |id: &str| -> Vec<&Value> {
        let ids = items.get(id).and_then(|i| i["childrenIds"].as_array());
        ids.into_iter().flatten().filter_map(|id| items.get(id.as_str()?).copied()).collect()
    };

    let spaces = objects("spaces")
        .map(|s| {
            // containerIDs: ["pinned", <id>, "unpinned", <id>]
            let ids = s["containerIDs"].as_array().map(Vec::as_slice).unwrap_or_default();
            let pinned = ids
                .windows(2)
                .find(|w| w[0] == "pinned")
                .and_then(|w| w[1].as_str())
                .map(|id| children(id).into_iter().filter_map(|i| arc_item(i, &children)).collect())
                .unwrap_or_default();
            let icon = &s["customInfo"]["iconType"];
            let emoji = icon["emoji_v2"].as_str().map(String::from).or_else(|| {
                let c = char::from_u32(icon["emoji"].as_u64()?.try_into().ok()?)?;
                Some(c.to_string())
            });
            ArcSpace { name: s["title"].as_str().unwrap_or("Arc").into(), emoji, pinned }
        })
        .collect();

    // topAppsContainerIDs: [<profile>, <id>, ...]; one favourites container per Arc profile.
    let favorites = c["topAppsContainerIDs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .flat_map(&children)
        .flat_map(|i| arc_tabs(i, &children))
        .collect();
    Ok(ArcSidebar { spaces, favorites })
}

fn arc_item<'a>(item: &'a Value, children: &impl Fn(&str) -> Vec<&'a Value>) -> Option<ArcItem> {
    if item["data"].get("list").is_some() {
        let name = item["title"].as_str().unwrap_or("Folder").into();
        return Some(ArcItem::Folder { name, tabs: arc_tabs(item, children) });
    }
    arc_tab(item).map(ArcItem::Tab)
}

/// The tab itself, or every tab under a folder, depth first.
fn arc_tabs<'a>(item: &'a Value, children: &impl Fn(&str) -> Vec<&'a Value>) -> Vec<ArcTab> {
    match arc_tab(item) {
        Some(t) => vec![t],
        None => children(item["id"].as_str().unwrap_or_default())
            .into_iter()
            .flat_map(|c| arc_tabs(c, children))
            .collect(),
    }
}

fn arc_tab(item: &Value) -> Option<ArcTab> {
    let tab = item["data"].get("tab")?;
    let url = tab["savedURL"].as_str().filter(|u| is_web(u))?;
    // A renamed tab keeps its name in the item's own title.
    let title = item["title"].as_str().or(tab["savedTitle"].as_str()).unwrap_or_default();
    Some(ArcTab { url: url.into(), title: title.into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_sidebar_fixture() {
        let tab = |id: &str, parent: &str, url: &str, title: Option<&str>| {
            serde_json::json!({"id": id, "parentID": parent, "childrenIds": [], "title": title,
                "data": {"tab": {"savedURL": url, "savedTitle": format!("saved {id}")}}})
        };
        let list = |id: &str, parent: &str, title: &str, kids: &[&str]| {
            serde_json::json!({"id": id, "parentID": parent, "childrenIds": kids, "title": title,
                "data": {"list": {}}})
        };
        let container = |id: &str, kids: &[&str]| {
            serde_json::json!({"id": id, "parentID": null, "childrenIds": kids, "title": null,
                "data": {"itemContainer": {"containerType": {"spaceItems": {"_0": "S1"}}}}})
        };
        let items = [
            container("P1", &["F1", "T1"]),
            container("U1", &["T9"]),
            container("TOP", &["FAV"]),
            list("F1", "P1", "Work", &["T2", "F2"]),
            list("F2", "F1", "Nested", &["T3"]),
            tab("T1", "P1", "https://one", Some("Renamed")),
            tab("T2", "F1", "https://two", None),
            tab("T3", "F2", "https://three", None),
            tab("T9", "U1", "https://today", None),
            tab("FAV", "TOP", "https://fav", None),
        ];
        let items: Vec<Value> = items.into_iter().flat_map(|i| [i["id"].clone(), i]).collect();
        let json = serde_json::json!({"version": 1, "sidebar": {"containers": [
            {"global": {}},
            {"spaces": ["S1", {"id": "S1", "title": "Home", "profile": {"default": true},
                "containerIDs": ["pinned", "P1", "unpinned", "U1"],
                "customInfo": {"iconType": {"emoji": 128038, "emoji_v2": "🐦"}}},
                "S2", {"id": "S2", "title": "Dev", "containerIDs": [], "customInfo": {"iconType": {"icon": "planet"}}}],
             "topAppsContainerIDs": [{"default": true}, "TOP"],
             "items": items}
        ]}});
        let path = std::env::temp_dir().join(format!("baa-arc-{}.json", std::process::id()));
        std::fs::write(&path, json.to_string()).unwrap();
        let arc = read_arc_sidebar(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        let t = |url: &str, title: &str| ArcTab { url: url.into(), title: title.into() };
        assert_eq!(arc, ArcSidebar {
            spaces: vec![
                ArcSpace {
                    name: "Home".into(),
                    emoji: Some("🐦".into()),
                    pinned: vec![
                        ArcItem::Folder {
                            name: "Work".into(),
                            tabs: vec![t("https://two", "saved T2"), t("https://three", "saved T3")]
                        },
                        ArcItem::Tab(t("https://one", "Renamed")),
                    ],
                },
                ArcSpace { name: "Dev".into(), emoji: None, pinned: vec![] },
            ],
            favorites: vec![t("https://fav", "saved FAV")],
        });
    }

    #[test]
    fn chromium_history_copy() {
        let dir = std::env::temp_dir().join(format!("baa-chromium-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("History");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE urls (url TEXT, title TEXT, visit_count INTEGER, last_visit_time INTEGER, hidden INTEGER);
             INSERT INTO urls VALUES ('https://a', 'A', 3, 13000000000000000, 0),
               ('https://hidden', '', 1, 13000000000000000, 1),
               ('chrome://settings', '', 9, 13000000000000000, 0),
               ('https://never', '', 0, 13000000000000000, 0);",
        )
        .unwrap();
        // The connection stays open, as when the browser is running: rows live in the -wal.
        let got = read_history(&Source { name: "Test", history: path, kind: SourceKind::Chromium }).unwrap();
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(got, [HistoryItem {
            url: "https://a".into(),
            title: "A".into(),
            visits: 3,
            last_visit: 13_000_000_000 - CHROMIUM_EPOCH
        }]);
    }
}
