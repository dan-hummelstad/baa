//! Mail in the shell (PLAN-mail.md §2, §5): the process's `Mail` and its events, what a window
//! holds of mail, and the nav, view and door the shell draws. The three renderers are
//! placeholders that lane 5 (WP-H to WP-K) fills.

use std::sync::Arc;

use baa_mail::{Event, Mail, MailSpace};
use baa_model::{Pane, Showing, SpaceId};
use baa_web::WebHost;
use futures::StreamExt;
use gpui::{div, prelude::*, px, AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Subscription, Window};

use crate::{
    actions::{DeleteMessage, ForwardMessage, NewMessage, Reply, ReplyAll, SearchMail, SendMessage},
    browser::Browser,
    library::Section,
    theme::Theme,
};

/// The process's one `Mail` and where its events arrive on the UI thread; set once by
/// [`start_mail`].
pub(crate) struct MailHub {
    mail: Arc<Mail>,
    events: Entity<MailEvents>,
}

impl gpui::Global for MailHub {}

pub(crate) struct MailEvents;

impl EventEmitter<Event> for MailEvents {}

/// Opens the process's `Mail` and gives it to every browser window. Call once at start, before
/// the first window. `open` gets the `notify` closure to pass to `Mail::open`; it may be called
/// on any thread, and each `Event` reaches every window with mail on the UI thread. An error
/// from `open` leaves mail off for this run.
pub fn start_mail(
    cx: &mut App,
    open: impl FnOnce(Box<dyn Fn(Event) + Send + Sync>) -> baa_mail::Result<Mail>,
) -> baa_mail::Result<()> {
    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    let mail = open(Box::new(move |event| {
        let _ = tx.unbounded_send(event);
    }))?;
    let events = cx.new(|_| MailEvents);
    let to = events.downgrade();
    cx.spawn(async move |cx| {
        while let Some(event) = rx.next().await {
            if to.update(cx, |_, cx| cx.emit(event)).is_err() {
                break;
            }
        }
    })
    .detach();
    cx.set_global(MailHub { mail: Arc::new(mail), events });
    Ok(())
}

/// Mail in one window: `Browser.mail`, `Some` only with mail on in a window that is not private.
pub(crate) struct MailUi<H: WebHost> {
    /// The process's one `Mail`, shared by its windows.
    pub(crate) mail: Arc<Mail>,
    /// The message and editor pages on screen.
    #[allow(dead_code)] // lane 5's reader and composer (WP-I, WP-J) keep their pages here
    pub(crate) documents: Vec<Entity<H::Page>>,
    /// The mail view's keyboard focus: its `Mail` key context is active while this has it.
    pub(crate) focus: FocusHandle,
    _events: Subscription,
}

// The mail view's keys (`Mail` key context); lane 5 fills them.
impl<H: WebHost> MailUi<H> {
    pub(crate) fn new_message(&mut self, _window: &mut Window, _cx: &mut Context<Browser<H>>) {}
    pub(crate) fn reply(&mut self, _all: bool, _window: &mut Window, _cx: &mut Context<Browser<H>>) {}
    pub(crate) fn forward(&mut self, _window: &mut Window, _cx: &mut Context<Browser<H>>) {}
    pub(crate) fn send(&mut self, _window: &mut Window, _cx: &mut Context<Browser<H>>) {}
    pub(crate) fn delete(&mut self, _window: &mut Window, _cx: &mut Context<Browser<H>>) {}
    pub(crate) fn search(&mut self, _window: &mut Window, _cx: &mut Context<Browser<H>>) {}
}

impl<H: WebHost> Browser<H> {
    /// Makes `mail` match the settings: on with mail enabled in a window that is not private
    /// once [`start_mail`] has run, else off, and with mail off no Space shows it.
    pub(crate) fn sync_mail(&mut self, cx: &mut Context<Self>) {
        let hub = cx.try_global::<MailHub>().filter(|_| self.settings.mail.enabled && !self.private);
        match hub {
            Some(_) if self.mail.is_some() => {}
            Some(hub) => {
                let (mail, events) = (hub.mail.clone(), hub.events.clone());
                self.mail = Some(MailUi {
                    mail,
                    documents: Vec::new(),
                    focus: cx.focus_handle(),
                    _events: cx.subscribe(&events, |this, _, event, cx| this.on_mail_event(event, cx)),
                });
            }
            None => {
                self.mail = None;
                self.session.forget_mail();
            }
        }
    }

    // ponytail: every event redraws the window; lane 5 narrows it to the views it touches.
    fn on_mail_event(&mut self, _event: &Event, cx: &mut Context<Self>) {
        cx.notify();
    }

    /// The one way into `Mail` for a Space: `None` with mail off, in a private window and
    /// for an incognito Space.
    pub(crate) fn mail_of(&self, space: SpaceId) -> Option<(&Arc<Mail>, MailSpace)> {
        let mail = self.mail.as_ref()?;
        Some((&mail.mail, MailSpace::of(self.session.space(space)?)?))
    }

    /// [`Self::mail_of`] the active Space.
    pub(crate) fn mail_space(&self) -> Option<(&Arc<Mail>, MailSpace)> {
        self.mail_of(self.session.active_space()?)
    }

    /// The active Space can have mail: the switcher, the Mailbox door and ⌘⇧M are there.
    pub(crate) fn mail_here(&self) -> bool {
        self.mail_space().is_some()
    }

    /// The active Space has mail in front: the sidebar is the mail nav, and the page area
    /// draws the mail view where the page was.
    pub(crate) fn mail_in_front(&self) -> bool {
        self.front() == Some(Pane::Mail)
    }

    /// Mail (`true`) or the browser in front, in the active Space. ⌘⇧M, the switcher and the door.
    pub(crate) fn show_mail(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self.session.active_space() else { return };
        if on && !self.mail_here() {
            return;
        }
        self.session.set_showing(space, if on { Showing::Mail } else { Showing::Browser });
        self.sync(window, cx);
    }

    /// The Archive (Library) at its Mailbox door.
    pub(crate) fn open_mailbox(&mut self, cx: &mut Context<Self>) {
        if self.mail_here() {
            self.open_library(Section::Mailbox, cx);
        }
    }

    /// The door's Open Mailbox: the Library closes, the sidebar is the Space's again, mail in front.
    pub(crate) fn open_mail_from_door(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_library(window, cx);
        self.show_mail(true, window, cx);
    }

    /// The mail nav, in the sidebar's place above the bottom bar while mail is in front.
    /// `other_spaces` are the other Spaces with accounts of their own and their unread counts.
    pub(crate) fn render_nav(&self, theme: &Theme, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        let other_spaces = self.mail_space().map(|(mail, space)| mail.other_spaces(space)).unwrap_or_default();
        placeholder("Mail navigation", theme)
            .debug_selector(|| "mail-nav".into())
            .child(format!("{} other Spaces with mail", other_spaces.len()))
            .into_any_element()
    }

    /// The mail view: the page area's content (or a split pane's) while mail shows.
    pub(crate) fn render_view(&self, theme: &Theme, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        placeholder("Mail", theme).into_any_element()
    }

    /// The Archive's Mailbox section: the door into mail.
    pub(crate) fn render_door(&self, theme: &Theme, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        placeholder("Mailbox", theme)
            .child(
                div()
                    .id("open-mailbox")
                    .px(px(12.))
                    .py(px(6.))
                    .rounded(px(8.))
                    .bg(theme.accent)
                    .text_color(gpui::white())
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| this.open_mail_from_door(window, cx)))
                    .child("Open Mailbox"),
            )
            .into_any_element()
    }

    /// The mail view with its key context and focus, and the mail actions bound to `MailUi`.
    pub(crate) fn mail_view(&self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(mail) = &self.mail else { return div().into_any_element() };
        macro_rules! hook {
            ($action:ty, $m:ident, $w:ident, $cx:ident => $call:expr) => {
                cx.listener(|this, _: &$action, $w, $cx| {
                    if let Some($m) = &mut this.mail {
                        $call
                    }
                })
            };
        }
        div()
            .id("mail-view")
            .debug_selector(|| "mail-view".into())
            .key_context("Mail")
            .track_focus(&mail.focus)
            .size_full()
            .on_action(hook!(NewMessage, m, w, cx => m.new_message(w, cx)))
            .on_action(hook!(Reply, m, w, cx => m.reply(false, w, cx)))
            .on_action(hook!(ReplyAll, m, w, cx => m.reply(true, w, cx)))
            .on_action(hook!(ForwardMessage, m, w, cx => m.forward(w, cx)))
            .on_action(hook!(SendMessage, m, w, cx => m.send(w, cx)))
            .on_action(hook!(DeleteMessage, m, w, cx => m.delete(w, cx)))
            .on_action(hook!(SearchMail, m, w, cx => m.search(w, cx)))
            .child(self.render_view(theme, window, cx))
            .into_any_element()
    }
}

/// What a Space's delete confirmation adds for its mail accounts (`Browser::mail_lost_with`).
pub(crate) fn delete_warning(lost_accounts: &[String]) -> String {
    match lost_accounts {
        [] => String::new(),
        [one] => format!(" Its mail account {one} is removed from baa, with its mail here; accounts in All Spaces stay."),
        many => format!(" Its mail accounts {} are removed from baa, with their mail here; accounts in All Spaces stay.", many.join(", ")),
    }
}

// ponytail: plain names in the middle of their place until lane 5 draws mocks A2/B2.
fn placeholder(name: &'static str, theme: &Theme) -> gpui::Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(12.))
        .items_center()
        .justify_center()
        .text_color(theme.ink_muted)
        .child(name)
}
