//! Mail in the shell (PLAN-mail.md §2, §5): the process's `Mail` and its events, what a window
//! holds of mail, and the frame of the mail view. The parts are in the files beside this one:
//! `nav` (the Archive's Mail section) and `list` (WP-H), `reader` and `commands` (WP-I),
//! `compose` and `tabs` (WP-J), `setup` (WP-K) and `code_offer` (WP-L).

mod code_offer;
mod commands;
pub(crate) mod compose;
mod list;
mod nav;
pub(crate) mod reader;
pub(crate) mod setup;
mod tabs;
#[cfg(test)]
mod tests_codes;
#[cfg(test)]
mod tests_compose;
#[cfg(test)]
mod tests_list;
#[cfg(test)]
mod tests_reader;
#[cfg(test)]
mod tests_setup;
#[cfg(test)]
mod tests_tabs;

use std::{collections::HashMap, sync::Arc, time::Instant};

use baa_mail::{AccountId, DraftId, Event, Filter, Mail, MailSpace, SyncState, Tab, View, ViewState};
use baa_model::{Pane, Showing, SpaceId};
use baa_web::{PageHandle, WebHost};
use futures::StreamExt;
use gpui::{div, prelude::*, px, AnyElement, App, Context, Div, Entity, EventEmitter, FocusHandle, Hsla, KeyDownEvent, Stateful, Subscription, Window};

use crate::{
    actions::{BackToList, CloseTab, DeleteMessage, ForwardMessage, NewMessage, Reply, ReplyAll, SearchMail, SendMessage},
    browser::{feel, Browser},
    input::{self, TextField},
    motion::Feel,
    theme::{ascii, fade, mix, r, surface, Theme},
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
        // A sync storm (a first sync, a mailbox refilled) is one Changed an account a turn.
        while let Some(first) = rx.next().await {
            let mut batch = vec![first];
            while let Ok(event) = rx.try_recv() {
                if !batch.contains(&event) {
                    batch.push(event);
                }
            }
            if to.update(cx, |_, cx| batch.into_iter().for_each(|event| cx.emit(event))).is_err() {
                break;
            }
        }
    })
    .detach();
    cx.set_global(MailHub { mail: Arc::new(mail), events });
    Ok(())
}

/// A text field of the mail view. They share [`MailUi::field_focus`]: the one in `field` has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    Search,
    To,
    Cc,
    Bcc,
    Subject,
}

/// The narrowest mail view that shows the list (380 px) beside a readable message; a narrower
/// one shows each in a tab of its own.
pub(crate) const MAIL_WIDE: f32 = 900.;

/// Tells every window an account is gone, as a sync's `Changed` does, so what showed of it goes.
pub(crate) fn account_removed(account: AccountId, cx: &mut App) {
    if let Some(events) = cx.try_global::<MailHub>().map(|hub| hub.events.clone()) {
        events.update(cx, |_, cx| cx.emit(Event::Changed { account }));
    }
}

/// Mail in one window: `Browser.mail`, `Some` only with mail on in a window that is not private.
pub(crate) struct MailUi<H: WebHost> {
    pub(crate) mail: Arc<Mail>,
    /// The mail view's keyboard focus: its `Mail` key context is active while this has it.
    pub(crate) focus: FocusHandle,
    /// Every text field of the mail view: while it has focus the `MailField` key context is
    /// on, where ⌫ edits text instead of deleting mail.
    pub(crate) field_focus: FocusHandle,
    pub(crate) field: Field,
    pub(crate) search: TextField,
    /// The Space whose view `state` is; another Space's is read when it becomes active.
    pub(crate) space: Option<MailSpace>,
    pub(crate) state: ViewState,
    /// The Unread and Flagged search folders; not kept across a Space switch.
    pub(crate) filter: Filter,
    pub(crate) list: gpui::UniformListScrollHandle,
    /// The selected conversation, in the reading pane or the preview tab.
    pub(crate) reader: Option<reader::Reader<H>>,
    /// The open drafts, as the tab strip shows them; `state.tab()` says which tab is in front.
    pub(crate) drafts: Vec<compose::Composer<H>>,
    /// The mail view was at least [`MAIL_WIDE`] when last laid out; `None` before that.
    pub(crate) wide: Option<bool>,
    /// The last state each account reported, and when the last sync finished.
    pub(crate) status: HashMap<AccountId, SyncState>,
    pub(crate) synced_at: Option<Instant>,
    /// Verification codes offered to tabs (PLAN-mail.md §7).
    pub(crate) offers: code_offer::Offers,
    /// Made at the first sync with a window: a document is made in one.
    events: Option<Subscription>,
}

impl<H: WebHost> MailUi<H> {
    pub(crate) fn view(&self) -> Option<View> {
        Some(View {
            space: self.space?,
            folder: self.state.folder,
            account: self.state.account,
            filter: self.filter,
            search: self.search.text.trim().to_owned(),
        })
    }

    /// Keeps what mail shows, for the next Space switch or launch.
    pub(crate) fn save_view(&self) {
        if let Some(space) = self.space {
            self.mail.set_view_state(space, &self.state);
        }
    }

    pub(crate) fn wide(&self) -> bool {
        self.wide.unwrap_or(true)
    }

    fn reader_drawn(&self) -> bool {
        match self.state.tab() {
            Tab::List => self.wide(),
            Tab::Preview => true,
            Tab::Draft(_) => false,
        }
    }

    /// The selected conversation is gone (deleted, moved, its account removed), and with it
    /// the preview tab.
    pub(crate) fn drop_reader(&mut self) {
        self.reader = None;
        self.state.selected = None;
        self.state.close_preview();
    }

    pub(crate) fn front_draft(&self) -> Option<&compose::Composer<H>> {
        let front = self.state.front?;
        self.drafts.iter().find(|d| d.id == front)
    }

    pub(crate) fn front_draft_mut(&mut self) -> Option<&mut compose::Composer<H>> {
        let front = self.state.front?;
        self.drafts.iter_mut().find(|d| d.id == front)
    }

    fn field_mut(&mut self) -> Option<&mut TextField> {
        match self.field {
            Field::Search => Some(&mut self.search),
            field => self.front_draft_mut().and_then(|d| d.field_mut(field)),
        }
    }

    /// The pages of mail: hidden unless mail is on screen, and then only the one in front.
    fn show_documents(&self, on_screen: bool, cx: &App) {
        for draft in &self.drafts {
            if let Some(page) = &draft.page {
                page.read(cx).set_visible(on_screen && self.state.front == Some(draft.id));
            }
        }
        if let Some(page) = self.reader.as_ref().and_then(|r| r.page.as_ref()) {
            page.read(cx).set_visible(on_screen && self.reader_drawn());
        }
    }
}

impl<H: WebHost> Browser<H> {
    /// Makes `mail` match the settings: on with mail enabled in a window that is not private
    /// once [`start_mail`] has run, else off, and with mail off no Space shows it.
    pub(crate) fn sync_mail(&mut self, cx: &mut Context<Self>) {
        let hub = cx.try_global::<MailHub>().filter(|_| self.settings.mail.enabled && !self.private);
        match hub {
            Some(_) if self.mail.is_some() => {}
            Some(hub) => {
                self.mail = Some(MailUi {
                    mail: hub.mail.clone(),
                    focus: cx.focus_handle(),
                    field_focus: cx.focus_handle(),
                    field: Field::Search,
                    search: TextField::default(),
                    space: None,
                    state: ViewState::default(),
                    filter: Filter::All,
                    list: gpui::UniformListScrollHandle::new(),
                    reader: None,
                    drafts: Vec::new(),
                    wide: None,
                    status: HashMap::new(),
                    synced_at: None,
                    offers: code_offer::Offers::default(),
                    events: None,
                });
            }
            None => {
                self.mail = None;
                self.session.forget_mail();
            }
        }
    }

    /// After every session change (`Browser::sync`): the active Space's view is the one
    /// loaded, and mail's pages show only while mail is on screen.
    pub(crate) fn sync_mail_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let space = self.mail_space().map(|(_, space)| space);
        let hub = cx.try_global::<MailHub>().map(|hub| hub.events.clone());
        let Some(mail) = &mut self.mail else { return };
        if let (None, Some(events)) = (&mail.events, hub) {
            mail.events = Some(cx.subscribe_in(&events, window, |this, _, event, window, cx| this.on_mail_event(event, window, cx)));
        }
        let Some(mail) = &mut self.mail else { return };
        if mail.space != space {
            mail.space = space;
            mail.state = space.map(|s| mail.mail.view_state(s)).unwrap_or_default();
            if let Some(wide) = mail.wide {
                mail.state.fit(wide);
            }
            mail.filter = Filter::All;
            mail.search = TextField::default();
            mail.reader = None;
            mail.drafts.clear();
            mail.list.scroll_to_item(mail.state.scroll, gpui::ScrollStrategy::Top);
            self.restore_drafts(window, cx);
            if let Some(thread) = self.mail.as_ref().and_then(|m| m.state.selected) {
                self.show_thread(thread, window, cx);
            }
        }
        self.show_mail_documents(cx);
        self.drop_hidden_offers(cx);
    }

    /// The mail view was laid out at `wide` or not: crossing [`MAIL_WIDE`] folds the preview tab
    /// into the reading pane or takes the pane away, and moves the reader's page with it.
    pub(crate) fn set_mail_wide(&mut self, wide: bool, cx: &mut Context<Self>) {
        let Some(mail) = &mut self.mail else { return };
        if mail.wide == Some(wide) {
            return;
        }
        mail.wide = Some(wide);
        mail.state.fit(wide);
        mail.save_view();
        self.show_mail_documents(cx);
        cx.notify();
    }

    /// After the page in front of mail changed (a draft opened or closed, a message selected).
    pub(crate) fn show_mail_documents(&self, cx: &App) {
        let on_screen = self.panes_on_screen().contains(&Pane::Mail);
        if let Some(mail) = &self.mail {
            mail.show_documents(on_screen, cx);
        }
    }

    fn on_mail_event(&mut self, event: &Event, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mail) = &mut self.mail else { return };
        let ours = |mail: &MailUi<H>, account: &AccountId| {
            mail.space.is_some_and(|space| mail.mail.accounts(space).iter().any(|a| a.id == *account))
        };
        match event {
            Event::Changed { account } => {
                self.on_codes_changed(cx);
                let Some(mail) = &self.mail else { return };
                if !ours(mail, account) {
                    return self.forget_account(*account, window, cx);
                }
                self.refresh_reader(window, cx);
                if !self.mail_drawn() {
                    return;
                }
            }
            Event::Status { account, state } => {
                mail.status.insert(*account, *state);
                if *state == SyncState::Synced {
                    mail.synced_at = Some(Instant::now());
                }
                if !ours(mail, account) || !self.mail_drawn() {
                    return;
                }
            }
            Event::Sent(draft) => return self.on_sent(*draft, cx),
            // `what` is for the user: it may name a host or an address, so it is not logged.
            Event::Failed { account, what } => {
                if !ours(mail, account) {
                    return;
                }
                let what = what.clone();
                self.show_toast(what, cx);
            }
        }
        cx.notify();
    }

    /// An account of this Space went (removed in Settings › Mail): its conversation, its
    /// drafts' tabs and a scope or folder of it leave the view.
    fn forget_account(&mut self, account: AccountId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mail) = &mut self.mail else { return };
        if mail.reader.as_ref().and_then(|r| r.message()).is_some_and(|m| m.account == account) {
            mail.drop_reader();
        }
        if mail.state.account == Some(account) {
            mail.state.account = None;
        }
        let folders = mail.space.map(|s| mail.mail.folders(s)).unwrap_or_default();
        if mail.state.folder.is_some_and(|f| !folders.iter().any(|g| g.id == f)) {
            mail.state.folder = None;
        }
        mail.save_view();
        let drafts: Vec<DraftId> = mail.drafts.iter().filter(|d| d.from == account).map(|d| d.id).collect();
        for draft in drafts {
            self.close_composer(draft, cx);
        }
        self.refresh_reader(window, cx);
        self.show_mail_documents(cx);
        cx.notify();
    }

    fn mail_drawn(&self) -> bool {
        self.panes_on_screen().contains(&Pane::Mail)
    }

    /// The one way into `Mail` for a Space: `None` with mail off, in a private window and
    /// for an incognito Space.
    pub(crate) fn mail_of(&self, space: SpaceId) -> Option<(&Arc<Mail>, MailSpace)> {
        let mail = self.mail.as_ref()?;
        Some((&mail.mail, MailSpace::of(self.session.space(space)?)?))
    }

    pub(crate) fn mail_space(&self) -> Option<(&Arc<Mail>, MailSpace)> {
        self.mail_of(self.session.active_space()?)
    }

    /// The active Space can have mail: the Archive's Mail item and ⌘⇧M are there.
    pub(crate) fn mail_here(&self) -> bool {
        self.mail_space().is_some()
    }

    /// The active Space has mail in front: the Archive is open on Mail, and the page area
    /// draws the mail view where the page was.
    pub(crate) fn mail_in_front(&self) -> bool {
        self.front() == Some(Pane::Mail)
    }

    /// Mail (`true`) or the browser in front, in the active Space; the Archive opens on Mail or
    /// closes with it (`sync_library`).
    pub(crate) fn show_mail(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self.session.active_space() else { return };
        if on && !self.mail_here() {
            return;
        }
        self.session.set_showing(space, if on { Showing::Mail } else { Showing::Browser });
        self.sync(window, cx);
    }

    /// Another Space, with mail in front: the account picker's Go to entries.
    pub(crate) fn show_mail_of(&mut self, space: SpaceId, window: &mut Window, cx: &mut Context<Self>) {
        if self.mail_of(space).is_none() {
            return;
        }
        self.activate_space(space, window, cx);
        self.show_mail(true, window, cx);
    }

    /// The text field with the keyboard, for the input method.
    fn mail_field(&mut self) -> Option<&mut TextField> {
        self.mail.as_mut()?.field_mut()
    }

    pub(crate) fn focus_mail_field(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mail) = &mut self.mail else { return };
        mail.field = field;
        window.focus(&mail.field_focus, cx);
        cx.notify();
    }

    fn on_mail_field_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mail) = &mut self.mail else { return };
        let field = mail.field;
        let Some(text) = mail.field_mut() else { return };
        let before = text.text.clone();
        let key = text.handle(&event.keystroke, cx);
        match field {
            Field::Search => self.on_search_key(key, &before, window, cx),
            field => self.on_compose_key(field, key, &event.keystroke.key, &before, window, cx),
        }
        key.stop_unless_typed(cx);
        cx.notify();
    }

    /// Esc in a field: the keyboard goes back to the mail view, where the mail keys are.
    pub(crate) fn leave_mail_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(mail) = &self.mail {
            window.focus(&mail.focus, cx);
        }
    }

    /// A text field of the mail view, drawn with its own placeholder; a click gives it the keyboard.
    pub(crate) fn render_mail_field(&self, field: Field, placeholder: &'static str, ink: gpui::Hsla, dim: gpui::Hsla, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(mail) = &self.mail else { return div().into_any_element() };
        let text = match field {
            Field::Search => Some(&mail.search),
            field => mail.front_draft().and_then(|d| d.field(field)),
        };
        let Some(text) = text else { return div().into_any_element() };
        let focused = mail.field == field && mail.field_focus.is_focused(window);
        div()
            .id(("mail-field", field as u64))
            .flex_1()
            .min_w_0()
            .cursor_text()
            .on_click(cx.listener(move |this, _, window, cx| this.focus_mail_field(field, window, cx)))
            .child(text.render(focused, placeholder, ink, dim, fade(mix(ink, gpui::blue(), 0.5), 0.3)))
            .into_any_element()
    }

    /// A button on the mail card: filled when `primary`; `(id, n)` is its element id.
    pub(crate) fn mail_button(&self, id: &'static str, n: u64, primary: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let key = (id, n);
        let h = self.motion.get(key, Feel::Hover);
        let (fg, bg): (Hsla, Hsla) = if primary { (surface::bg().into(), surface::ink().into()) } else { (surface::ink().into(), surface::key().into()) };
        let el = div()
            .id((id, n))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .h(px(26.))
            .px(px(10.))
            .text_size(px(12.))
            .text_color(fg)
            .cursor_pointer();
        let el = if ascii() {
            el.border_1().border_color(mix(surface::border().into(), surface::ink().into(), h)).when(primary, |d| d.bg(bg))
        } else {
            el.rounded(r(7.)).bg(mix(bg, surface::muted().into(), h * 0.25))
        };
        feel(el, key, cx)
    }

    pub(crate) fn mail_view(&self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(mail) = &self.mail else { return div().into_any_element() };
        // The fields' keyboard: no area of its own, so a click elsewhere in mail never lands
        // in a field (a click on a field gives it the keyboard, `render_mail_field`).
        let fields = div()
            .absolute()
            .size_0()
            .key_context("MailField")
            .track_focus(&mail.field_focus)
            .on_key_down(cx.listener(Self::on_mail_field_key));
        let fields = input::wire(fields, &mail.field_focus, true, window, cx, Self::mail_field, Self::on_mail_field_key);
        div()
            .id("mail-view")
            .debug_selector(|| "mail-view".into())
            .key_context("Mail")
            .track_focus(&mail.focus)
            .size_full()
            .on_action(hook::<H, NewMessage>(|b, w, cx| b.new_message(w, cx), cx))
            .on_action(hook::<H, Reply>(|b, w, cx| b.reply_to_selected(false, w, cx), cx))
            .on_action(hook::<H, ReplyAll>(|b, w, cx| b.reply_to_selected(true, w, cx), cx))
            .on_action(hook::<H, ForwardMessage>(|b, w, cx| b.forward_selected(w, cx), cx))
            .on_action(hook::<H, SendMessage>(|b, w, cx| b.send_front_draft(w, cx), cx))
            .on_action(hook::<H, DeleteMessage>(|b, w, cx| b.delete_selected(w, cx), cx))
            .on_action(hook::<H, SearchMail>(|b, w, cx| b.focus_mail_field(Field::Search, w, cx), cx))
            .on_action(cx.listener(|this, _: &BackToList, window, cx| {
                if !this.back_to_list(window, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &CloseTab, _, cx| this.close_mail_tab(cx)))
            .child(self.render_view(theme, window, cx))
            .child(fields)
            .child(self.measure_mail(cx))
            .into_any_element()
    }

    /// Reads the mail view's own width once laid out: a split pane is narrow in a wide window.
    fn measure_mail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let known = self.mail.as_ref().and_then(|m| m.wide);
        let this = cx.entity().downgrade();
        gpui::canvas(
            move |bounds, _, cx| {
                let wide = f32::from(bounds.size.width) >= MAIL_WIDE;
                if known != Some(wide) {
                    cx.defer(move |cx| {
                        let _ = this.update(cx, |b, cx| b.set_mail_wide(wide, cx));
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    /// The mail view: the page area's content (or a split pane's) while mail shows. The tab
    /// strip, then the tab in front across the whole view: the list (beside the reading pane
    /// when wide), the preview, or a draft; the list and the preview have their command row.
    pub(crate) fn render_view(&self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(mail) = &self.mail else { return div().into_any_element() };
        let tab = mail.state.tab();
        let commands = self.render_commands(tab, theme, window, cx);
        let body = match tab {
            Tab::Draft(_) => self.render_composer(theme, window, cx),
            Tab::Preview => self.render_reader(theme, window, cx),
            Tab::List if mail.wide() => div()
                .size_full()
                .flex()
                .child(self.render_list(true, theme, window, cx))
                .child(div().w(px(1.)).h_full().bg(surface::line()))
                .child(div().flex_1().min_w_0().h_full().child(self.render_reader(theme, window, cx)))
                .into_any_element(),
            Tab::List => self.render_list(false, theme, window, cx),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(surface::bg())
            .text_color(surface::ink())
            .child(self.render_mail_tabs(theme, window, cx))
            .children(commands)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}

type Hook<H> = fn(&mut Browser<H>, &mut Window, &mut Context<Browser<H>>);
type Listener<A> = Box<dyn Fn(&A, &mut Window, &mut App)>;

fn hook<H: WebHost, A: gpui::Action>(on: Hook<H>, cx: &mut Context<Browser<H>>) -> Listener<A> {
    Box::new(cx.listener(move |this, _: &A, window, cx| {
        if this.mail.is_some() {
            on(this, window, cx);
        }
    }))
}

/// What a Space's delete confirmation adds for its mail accounts (`Browser::mail_lost_with`).
pub(crate) fn delete_warning(lost_accounts: &[String]) -> String {
    match lost_accounts {
        [] => String::new(),
        [one] => format!(" Its mail account {one} is removed from baa, with its mail here; accounts in All Spaces stay."),
        many => format!(" Its mail accounts {} are removed from baa, with their mail here; accounts in All Spaces stay.", many.join(", ")),
    }
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}
