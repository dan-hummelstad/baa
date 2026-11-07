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
            let fetch = cx.background_spawn(async move { baa_model::live::fetch(&source) });
            cx.spawn_in(window, async move |this, cx| {
                let result = fetch.await;
                this.update_in(cx, |this, window, cx| {
                    match result {
                        Ok(items) => {
                            this.live_errors.remove(&id);
                            this.session.set_live_items(id, &items);
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

    /// What "New Live Folder from This Page" would make of the current tab.
    pub(crate) fn live_source_here(&self) -> Option<(String, LiveSource)> {
        LiveSource::from_page(&self.session.tab(self.active_tab()?)?.url)
    }
}
