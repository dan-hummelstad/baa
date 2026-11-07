//! Extensions shared to every Space, and which Space a new install goes to.

use baa_model::ProfileId;
use baa_web::ExtensionChange;

use super::*;

/// Where the install prompt puts the extension being added.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum InstallTarget {
    /// The Space whose tab asked (Chrome always installs there).
    #[default]
    Here,
    Space(SpaceId),
    All,
}

/// An install accepted with a target other than [`InstallTarget::Here`], waiting for the
/// extension to show up in `profile`.
pub(crate) struct PendingInstall {
    profile: ProfileId,
    before: Vec<String>,
    target: InstallTarget,
}

// ponytail: Chrome does not say which extension the prompt installed, so the profile is
// polled for a new id; gives up after POLL_TRIES (a slow download is then left where it landed).
const POLL_EVERY: std::time::Duration = std::time::Duration::from_secs(1);
const POLL_TRIES: u32 = 120;

impl<H: WebHost> Browser<H> {
    /// Profiles of the Spaces that keep extensions (not incognito), each once.
    pub(crate) fn extension_profiles(&self) -> Vec<ProfileId> {
        let mut out = Vec::new();
        for s in self.session.spaces().iter().filter(|s| !s.ephemeral) {
            if !out.contains(&s.profile) {
                out.push(s.profile);
            }
        }
        out
    }

    fn installed(&self, profile: ProfileId) -> Vec<String> {
        self.host.extensions(profile).into_iter().map(|e| e.id).collect()
    }

    /// Runs [`Self::share_extensions`] when Spaces with new profiles appeared.
    pub(super) fn share_extensions_if_new_profiles(&mut self, cx: &mut Context<Self>) {
        let profiles = self.extension_profiles();
        if profiles != self.shared_to {
            self.shared_to = profiles;
            self.share_extensions(cx);
        }
    }

    /// Copies each shared extension into every Space's profile that lacks it.
    pub(crate) fn share_extensions(&mut self, cx: &mut Context<Self>) {
        if self.settings.shared_extensions.is_empty() {
            return;
        }
        let installed: Vec<(ProfileId, Vec<String>)> =
            self.extension_profiles().into_iter().map(|p| (p, self.installed(p))).collect();
        for id in self.settings.shared_extensions.clone() {
            // Nowhere has it (all copies removed or switched off): nothing to copy from.
            let Some(from) = installed.iter().find(|(_, ids)| ids.contains(&id)).map(|(p, _)| *p) else { continue };
            for (to, ids) in &installed {
                if !ids.contains(&id) {
                    self.copy_extension(*to, from, id.clone(), false, cx);
                }
            }
        }
    }

    /// Shares `id` (installed in `space`) to every Space, or stops sharing it. Stopping keeps
    /// the copies; each Space removes its own.
    pub(crate) fn set_extension_shared(&mut self, id: &str, shared: bool, cx: &mut Context<Self>) {
        self.settings.shared_extensions.retain(|e| e != id);
        if shared {
            self.settings.shared_extensions.push(id.into());
        }
        self.save_settings(cx);
        self.share_extensions(cx);
    }

    pub(crate) fn is_extension_shared(&self, id: &str) -> bool {
        self.settings.shared_extensions.iter().any(|e| e == id)
    }

    /// A shared extension removed anywhere goes from every Space, or sharing would bring it back.
    pub(crate) fn remove_shared_everywhere(&mut self, id: &str, except: ProfileId, cx: &mut Context<Self>) {
        self.settings.shared_extensions.retain(|e| e != id);
        self.save_settings(cx);
        for p in self.extension_profiles().into_iter().filter(|p| *p != except) {
            self.host.change_extension(p, id, ExtensionChange::Remove, cx, Box::new(|_, _| {}));
        }
    }

    /// `then_remove`: a move, `from` loses it once `to` has it.
    fn copy_extension(&mut self, to: ProfileId, from: ProfileId, id: String, then_remove: bool, cx: &mut Context<Self>) {
        if !self.copying.insert((to, id.clone())) {
            return;
        }
        let this = cx.weak_entity();
        self.host.copy_extension(
            to,
            from,
            &id.clone(),
            cx,
            Box::new(move |result, cx| {
                cx.defer(move |cx| {
                    this.update(cx, |this, cx| {
                        this.copying.remove(&(to, id.clone()));
                        match result {
                            Ok(_) if then_remove => {
                                this.host.change_extension(from, &id, ExtensionChange::Remove, cx, Box::new(|_, _| {}))
                            }
                            Ok(_) => {}
                            Err(e) => {
                                log::warn!("copying extension {id}: {e}");
                                this.show_toast(format!("Could not add the extension to a Space: {e}"), cx);
                            }
                        }
                        this.reread_extensions(cx);
                    })
                    .ok();
                })
            }),
        );
    }

    fn reread_extensions(&mut self, cx: &mut Context<Self>) {
        if let Some(active) = self.active_space() {
            self.extensions = self.host.extensions(active.profile);
        }
        match self.extensions_page.space {
            Some(space) => self.refresh_extensions_for(space, cx),
            None => cx.notify(),
        }
    }

    /// The install prompt was accepted in `space` for `target`; carried out once the
    /// extension shows up there.
    pub(crate) fn install_accepted(&mut self, space: SpaceId, target: InstallTarget, cx: &mut Context<Self>) {
        let Some(profile) = self.session.space(space).map(|s| s.profile) else { return };
        if target == InstallTarget::Here || self.session.space(space).is_some_and(|s| s.ephemeral) {
            return;
        }
        self.pending_install = Some(PendingInstall { profile, before: self.installed(profile), target });
        cx.spawn(async move |this, cx| {
            for _ in 0..POLL_TRIES {
                cx.background_executor().timer(POLL_EVERY).await;
                let done = this.update(cx, |this, cx| this.poll_install(cx)).unwrap_or(true);
                if done {
                    return;
                }
            }
            this.update(cx, |this, _| this.pending_install = None).ok();
        })
        .detach();
    }

    /// True once the pending install is carried out (or gone).
    fn poll_install(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(p) = &self.pending_install else { return true };
        let Some(id) = self.installed(p.profile).into_iter().find(|id| !p.before.contains(id)) else { return false };
        let (from, target) = (p.profile, p.target);
        self.pending_install = None;
        match target {
            InstallTarget::Here => {}
            InstallTarget::All => self.set_extension_shared(&id, true, cx),
            InstallTarget::Space(space) => {
                if let Some(to) = self.session.space(space).map(|s| s.profile).filter(|to| *to != from) {
                    self.copy_extension(to, from, id, true, cx);
                }
            }
        }
        true
    }

    /// The extensions that go with `space` if it is deleted: its profile's, unless another
    /// Space shares the profile, less the ones every Space has.
    pub(crate) fn extensions_lost_with(&self, space: SpaceId) -> Vec<String> {
        let Some(s) = self.session.space(space).filter(|s| !s.ephemeral) else { return Vec::new() };
        if self.session.spaces().iter().any(|o| o.id != space && o.profile == s.profile) {
            return Vec::new();
        }
        self.host
            .extensions(s.profile)
            .into_iter()
            .filter(|e| !self.is_extension_shared(&e.id))
            .map(|e| e.name)
            .collect()
    }
}
