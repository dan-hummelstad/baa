//! Spaces and folders: making, naming, colouring, deleting and stepping through them.

use super::*;

impl<H: WebHost> Browser<H> {
    pub(crate) fn activate_space(
        &mut self,
        id: SpaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.session.set_active_space(id);
        self.sync(window, cx);
    }

    pub(crate) fn new_space(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.session.spaces().len();
        let id = self
            .session
            .add_space(&format!("Space {}", n + 1), ICONS[n % ICONS.len()], None);
        if let Some(space) = self.session.space_mut(id) {
            let (top, bottom) = PALETTE[n % PALETTE.len()];
            (space.accent, space.accent_to) = (top, Some(bottom));
        }
        self.activate_space(id, window, cx);
        self.start_rename(window, cx);
    }

    pub(crate) fn new_incognito_space(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self
            .session
            .add_ephemeral_space("Incognito", self.host.ephemeral_profile());
        if let Some(space) = self.session.space_mut(id) {
            (space.accent, space.accent_to) = (INCOGNITO.0, Some(INCOGNITO.1));
        }
        self.activate_space(id, window, cx);
        self.open_bar(Mode::NewTab, window, cx);
    }

    pub(crate) fn delete_space(&mut self, id: SpaceId, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.spaces().len() < 2 {
            return;
        }
        let orphan = self.session.remove_space(id);
        // Pages must be gone before the profile directory is deleted.
        self.sync(window, cx);
        if let Some(profile) = orphan {
            self.host.delete_profile(profile);
        }
    }

    /// "🐑 Work" sets icon and name; plain "Work" keeps the icon.
    pub(crate) fn rename_space(&mut self, id: SpaceId, text: &str) {
        let Some(space) = self.session.space_mut(id) else {
            return;
        };
        let text = text.trim();
        let (icon, name) = match text.split_once(' ') {
            Some((first, rest)) if !first.chars().any(char::is_alphanumeric) => {
                (Some(first), rest.trim())
            }
            _ => (None, text),
        };
        if let Some(icon) = icon {
            space.icon = icon.into();
        }
        if !name.is_empty() {
            space.name = name.into();
        }
        self.save();
    }

    pub(crate) fn cycle_color_of(&mut self, id: SpaceId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self.session.space_mut(id) else {
            return;
        };
        let next = PALETTE
            .iter()
            .position(|(top, _)| *top == space.accent)
            .map_or(0, |i| i + 1)
            % PALETTE.len();
        let (top, bottom) = PALETTE[next];
        (space.accent, space.accent_to) = (top, Some(bottom));
        self.sync(window, cx);
    }

    pub(crate) fn swap_sidebar_side(&mut self, cx: &mut Context<Self>) {
        self.settings.sidebar_side = match self.settings.sidebar_side {
            SidebarSide::Left => SidebarSide::Right,
            SidebarSide::Right => SidebarSide::Left,
        };
        if let Some(Err(e)) = self.store.as_ref().map(|s| s.save_settings(&self.settings)) {
            log::error!("saving settings: {e}");
        }
        cx.notify();
    }

    /// Edits the Space's name where the sidebar shows it, all selected.
    pub(crate) fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.active_space().map(|s| s.name.clone()).unwrap_or_default();
        self.bar = None;
        self.find = None;
        self.renaming = Some(TextField::selected(name));
        self.renaming_folder = None;
        if !self.sidebar_open {
            self.reveal.set_target(1.0);
        }
        window.focus(&self.input_focus, cx);
        cx.notify();
    }

    /// A new folder in the active Space, holding `tab` if given, named in place.
    pub(crate) fn new_folder(&mut self, tab: Option<TabId>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self.session.active_space() else {
            return;
        };
        let id = self.session.add_folder(space, "New Folder");
        if let Some(tab) = tab {
            self.session.move_to_folder(tab, id, usize::MAX);
        }
        self.sync(window, cx);
        self.start_folder_rename(id, window, cx);
    }

    pub(crate) fn start_folder_rename(&mut self, id: FolderId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.session.folder(id).map(|f| f.name.clone()) else {
            return;
        };
        self.start_rename(window, cx);
        self.renaming = Some(TextField::selected(name));
        self.renaming_folder = Some(id);
    }

    pub(crate) fn set_folder_open(&mut self, id: FolderId, open: bool) {
        let Some(folder) = self.session.folder_mut(id) else {
            return;
        };
        let from = if folder.collapsed { 0.0 } else { 1.0 };
        folder.collapsed = !open;
        self.folder_motion
            .entry(id)
            .or_insert_with(|| Spring::new(from))
            .set_target(if open { 1.0 } else { 0.0 });
    }

    /// Keeps the name being typed (Enter, or a click anywhere else).
    pub(crate) fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = &self.renaming else {
            return;
        };
        let text = field.text.trim().to_string();
        match self.renaming_folder.and_then(|id| self.session.folder_mut(id)) {
            Some(folder) if !text.is_empty() => folder.name = text,
            Some(_) => {}
            None => {
                if let Some(id) = self.session.active_space() {
                    self.rename_space(id, &text);
                }
            }
        }
        self.save();
        self.close_overlays(window, cx);
    }

    pub(super) fn space_tab_ids(&self) -> Vec<TabId> {
        self.active_space()
            .map(|s| self.session.space_tabs(s.id).map(|t| t.id).collect())
            .unwrap_or_default()
    }

    pub(super) fn step_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let tabs = self.space_tab_ids();
        if tabs.is_empty() {
            return;
        }
        let i = self
            .active_tab()
            .and_then(|a| tabs.iter().position(|t| *t == a))
            .unwrap_or(0);
        let next = (i as isize + delta).rem_euclid(tabs.len() as isize) as usize;
        self.activate_tab(tabs[next], window, cx);
    }

    pub(crate) fn step_space(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let spaces: Vec<SpaceId> = self.session.spaces().iter().map(|s| s.id).collect();
        let Some(i) = spaces
            .iter()
            .position(|s| Some(*s) == self.session.active_space())
        else {
            return;
        };
        let next = (i as isize + delta).rem_euclid(spaces.len() as isize) as usize;
        self.activate_space(spaces[next], window, cx);
    }
}
