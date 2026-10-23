//! Data model + persistence (PLAN.md §4). No gpui, no cef.
//!
//! [`Session`] is the in-memory truth the UI mutates; [`Store`] loads/saves it and
//! [`Settings`] to SQLite. Times are unix seconds passed in by the caller so the
//! archive policy is testable.

pub mod import;
mod settings;
mod store;

pub use settings::{parse_suggestions, ChromeStyle, SearchEngine, Settings, SidebarSide, SEARCH_ENGINES};
pub use store::{DownloadRecord, HistoryItem, Store};

use serde::{Deserialize, Serialize};

/// Identifies a storage profile = one CEF request context (PLAN.md §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfileId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpaceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TabId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FolderId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Zone {
    Pinned,
    Today,
    /// Shown at the top of every Space. The tab keeps the Space (and so the
    /// profile) it was favourited from.
    Favorite,
}

/// A collapsible group of pinned tabs in one Space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: FolderId,
    pub space: SpaceId,
    pub name: String,
    pub collapsed: bool,
}

/// User-visible storage profile ("Personal", "Work"). Several Spaces may share one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Space {
    pub id: SpaceId,
    pub name: String,
    /// Emoji or short glyph shown in the switcher.
    pub icon: String,
    /// 0xRRGGBB theme tint; `accent_to` makes it a gradient.
    pub accent: u32,
    pub accent_to: Option<u32>,
    pub profile: ProfileId,
    /// Incognito: never written to disk, profile comes from `WebHost::ephemeral_profile`.
    #[serde(skip)]
    pub ephemeral: bool,
    /// Overrides [`Settings::sidebar_side`] for this Space.
    pub sidebar_side: Option<SidebarSide>,
    pub active_tab: Option<TabId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub id: TabId,
    pub space: SpaceId,
    pub zone: Zone,
    /// Set only on pinned tabs.
    #[serde(default)]
    pub folder: Option<FolderId>,
    pub url: String,
    pub title: String,
    /// Favicon URL as reported by the page.
    pub favicon: Option<String>,
    /// Last time the tab was shown or left; drives auto-archive.
    pub last_active: u64,
    pub archived_at: Option<u64>,
}

// ponytail: closed-tab stack is in memory only (lost at quit; the archive still has them); 25 deep.
const CLOSED_CAP: usize = 25;
// ponytail: archive is trimmed to the newest 500; past that, old entries are gone for good.
const ARCHIVE_CAP: usize = 500;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    profiles: Vec<Profile>,
    spaces: Vec<Space>,
    /// Live tabs. Order within each list (see `same_list`) is the sidebar order.
    tabs: Vec<Tab>,
    /// In sidebar order; per Space they sit above the loose pinned tabs.
    folders: Vec<Folder>,
    /// Newest first.
    archive: Vec<Tab>,
    /// Reopen stack: (tab, its index within its (space, zone) list when closed).
    #[serde(skip)]
    closed: Vec<(Tab, usize)>,
    active_space: Option<SpaceId>,
    last_id: u64,
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Session {
    /// First-launch session: one Space with its own profile.
    pub fn first_run() -> Self {
        let mut s = Session::default();
        let space = s.add_space("Personal", "🐑", None);
        s.active_space = Some(space);
        s
    }

    fn alloc(&mut self) -> u64 {
        // Shared counter, starting at 1, so ids never repeat across kinds or deletes:
        // a reused ProfileId could reopen a cache dir whose delete is still queued.
        self.last_id += 1;
        self.last_id
    }

    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    pub fn add_profile(&mut self, name: &str) -> ProfileId {
        let id = ProfileId(self.alloc());
        self.profiles.push(Profile { id, name: name.into() });
        id
    }

    pub fn spaces(&self) -> &[Space] {
        &self.spaces
    }

    pub fn space(&self, id: SpaceId) -> Option<&Space> {
        self.spaces.iter().find(|s| s.id == id)
    }

    pub fn space_mut(&mut self, id: SpaceId) -> Option<&mut Space> {
        self.spaces.iter_mut().find(|s| s.id == id)
    }

    /// `profile: None` gives the Space a fresh profile named after it (PLAN.md §6 default).
    pub fn add_space(&mut self, name: &str, icon: &str, profile: Option<ProfileId>) -> SpaceId {
        let profile = profile.unwrap_or_else(|| self.add_profile(name));
        self.push_space(name, icon, profile, false)
    }

    /// Incognito Space on the host's in-memory profile; not persisted, its tabs never archived.
    pub fn add_ephemeral_space(&mut self, name: &str, profile: ProfileId) -> SpaceId {
        self.push_space(name, "🕶", profile, true)
    }

    fn push_space(&mut self, name: &str, icon: &str, profile: ProfileId, ephemeral: bool) -> SpaceId {
        let id = SpaceId(self.alloc());
        self.spaces.push(Space {
            id,
            name: name.into(),
            icon: icon.into(),
            accent: 0x6c9cff,
            accent_to: None,
            profile,
            ephemeral,
            sidebar_side: None,
            active_tab: None,
        });
        id
    }

    /// Drops the Space and its live tabs. Returns the profile if no other Space uses it;
    /// the caller passes that to `WebHost::delete_profile` after closing the pages.
    pub fn remove_space(&mut self, id: SpaceId) -> Option<ProfileId> {
        let i = self.spaces.iter().position(|s| s.id == id)?;
        let space = self.spaces.remove(i);
        self.tabs.retain(|t| t.space != id);
        self.folders.retain(|f| f.space != id);
        if self.active_space == Some(id) {
            self.active_space = self.spaces.get(i.saturating_sub(1)).map(|s| s.id);
        }
        if space.ephemeral || self.spaces.iter().any(|s| s.profile == space.profile) {
            return None;
        }
        self.profiles.retain(|p| p.id != space.profile);
        Some(space.profile)
    }

    pub fn move_space(&mut self, id: SpaceId, index: usize) {
        if let Some(i) = self.spaces.iter().position(|s| s.id == id) {
            let s = self.spaces.remove(i);
            self.spaces.insert(index.min(self.spaces.len()), s);
        }
    }

    pub fn active_space(&self) -> Option<SpaceId> {
        self.active_space
    }

    pub fn set_active_space(&mut self, id: SpaceId) {
        if self.space(id).is_some() {
            self.active_space = Some(id);
        }
    }

    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    /// For page-driven updates (url, title, favicon).
    pub fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// One sidebar zone of a Space, in display order, without the tabs in
    /// folders. `Zone::Favorite` lists every Space's favourites.
    pub fn tabs(&self, space: SpaceId, zone: Zone) -> impl Iterator<Item = &Tab> {
        self.tabs
            .iter()
            .filter(move |t| t.zone == zone && t.folder.is_none() && (zone == Zone::Favorite || t.space == space))
    }

    /// Every live tab of a Space, in sidebar order: folders, loose pinned, Today.
    /// Favourites are not part of any one Space.
    pub fn space_tabs(&self, space: SpaceId) -> impl Iterator<Item = &Tab> {
        self.folders(space)
            .flat_map(|f| self.folder_tabs(f.id))
            .chain(self.tabs(space, Zone::Pinned))
            .chain(self.tabs(space, Zone::Today))
    }

    pub fn folders(&self, space: SpaceId) -> impl Iterator<Item = &Folder> {
        self.folders.iter().filter(move |f| f.space == space)
    }

    pub fn folder(&self, id: FolderId) -> Option<&Folder> {
        self.folders.iter().find(|f| f.id == id)
    }

    pub fn folder_mut(&mut self, id: FolderId) -> Option<&mut Folder> {
        self.folders.iter_mut().find(|f| f.id == id)
    }

    pub fn folder_tabs(&self, folder: FolderId) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().filter(move |t| t.folder == Some(folder))
    }

    pub fn add_folder(&mut self, space: SpaceId, name: &str) -> FolderId {
        let id = FolderId(self.alloc());
        self.folders.push(Folder { id, space, name: name.into(), collapsed: false });
        id
    }

    /// Its tabs stay pinned, below the remaining folders.
    pub fn remove_folder(&mut self, id: FolderId) {
        self.folders.retain(|f| f.id != id);
        let tabs: Vec<TabId> = self.folder_tabs(id).map(|t| t.id).collect();
        for tab in tabs {
            if let Some((mut tab, _)) = self.take(tab) {
                tab.folder = None;
                self.insert(tab, usize::MAX);
            }
        }
    }

    /// Drag into a folder: the tab becomes pinned in the folder's Space.
    pub fn move_to_folder(&mut self, id: TabId, folder: FolderId, index: usize) {
        let Some(space) = self.folder(folder).map(|f| f.space) else { return };
        let Some((mut tab, old_index)) = self.take(id) else { return };
        let (from, old_zone) = (tab.space, tab.zone);
        tab.space = space;
        tab.zone = Zone::Pinned;
        tab.folder = Some(folder);
        self.insert(tab, index);
        if from != space {
            self.pick_neighbour(from, id, old_zone, old_index);
        }
    }

    /// Opens at the top of Today (Arc puts new tabs under the New Tab row) and activates it.
    pub fn open_tab(&mut self, space: SpaceId, url: &str, now: u64) -> TabId {
        let id = TabId(self.alloc());
        let tab = Tab {
            id,
            space,
            zone: Zone::Today,
            folder: None,
            url: url.into(),
            title: String::new(),
            favicon: None,
            last_active: now,
            archived_at: None,
        };
        self.insert(tab, 0);
        self.activate_tab(id, now);
        id
    }

    /// Each Arc Space becomes a new Space with its own profile, holding Arc's pinned tabs
    /// and folders; Arc favourites not already here become favourites of the first one.
    /// Returns how many tabs were added.
    pub fn import_arc(&mut self, arc: &import::ArcSidebar) -> usize {
        let before = self.tabs.len();
        let mut first = None;
        for sp in &arc.spaces {
            let space = self.add_space(&sp.name, sp.emoji.as_deref().unwrap_or("🐑"), None);
            first.get_or_insert(space);
            for item in &sp.pinned {
                match item {
                    import::ArcItem::Tab(t) => self.push_tab(space, Zone::Pinned, None, t),
                    import::ArcItem::Folder { name, tabs } => {
                        let folder = self.add_folder(space, name);
                        for t in tabs {
                            self.push_tab(space, Zone::Pinned, Some(folder), t);
                        }
                    }
                }
            }
        }
        if let Some(space) = first.or(self.active_space) {
            for t in &arc.favorites {
                if !self.tabs(space, Zone::Favorite).any(|f| f.url == t.url) {
                    self.push_tab(space, Zone::Favorite, None, t);
                }
            }
        }
        self.tabs.len() - before
    }

    /// Appends an inactive tab to the end of its list.
    fn push_tab(&mut self, space: SpaceId, zone: Zone, folder: Option<FolderId>, t: &import::ArcTab) {
        let id = TabId(self.alloc());
        let tab = Tab {
            id,
            space,
            zone,
            folder,
            url: t.url.clone(),
            title: t.title.clone(),
            favicon: None,
            last_active: now(),
            archived_at: None,
        };
        self.insert(tab, usize::MAX);
    }

    /// Inserts at `index` within the tab's list, clamped to its end.
    fn insert(&mut self, tab: Tab, index: usize) {
        let same: Vec<usize> = (0..self.tabs.len()).filter(|&i| same_list(&self.tabs[i], &tab)).collect();
        let at = same.get(index).copied().or(same.last().map(|i| i + 1)).unwrap_or(self.tabs.len());
        self.tabs.insert(at, tab);
    }

    /// Removes a live tab; returns it and its index within its list.
    fn take(&mut self, id: TabId) -> Option<(Tab, usize)> {
        let i = self.tabs.iter().position(|t| t.id == id)?;
        let tab = self.tabs.remove(i);
        let index = self.tabs[..i].iter().filter(|t| same_list(t, &tab)).count();
        Some((tab, index))
    }

    /// Drag: reorder, change zone (pin/unpin/favourite), move to another Space,
    /// or out of a folder. A favourite keeps its own Space.
    pub fn move_tab(&mut self, id: TabId, space: SpaceId, zone: Zone, index: usize) {
        let Some((mut tab, old_index)) = self.take(id) else { return };
        let (from, old_zone) = (tab.space, tab.zone);
        let space = if zone == Zone::Favorite { from } else { space };
        tab.space = space;
        tab.zone = zone;
        tab.folder = None;
        self.insert(tab, index);
        if from != space {
            self.pick_neighbour(from, id, old_zone, old_index);
        }
    }

    /// ⌘D: Today → bottom of Pinned, Pinned → top of Today.
    pub fn toggle_pin(&mut self, id: TabId) {
        let Some(tab) = self.tab(id) else { return };
        let (space, zone) = (tab.space, tab.zone);
        match zone {
            Zone::Today => self.move_tab(id, space, Zone::Pinned, usize::MAX),
            Zone::Pinned => self.move_tab(id, space, Zone::Today, 0),
            Zone::Favorite => {}
        }
    }

    /// Shows a tab: makes it and its Space active, and stamps both it and the tab it
    /// replaces so a tab is never archived right after you leave it.
    /// A favourite shows in the active Space instead of switching to its own.
    pub fn activate_tab(&mut self, id: TabId, now: u64) {
        let Some(tab) = self.tab(id) else { return };
        let space = match tab.zone {
            Zone::Favorite => self.active_space.unwrap_or(tab.space),
            _ => tab.space,
        };
        let prev = self.space(space).and_then(|s| s.active_tab);
        for t in self.tabs.iter_mut().filter(|t| t.id == id || Some(t.id) == prev) {
            t.last_active = now;
        }
        if let Some(s) = self.space_mut(space) {
            s.active_tab = Some(id);
        }
        self.active_space = Some(space);
    }

    /// If `id` (already gone from `space`) was its active tab, hand that to the tab now at
    /// `index` in `zone` (the one below it), else the one above, else any tab left in the Space.
    fn pick_neighbour(&mut self, space: SpaceId, id: TabId, zone: Zone, index: usize) {
        if self.space(space).and_then(|s| s.active_tab) != Some(id) {
            return;
        }
        let next = self
            .tabs(space, zone)
            .nth(index)
            .or_else(|| self.tabs(space, zone).nth(index.checked_sub(1)?))
            .or_else(|| self.space_tabs(space).next())
            .map(|t| t.id);
        if let Some(s) = self.space_mut(space) {
            s.active_tab = next;
        }
    }

    /// ⌘W. Pushes onto the reopen stack and, unless incognito, the archive.
    /// A favourite stays; it is only put away, like Arc's.
    pub fn close_tab(&mut self, id: TabId, now: u64) {
        if self.tab(id).is_some_and(|t| t.zone == Zone::Favorite) {
            for i in 0..self.spaces.len() {
                if self.spaces[i].active_tab == Some(id) {
                    let next = self.space_tabs(self.spaces[i].id).next().map(|t| t.id);
                    self.spaces[i].active_tab = next;
                }
            }
            return;
        }
        let Some((tab, index)) = self.take(id) else { return };
        self.pick_neighbour(tab.space, id, tab.zone, index);
        if !self.space(tab.space).is_some_and(|s| s.ephemeral) {
            self.push_archive(Tab { archived_at: Some(now), ..tab.clone() });
        }
        self.closed.push((tab, index));
        if self.closed.len() > CLOSED_CAP {
            self.closed.remove(0);
        }
    }

    /// ⌘⇧T. Back to its old Space and position.
    pub fn reopen_closed(&mut self, now: u64) -> Option<TabId> {
        let (tab, index) = self.closed.pop()?;
        self.archive.retain(|t| t.id != tab.id);
        self.revive(tab, index, now)
    }

    /// Puts a closed/archived tab back live and shows it; its Space, or the active one if that is gone.
    fn revive(&mut self, mut tab: Tab, index: usize, now: u64) -> Option<TabId> {
        if self.space(tab.space).is_none() {
            tab.space = self.active_space?;
        }
        if tab.folder.is_some_and(|f| self.folder(f).is_none_or(|f| f.space != tab.space)) {
            tab.folder = None;
        }
        tab.archived_at = None;
        let id = tab.id;
        self.insert(tab, index);
        self.activate_tab(id, now);
        Some(id)
    }

    /// Newest first.
    pub fn archived(&self) -> &[Tab] {
        &self.archive
    }

    fn push_archive(&mut self, tab: Tab) {
        self.archive.insert(0, tab);
        self.archive.truncate(ARCHIVE_CAP);
    }

    /// Auto-archive (PLAN.md §3): Today tabs idle for `after_hours` move to the archive.
    /// Never touches pinned tabs, a Space's active tab, or incognito Spaces. `after_hours == 0`
    /// disables it. Returns the archived ids so the UI can drop their pages.
    pub fn archive_expired(&mut self, now: u64, after_hours: u32) -> Vec<TabId> {
        if after_hours == 0 {
            return Vec::new();
        }
        let cutoff = now.saturating_sub(u64::from(after_hours) * 3600);
        let keep: Vec<TabId> = self.spaces.iter().filter_map(|s| s.active_tab).collect();
        let ephemeral: Vec<SpaceId> = self.spaces.iter().filter(|s| s.ephemeral).map(|s| s.id).collect();
        let (expired, live): (Vec<Tab>, Vec<Tab>) = std::mem::take(&mut self.tabs).into_iter().partition(|t| {
            t.zone == Zone::Today && t.last_active < cutoff && !keep.contains(&t.id) && !ephemeral.contains(&t.space)
        });
        self.tabs = live;
        let ids = expired.iter().map(|t| t.id).collect();
        for t in expired {
            self.push_archive(Tab { archived_at: Some(now), ..t });
        }
        ids
    }

    /// Back to the top of Today.
    pub fn restore_archived(&mut self, id: TabId, now: u64) -> Option<TabId> {
        let i = self.archive.iter().position(|t| t.id == id)?;
        let mut tab = self.archive.remove(i);
        self.closed.retain(|(t, _)| t.id != id);
        tab.zone = Zone::Today;
        tab.folder = None;
        self.revive(tab, 0, now)
    }
}

/// Tabs share an ordered list: all favourites, or one Space's folder, loose pinned or Today tabs.
fn same_list(a: &Tab, b: &Tab) -> bool {
    a.zone == b.zone && (a.zone == Zone::Favorite || (a.space == b.space && a.folder == b.folder))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(s: &Session, space: SpaceId, zone: Zone) -> Vec<String> {
        s.tabs(space, zone).map(|t| t.url.clone()).collect()
    }

    #[test]
    fn folders_hold_pinned_tabs_and_give_them_back() {
        let mut s = Session::first_run();
        let sp = s.active_space().unwrap();
        let a = s.open_tab(sp, "a", 0);
        let b = s.open_tab(sp, "b", 0);
        let f = s.add_folder(sp, "Reading");
        s.move_to_folder(a, f, usize::MAX);
        s.move_to_folder(b, f, 0);
        assert_eq!(s.folder_tabs(f).map(|t| t.url.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(s.tab(a).unwrap().zone, Zone::Pinned, "a folder pins what goes in");
        assert!(urls(&s, sp, Zone::Pinned).is_empty() && urls(&s, sp, Zone::Today).is_empty());
        assert_eq!(s.space_tabs(sp).count(), 2);
        s.move_tab(b, sp, Zone::Today, 0);
        assert_eq!(s.tab(b).unwrap().folder, None);
        s.remove_folder(f);
        assert_eq!(urls(&s, sp, Zone::Pinned), ["a"]);
    }

    #[test]
    fn favorites_show_in_every_space_and_only_put_away() {
        let mut s = Session::first_run();
        let sp = s.active_space().unwrap();
        let work = s.add_space("Work", "💼", None);
        let fav = s.open_tab(sp, "fav", 0);
        let other = s.open_tab(sp, "other", 0);
        s.move_tab(fav, work, Zone::Favorite, 0);
        assert_eq!(s.tab(fav).unwrap().space, sp, "keeps its profile's Space");
        assert_eq!(urls(&s, work, Zone::Favorite), ["fav"]);
        s.set_active_space(work);
        s.activate_tab(fav, 1);
        assert_eq!(s.active_space(), Some(work), "opens in the Space you are in");
        assert_eq!(s.space(work).unwrap().active_tab, Some(fav));
        s.close_tab(fav, 2);
        assert!(s.tab(fav).is_some());
        assert_eq!(s.space(work).unwrap().active_tab, None);
        s.activate_tab(other, 3);
        assert_eq!(s.active_space(), Some(sp));
    }

    #[test]
    fn open_order_pin_and_move() {
        let mut s = Session::first_run();
        let sp = s.active_space().unwrap();
        let a = s.open_tab(sp, "a", 0);
        let b = s.open_tab(sp, "b", 0);
        let c = s.open_tab(sp, "c", 0);
        assert_eq!(urls(&s, sp, Zone::Today), ["c", "b", "a"]);
        s.toggle_pin(a);
        s.toggle_pin(b);
        assert_eq!(urls(&s, sp, Zone::Pinned), ["a", "b"]);
        s.move_tab(b, sp, Zone::Pinned, 0);
        assert_eq!(urls(&s, sp, Zone::Pinned), ["b", "a"]);
        let work = s.add_space("Work", "💼", None);
        s.move_tab(a, work, Zone::Today, 3);
        assert_eq!(urls(&s, work, Zone::Today), ["a"]);
        assert_eq!(s.profiles().len(), 2);
        s.move_tab(c, work, Zone::Today, 0);
        assert_eq!(s.space(sp).unwrap().active_tab, Some(b), "active tab moved away; falls back to what is left");
    }

    #[test]
    fn close_and_reopen_restores_position_and_active() {
        let mut s = Session::first_run();
        let sp = s.active_space().unwrap();
        s.open_tab(sp, "a", 0);
        let b = s.open_tab(sp, "b", 0);
        s.open_tab(sp, "c", 0);
        s.activate_tab(b, 1);
        s.close_tab(b, 2);
        assert_eq!(urls(&s, sp, Zone::Today), ["c", "a"]);
        assert_eq!(s.tab(s.space(sp).unwrap().active_tab.unwrap()).unwrap().url, "a");
        assert_eq!(s.archived()[0].id, b);
        assert_eq!(s.reopen_closed(3), Some(b));
        assert_eq!(urls(&s, sp, Zone::Today), ["c", "b", "a"]);
        assert!(s.archived().is_empty());
        assert_eq!(s.space(sp).unwrap().active_tab, Some(b));
    }

    #[test]
    fn auto_archive_skips_pinned_active_and_fresh() {
        let mut s = Session::first_run();
        let sp = s.active_space().unwrap();
        let old = s.open_tab(sp, "old", 0);
        let pinned = s.open_tab(sp, "pinned", 0);
        s.toggle_pin(pinned);
        let fresh = s.open_tab(sp, "fresh", 0);
        s.activate_tab(fresh, 13 * 3600);
        let active = s.open_tab(sp, "active", 13 * 3600);
        let archived = s.archive_expired(13 * 3600, 12);
        assert_eq!(archived, [old]);
        assert!(s.tab(active).is_some() && s.tab(pinned).is_some() && s.tab(fresh).is_some());
        assert_eq!(s.restore_archived(old, 20 * 3600), Some(old));
        assert_eq!(s.tabs(sp, Zone::Today).next().unwrap().id, old);
    }

    #[test]
    fn import_arc_adds_spaces_folders_and_new_favorites() {
        use import::{ArcItem, ArcSidebar, ArcSpace, ArcTab};
        let t = |u: &str| ArcTab { url: u.into(), title: u.to_uppercase() };
        let mut s = Session::first_run();
        let home = s.active_space().unwrap();
        let fav = s.open_tab(home, "https://fav", 0);
        s.move_tab(fav, home, Zone::Favorite, 0);
        let arc = ArcSidebar {
            spaces: vec![
                ArcSpace {
                    name: "Work".into(),
                    emoji: Some("🐦".into()),
                    pinned: vec![
                        ArcItem::Tab(t("https://a")),
                        ArcItem::Folder { name: "Docs".into(), tabs: vec![t("https://b"), t("https://c")] },
                    ],
                },
                ArcSpace { name: "Dev".into(), emoji: None, pinned: vec![] },
            ],
            favorites: vec![t("https://fav"), t("https://new")],
        };
        assert_eq!(s.import_arc(&arc), 4);
        let work = s.spaces()[1].clone();
        assert_eq!((work.name.as_str(), work.icon.as_str()), ("Work", "🐦"));
        assert_eq!(s.spaces().len(), 3);
        assert_eq!(s.profiles().len(), 3, "each imported Space gets its own profile");
        assert_eq!(urls(&s, work.id, Zone::Pinned), ["https://a"]);
        assert_eq!(s.tabs(work.id, Zone::Pinned).next().unwrap().title, "HTTPS://A");
        let folder = s.folders(work.id).next().unwrap();
        assert_eq!(folder.name, "Docs");
        assert_eq!(s.folder_tabs(folder.id).map(|t| t.url.as_str()).collect::<Vec<_>>(), ["https://b", "https://c"]);
        assert_eq!(urls(&s, home, Zone::Favorite), ["https://fav", "https://new"]);
        assert_eq!(s.active_space(), Some(home), "importing does not switch Space");
    }

    #[test]
    fn remove_space_returns_orphaned_profile_only() {
        let mut s = Session::first_run();
        let personal = s.active_space().unwrap();
        let profile = s.space(personal).unwrap().profile;
        let shared = s.add_space("Side", "x", Some(profile));
        let incog = s.add_ephemeral_space("Incognito", ProfileId(u64::MAX));
        assert_eq!(s.remove_space(incog), None);
        assert_eq!(s.remove_space(shared), None);
        assert_eq!(s.remove_space(personal), Some(profile));
        assert!(s.profiles().is_empty());
    }
}
