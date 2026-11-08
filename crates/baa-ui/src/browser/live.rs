//! Live folders: making them and keeping their tabs current.

use baa_model::LiveSource;

use super::*;

// ponytail: one fixed interval for every live folder; per-folder schedules if a source needs it.
const REFRESH_EVERY: std::time::Duration = std::time::Duration::from_secs(10 * 60);

impl<H: WebHost> Browser<H> {
    pub(super) fn start_live_refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| loop {
            if this.update_in(cx, |this, window, cx| this.refresh_live(None, window, cx)).is_err() {
                break;
            }
            cx.background_executor().timer(REFRESH_EVERY).await;
        })
        .detach();
    }

    /// Fetches `only`, or every live folder, off the UI thread.
    pub(crate) fn refresh_live(&mut self, only: Option<FolderId>, window: &mut Window, cx: &mut Context<Self>) {
        let jobs: Vec<(FolderId, LiveSource)> = self
            .session
            .spaces()
            .iter()
            .flat_map(|s| self.session.folders(s.id))
            .filter(|f| only.is_none_or(|id| id == f.id))
            .filter_map(|f| Some((f.id, f.live.clone()?)))
            .collect();
        for (id, source) in jobs {
            let asked = source.clone();
            let fetch = cx.background_spawn(async move { baa_model::live::fetch(&source) });
            cx.spawn_in(window, async move |this, cx| {
                let result = fetch.await;
                this.update_in(cx, |this, window, cx| {
                    // The filters changed while it ran; the fetch they started wins.
                    if this.session.folder(id).and_then(|f| f.live.as_ref()) != Some(&asked) {
                        return;
                    }
                    match result {
                        Ok(items) => {
                            this.live_errors.remove(&id);
                            this.live_fetched.insert(id, now());
                            this.live_items.insert(id, items);
                            this.show_live_items(id);
                            this.save();
                        }
                        Err(e) => {
                            log::warn!("live folder {}: {e}", id.0);
                            this.live_errors.insert(id, e);
                        }
                    }
                    this.sync(window, cx);
                })
            })
            .detach();
        }
    }

    /// A live folder in the active Space, filled as soon as its source answers.
    pub(crate) fn new_live_folder(&mut self, name: &str, source: LiveSource, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self.session.active_space() else {
            return;
        };
        let id = self.session.add_live_folder(space, name, source);
        self.sync(window, cx);
        self.refresh_live(Some(id), window, cx);
    }

    /// The folder's tabs from its last fetch, less its hidden repos.
    pub(super) fn show_live_items(&mut self, id: FolderId) {
        let Some(source) = self.session.folder(id).and_then(|f| f.live.clone()) else { return };
        let items = source.visible(self.live_items.get(&id).map_or(&[], Vec::as_slice));
        self.session.set_live_items(id, &items);
    }

    /// Repos the last fetch saw, and hidden ones, by name.
    pub(crate) fn live_repos(&self, id: FolderId) -> Vec<String> {
        let mut repos: Vec<String> =
            self.live_items.get(&id).into_iter().flatten().filter_map(|i| baa_model::live::repo_of(&i.url)).collect();
        if let Some(Some(LiveSource::GitHub(gh))) = self.session.folder(id).map(|f| &f.live) {
            repos.extend(gh.hidden_repos.iter().cloned());
        }
        repos.sort_by_key(|r| r.to_lowercase());
        repos.dedup();
        repos
    }

    pub(crate) fn toggle_live_filter(
        &mut self,
        id: FolderId,
        filter: baa_model::live::GitHubFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(Some(LiveSource::GitHub(gh))) = self.session.folder_mut(id).map(|f| &mut f.live) else { return };
        if !gh.toggle(filter) {
            return self.show_toast("A live folder needs at least one filter", cx);
        }
        self.save();
        self.refresh_live(Some(id), window, cx);
        cx.notify();
    }

    pub(crate) fn set_live_repo_hidden(&mut self, id: FolderId, repo: &str, hidden: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Some(LiveSource::GitHub(gh))) = self.session.folder_mut(id).map(|f| &mut f.live) else { return };
        gh.set_repo_hidden(repo, hidden);
        self.show_live_items(id);
        self.sync(window, cx);
    }

    /// What "New Live Folder from This Page" would make of the current tab.
    pub(crate) fn live_source_here(&self) -> Option<(String, LiveSource)> {
        LiveSource::from_page(&self.session.tab(self.active_tab()?)?.url)
    }
}
