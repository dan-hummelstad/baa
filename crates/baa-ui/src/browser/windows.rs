//! More than one window, as Arc has them. Regular windows share one session: each keeps
//! a copy, sends it to the others when it changes, and keeps its own Space, tab and
//! sidebar. A tab's page lives in one window at a time; showing it in another moves it
//! there and leaves "open in another window" behind. An incognito window has a session
//! of its own, never saved and never shared.

use std::rc::Rc;

use baa_model::Settings;
use gpui::{size, App, Bounds, TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowOptions};

use super::*;

/// What every window is made from.
struct Shared<H> {
    host: Rc<H>,
    store: Option<Rc<Store>>,
}

impl<H: 'static> gpui::Global for Shared<H> {}

/// The incognito windows. Kept apart from the views so finding the regular windows never
/// reads one, which panics while that view is being updated.
#[derive(Default)]
struct Incognito(std::collections::HashSet<gpui::WindowId>);

impl gpui::Global for Incognito {}

/// Called once, by the launch window.
pub(crate) fn init<H: WebHost>(b: &Browser<H>, cx: &mut App) {
    cx.set_global(Shared { host: b.host.clone(), store: b.store.clone() });
    // A tab shown as open in another window gets its page back once that window is gone.
    cx.on_window_closed(|cx, _| {
        for w in regular::<H>(cx) {
            w.update(cx, |b, window, cx| b.sync(window, cx)).ok();
        }
    })
    .detach();
}

/// Each new window steps down and right from the centre, so none hides another exactly.
pub fn window_options(cx: &App) -> WindowOptions {
    let step = px(24. * cx.windows().len() as f32);
    let mut bounds = Bounds::centered(None, size(px(1280.), px(860.)), cx);
    bounds.origin += point(step, step);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        // The sidebar draws the toolbar; the traffic lights sit in it.
        titlebar: Some(TitlebarOptions {
            title: Some("baa".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(16.), px(12.))),
        }),
        // Pages are native views under gpui's layer, seen through the hole
        // each page element punches; that needs a transparent layer.
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    }
}

/// The regular (not incognito) browser windows.
pub fn regular<H: WebHost>(cx: &App) -> Vec<WindowHandle<Browser<H>>> {
    let incognito = cx.try_global::<Incognito>();
    cx.windows()
        .into_iter()
        .filter(|w| incognito.is_none_or(|i| !i.0.contains(&w.window_id())))
        .filter_map(|w| w.downcast::<Browser<H>>())
        .collect()
}

/// The regular window nearest the front: links from other apps, Claude and the dock go there.
pub fn front_window<H: WebHost>(cx: &App) -> Option<WindowHandle<Browser<H>>> {
    let all = regular::<H>(cx);
    let stack = cx.window_stack().unwrap_or_default();
    stack
        .iter()
        .find_map(|w| all.iter().find(|b| b.window_id() == w.window_id()).copied())
        .or_else(|| all.first().copied())
}

/// A new window: a regular one on the front window's session (its Space, no tab yet), or
/// an incognito one. Call it outside any window's update, so the front window can be read.
pub fn open_window<H: WebHost>(private: bool, cx: &mut App) -> Option<WindowHandle<Browser<H>>> {
    let Shared { host, store } = cx.try_global::<Shared<H>>()?;
    let (host, store) = (host.clone(), store.clone());
    let from = front_window::<H>(cx).and_then(|w| w.read(cx).ok().map(|b| (b.session.clone(), b.settings.clone())));
    let settings = from.as_ref().map(|f| f.1.clone()).unwrap_or_else(|| {
        store.as_ref().and_then(|s| s.load_settings().ok()).unwrap_or_default()
    });
    let (session, store) = if private {
        let mut s = Session::default();
        let id = s.add_ephemeral_space("Incognito", host.ephemeral_profile());
        if let Some(space) = s.space_mut(id) {
            (space.accent, space.accent_to) = (INCOGNITO.0, Some(INCOGNITO.1));
        }
        s.set_active_space(id);
        (s, None)
    } else {
        let mut s = match from {
            Some((s, _)) => s,
            None => store.as_ref().and_then(|s| s.load_session().ok()).unwrap_or_else(Session::first_run),
        };
        // Its own tab comes from the command bar; the front window keeps the one it shows.
        if let Some(space) = s.active_space().and_then(|id| s.space_mut(id)) {
            space.active_tab = None;
        }
        (s, store)
    };
    let opened = cx.open_window(window_options(cx), |window, cx| {
        cx.new(|cx| Browser::build(host, store, settings, session, private, window, cx))
    });
    let handle = match opened {
        Ok(h) => h,
        Err(e) => {
            log::error!("opening a window: {e}");
            return None;
        }
    };
    if private {
        cx.default_global::<Incognito>().0.insert(handle.window_id());
    }
    handle
        .update(cx, |b, window, cx| {
            window.activate_window();
            b.open_bar(Mode::NewTab, window, cx);
        })
        .ok();
    Some(handle)
}

impl<H: WebHost> Browser<H> {
    /// Opens a window once this update is over (see `open_window`).
    pub(crate) fn open_window_later(&self, private: bool, cx: &mut Context<Self>) {
        cx.defer(move |cx| {
            open_window::<H>(private, cx);
        });
    }

    /// The shown tab's title, else the Space's name, as Arc names its windows.
    pub(crate) fn window_title(&self) -> String {
        let shown = self
            .active_tab()
            .and_then(|id| self.session.tab(id))
            .map(|t| if t.title.is_empty() { t.url.clone() } else { t.title.clone() })
            .or_else(|| self.active_space().map(|s| s.name.clone()))
            .unwrap_or_else(|| "baa".into());
        if self.private {
            format!("Incognito — {shown}")
        } else {
            shown
        }
    }

    /// The other regular windows.
    fn others(&self, cx: &App) -> Vec<WindowHandle<Browser<H>>> {
        if self.private {
            return Vec::new();
        }
        regular::<H>(cx).into_iter().filter(|w| w.window_id() != self.window.window_id()).collect()
    }

    /// Sends this window's session to the others, once per change; from `render`.
    pub(crate) fn share_session(&mut self, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.unshared) || self.private {
            return;
        }
        let others = self.others(cx);
        if others.is_empty() {
            return;
        }
        let (session, settings) = (self.session.clone(), self.settings.clone());
        cx.defer(move |cx| {
            for w in others {
                w.update(cx, |b, window, cx| b.adopt(session.clone(), settings.clone(), window, cx)).ok();
            }
        });
    }

    /// Takes another window's session, keeping what this window shows: its Space, the tab
    /// picked in each Space (while that tab is still there) and its sidebar.
    pub(crate) fn adopt(&mut self, mut session: Session, settings: Settings, window: &mut Window, cx: &mut Context<Self>) {
        let mine = &self.session;
        for space in mine.spaces() {
            let kept = space.active_tab.is_none_or(|t| session.tab(t).is_some_and(|t| t.space == space.id));
            if let Some(theirs) = session.space_mut(space.id).filter(|_| kept) {
                theirs.active_tab = space.active_tab;
            }
        }
        if let Some(id) = mine.active_space() {
            session.set_active_space(id);
        }
        session.sidebar_collapsed = mine.sidebar_collapsed;
        self.session = session;
        self.settings = settings;
        self.adopting = true;
        self.sync(window, cx);
        self.adopting = false;
    }

    /// The other window that holds `id`'s page, if one does.
    pub(crate) fn holder(&self, id: TabId, cx: &App) -> Option<WindowHandle<Browser<H>>> {
        self.others(cx).into_iter().find(|w| w.read(cx).is_ok_and(|b| b.pages.contains_key(&id)))
    }

    /// `id`'s page comes to this window from `from` (or is made anew if it is gone).
    pub(crate) fn take_page(&mut self, from: WindowHandle<Browser<H>>, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let page = from
            .update(cx, |b, _, cx| {
                cx.notify();
                b.pages.remove(&id).map(|(page, _)| page)
            })
            .ok()
            .flatten();
        match page {
            Some(page) => self.attach_page(id, page, window, cx),
            None => self.create_page(id, window, cx),
        }
    }

    /// "Show here" on a tab open in another window.
    pub(crate) fn bring_here(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(from) = self.holder(id, cx) {
            self.take_page(from, id, window, cx);
        }
        self.sync(window, cx);
    }

    /// What a pane shows while its page is in another window.
    pub(crate) fn render_elsewhere(&self, id: TabId, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.))
            .bg(surface::bg())
            .text_color(surface::muted())
            .text_size(px(13.))
            .child("This tab is open in another window")
            .child(
                div()
                    .id(("show-here", id.0))
                    .px(px(12.))
                    .py(px(6.))
                    .rounded(r(8.))
                    .bg(fade(theme.ink, 0.08))
                    .text_color(surface::ink())
                    .hover(|d| d.bg(fade(theme.ink, 0.14)))
                    .on_click(cx.listener(move |this, _, window, cx| this.bring_here(id, window, cx)))
                    .child("Show Here"),
            )
            .into_any_element()
    }
}
