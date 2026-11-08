//! The sidebar's mini-player for media playing off screen, and Picture in Picture for
//! video that leaves the screen (PLAN-m4-features.md C). Media and muted state are Chrome's, never guessed.

use baa_web::{Media, MediaAction};

use super::*;

const MINI: motion::Key = ("mini-player", 0);
const MINI_H: f32 = 48.;

impl<H: WebHost> Browser<H> {
    pub(crate) fn on_media(&mut self, id: TabId, media: &Media, cx: &mut Context<Self>) {
        let latest = self.media.values().map(|(_, played)| *played).max().unwrap_or(0);
        let played = match self.media.get(&id) {
            _ if media.playing => latest + 1,
            Some((_, played)) => *played,
            None => 0,
        };
        self.media.insert(id, (media.clone(), played));
        self.retarget_mini_player();
        cx.notify();
    }

    /// The tab that played last and is not on screen, from any Space, while it is playing
    /// sound (or would be, but is muted). An incognito Space's media shows only in that Space.
    pub(crate) fn mini_player_tab(&self) -> Option<TabId> {
        let shown = self.shown_panes();
        let active = self.session.active_space();
        let private = |id: TabId| {
            let space = self.session.tab(id).map(|t| t.space);
            space != active && space.and_then(|s| self.session.space(s)).is_some_and(|s| s.ephemeral)
        };
        self.media
            .iter()
            .filter(|(id, (m, played))| {
                // Chrome's audible flag, so a silent autoplay video or a paused track shows nothing.
                let sounding = m.playing && (self.audible.contains(id) || self.muted.contains(id));
                *played > 0 && sounding && !shown.contains(id) && !private(**id)
            })
            .max_by_key(|(_, (_, played))| *played)
            .map(|(id, _)| *id)
    }

    pub(crate) fn retarget_mini_player(&mut self) {
        let tab = self.mini_player_tab();
        if tab.is_some() {
            self.mini_tab = tab;
        }
        self.motion.to(MINI, Feel::Glide, tab.is_some() as u8 as f32);
    }

    /// A playing video that leaves the screen (tab switch, Space switch, split change) keeps
    /// playing in Picture in Picture, and coming back closes it, as Arc does. Chrome's own auto
    /// PiP is not relied on: it only sees tab switches inside one of its windows (a Space is a
    /// window), and needs the tab recently audible and focused. Only PiP baa opened is closed.
    pub(crate) fn pip_on_screen_change(&mut self, before: &[TabId], cx: &mut Context<Self>) {
        let pages = &self.pages;
        self.pipped.retain(|id| pages.contains_key(id));
        for id in self.on_screen.iter().filter(|id| !before.contains(id)) {
            if let Some((page, _)) = self.pipped.remove(id).then(|| self.pages.get(id)).flatten() {
                page.read(cx).set_media_action(MediaAction::ExitPictureInPicture);
            }
        }
        if !self.settings.auto_pip {
            return;
        }
        for id in before.iter().filter(|id| !self.on_screen.contains(id)) {
            let pip = self
                .media
                .get(id)
                .is_some_and(|(m, _)| m.playing && m.actions.contains(&MediaAction::PictureInPicture));
            if let Some((page, _)) = self.pages.get(id).filter(|_| pip) {
                page.read(cx).set_media_action(MediaAction::PictureInPicture);
                self.pipped.insert(*id);
            }
        }
    }

    fn media_action(&mut self, id: TabId, action: MediaAction, cx: &mut Context<Self>) {
        if let Some((page, _)) = self.pages.get(&id) {
            page.read(cx).set_media_action(action);
        }
    }

    fn mini_button(
        &self,
        id: &'static str,
        name: &'static str,
        theme: &Theme,
        cx: &mut Context<Self>,
        click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        let key = (id, 0);
        let h = self.motion.get(key, Feel::Hover);
        let el = div()
            .id(id)
            .size(px(24.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(r(6.))
            .bg(fade(theme.glass_strong, h))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                click(this, cx);
            }))
            .child(icon(name, 14., mix(theme.ink_muted, theme.ink, h)));
        feel(el, key, cx)
    }

    /// Between the tab lists and the bottom bar; grows in and fades with a Glide spring.
    pub(crate) fn render_mini_player(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = self.motion.get(MINI, Feel::Glide);
        let id = self.mini_tab.filter(|_| t > 0.01)?;
        let (media, _) = self.media.get(&id)?;
        let title = match media.title.is_empty() {
            true => self.session.tab(id).map(|t| tab_title(t).to_string()).unwrap_or_default(),
            false => media.title.clone(),
        };
        let (playing, muted) = (media.playing, self.muted.contains(&id));
        let can = |a| media.actions.contains(&a);
        let toggle = if playing { MediaAction::Pause } else { MediaAction::Play };
        let h = self.motion.get(("mini-title", 0), Feel::Hover);
        // ponytail: no artwork. Its URL is the page's, and fetching it outside Chromium needs
        // the is_web and --proto checks; title only until Chrome hands over the image.
        let info = div()
            .id("mini-title")
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| this.activate_tab(id, window, cx)))
            .child(div().text_size(px(12.5)).text_color(mix(theme.ink_muted, theme.ink, 0.6 + 0.4 * h)).truncate().child(title))
            .when(!media.artist.is_empty(), |d| {
                d.child(div().text_size(px(11.)).text_color(theme.ink_muted).truncate().child(media.artist.clone()))
            });
        let row = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .h(px(MINI_H - 8.))
            .px(px(8.))
            .rounded(r(8.))
            .bg(theme.glass)
            .child(feel(info, ("mini-title", 0), cx))
            .when(can(toggle), |d| {
                d.child(self.mini_button("mini-play", if playing { "pause" } else { "play" }, theme, cx, move |this, cx| {
                    this.media_action(id, toggle, cx)
                }))
            })
            .when(can(MediaAction::Next), |d| {
                d.child(self.mini_button("mini-next", "skip-forward", theme, cx, move |this, cx| {
                    this.media_action(id, MediaAction::Next, cx)
                }))
            })
            .child(self.mini_button("mini-mute", if muted { "volume-x" } else { "volume-2" }, theme, cx, move |this, cx| {
                this.toggle_mute(id, cx)
            }));
        Some(
            div()
                .flex_none()
                .h(px(MINI_H * t.max(0.)))
                .opacity(t.clamp(0., 1.))
                .overflow_hidden()
                .pt(px(4.))
                .child(row)
                .into_any_element(),
        )
    }
}
