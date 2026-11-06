//! Chrome tab groups <-> sidebar folders (PLAN-m3 §2.4).

use super::*;

type Group = Option<(u64, String)>;

#[derive(Default)]
pub(crate) struct Groups {
    /// The last group sent to (or reported by) Chrome per tab.
    applied: HashMap<TabId, Group>,
    /// Folders born from a group an extension made, by that group's key.
    chrome_keys: HashMap<u64, FolderId>,
}

impl Groups {
    fn key(&self, folder: FolderId) -> u64 {
        self.chrome_keys.iter().find(|(_, f)| **f == folder).map_or(folder.0, |(k, _)| *k)
    }
}

impl<H: WebHost> Browser<H> {
    /// Tells each live page the folder it sits in, when that changed.
    pub(super) fn sync_groups(&mut self, cx: &mut Context<Self>) {
        let session = &self.session;
        let groups = &mut self.page_state.groups;
        groups.chrome_keys.retain(|_, f| session.folder(*f).is_some());
        groups.applied.retain(|id, _| self.pages.contains_key(id));
        for (id, (page, _)) in &self.pages {
            let folder = session.tab(*id).and_then(|t| t.folder).and_then(|f| session.folder(f));
            let want: Group = folder.map(|f| (groups.key(f.id), f.name.clone()));
            let had = groups.applied.get(id).cloned().flatten();
            if want != had {
                page.read(cx).set_group(want.as_ref().map(|(k, n)| (*k, n.as_str())));
                groups.applied.insert(*id, want);
            }
        }
    }

    pub(super) fn on_group_changed(&mut self, id: TabId, group: Option<u64>, title: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.session.tab(id) else { return };
        let space = tab.space;
        let groups = &mut self.page_state.groups;
        let applied = match group {
            None => {
                if tab.folder.is_some() {
                    self.session.move_tab(id, space, Zone::Pinned, usize::MAX);
                }
                None
            }
            Some(key) => {
                let known = groups.chrome_keys.get(&key).copied().or(Some(FolderId(key)));
                let folder = match known.filter(|f| self.session.folder(*f).is_some()) {
                    Some(f) => f,
                    None => {
                        let f = self.session.add_folder(space, if title.is_empty() { "Group" } else { title });
                        groups.chrome_keys.insert(key, f);
                        f
                    }
                };
                if self.session.tab(id).is_some_and(|t| t.folder != Some(folder)) {
                    self.session.move_to_folder(id, folder, usize::MAX);
                }
                let f = self.session.folder_mut(folder).expect("exists above");
                if !title.is_empty() {
                    f.name = title.into();
                }
                Some((key, f.name.clone()))
            }
        };
        // Chrome made this change: no set_group echoes back.
        groups.applied.insert(id, applied);
        self.sync(window, cx);
    }
}
