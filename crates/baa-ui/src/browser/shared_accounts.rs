//! Accounts signed in to in every Space: after a sign-in, the ask whether to share it.

use std::sync::atomic::{AtomicU64, Ordering};

use baa_model::{Account, ProfileId};

use super::*;
use crate::prompts::{button, ACTIVATION_DELAY};

/// "Use this account in all Spaces?", after `profile` signed in to `account`.
pub(crate) struct AccountAsk {
    /// Element ids come from it, so a click aimed at one ask cannot land on the next.
    pub id: u64,
    pub profile: ProfileId,
    pub account: Account,
    pub shown_at: Instant,
}

static NEXT_ASK: AtomicU64 = AtomicU64::new(1);

impl<H: WebHost> Browser<H> {
    /// Sign-ins the engine saw. Asks about each that is not shared already, not kept to its
    /// Space by an earlier answer, and from a Space of this window's that is not incognito.
    pub(super) fn signed_in(&mut self, sign_ins: Vec<(ProfileId, Account)>, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        for (profile, account) in sign_ins {
            let regular = self.session.spaces().iter().any(|s| s.profile == profile && !s.ephemeral);
            let decided = self.settings.shared_accounts.contains(&account) || self.settings.kept_accounts.contains(&(profile, account));
            let asked = self.account_asks.iter().any(|a| a.profile == profile && a.account == account);
            if self.private || !regular || decided || asked {
                continue;
            }
            let id = NEXT_ASK.fetch_add(1, Ordering::Relaxed);
            self.account_asks.push(AccountAsk { id, profile, account, shown_at: now });
            if self.account_asks.len() == 1 {
                self.motion.from(("account-in", 0), Feel::Glide, 0.0, 1.0);
            }
        }
        cx.notify();
    }

    /// Answers the shown ask: share the account to every Space, or keep it to its own.
    /// Sharing is ignored for [`ACTIVATION_DELAY`] after the ask appears.
    pub(crate) fn answer_account(&mut self, id: u64, share: bool, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let Some(first) = self.account_asks.first().filter(|a| a.id == id) else { return };
        if share && now < first.shown_at + ACTIVATION_DELAY {
            return;
        }
        let ask = self.account_asks.remove(0);
        if let Some(next) = self.account_asks.first_mut() {
            next.shown_at = now;
            self.motion.from(("account-in", 0), Feel::Glide, 0.0, 1.0);
        }
        if share {
            self.settings.shared_accounts.push(ask.account);
            // Asks about the same account from other Spaces are answered by this one.
            self.account_asks.retain(|a| a.account != ask.account);
            self.save_settings(cx);
            self.host.share_accounts(&self.settings.shared_accounts, Some(ask.profile));
            self.show_toast(format!("{} is now signed in to in every Space", ask.account.name()), cx);
        } else {
            self.settings.kept_accounts.push((ask.profile, ask.account));
            self.save_settings(cx);
        }
    }

    /// Stops sharing `account`: each Space keeps the sign-in it has, which then goes its own way.
    pub(crate) fn unshare_account(&mut self, account: Account, cx: &mut Context<Self>) {
        self.settings.shared_accounts.retain(|a| *a != account);
        self.save_settings(cx);
        self.host.share_accounts(&self.settings.shared_accounts, None);
    }

    /// The first ask, as a banner over the page. Buttons only: no key answers it, so Enter
    /// aimed at the page cannot share an account.
    pub(crate) fn render_account_ask(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ask = self.account_asks.first()?;
        let space = self.session.spaces().iter().find(|s| s.profile == ask.profile && !s.ephemeral)?;
        let t = self.motion.get(("account-in", 0), Feel::Glide);
        let id = ask.id;
        let text = format!("You signed in to {} in \u{201C}{}\u{201D}. Use this account in all Spaces?", ask.account.name(), space.name);
        Some(
            div()
                .id(("account-ask", id))
                .occlude()
                .mt(px(10. - 14. * (1. - t)))
                .opacity(t.clamp(0., 1.))
                .max_w(relative(0.94))
                .pl(px(14.))
                .pr(px(8.))
                .py(px(8.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(r(12.))
                .bg(surface::bg())
                .border_1()
                .border_color(surface::border())
                .shadow(vec![shadow(gpui::rgba(0x0A08142E).into(), 8., 24.)])
                .child(icon("key-round", 16., theme.accent))
                .child(div().min_w_0().text_size(px(13.)).text_color(surface::ink()).child(SharedString::from(text)))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .gap(px(6.))
                        .child(
                            button(("account-keep", id), "Just This Space", false, theme, self, cx)
                                .on_click(cx.listener(move |this, _, _, cx| this.answer_account(id, false, cx))),
                        )
                        .child(
                            button(("account-share", id), "Use in All Spaces", true, theme, self, cx)
                                .on_click(cx.listener(move |this, _, _, cx| this.answer_account(id, true, cx))),
                        ),
                )
                .into_any_element(),
        )
    }
}
