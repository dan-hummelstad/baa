//! Mail in the shell (PLAN-mail.md §5), on `MockHost`.

use baa_mail::{Filter, Mail, MemorySecrets};
use baa_model::{Pane, Session, Settings, Showing, SpaceId, Store, TabId};
use baa_web::{DocumentKind, PageHandle, WebHost};
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::{actions::*, library::Section, mock::MockHost, Browser};

pub(crate) type B = Entity<Browser<MockHost>>;

/// A window on `store`, after the app has started the process's `Mail` with mail.db in memory.
pub(crate) fn window(cx: &mut TestAppContext, store: Store) -> (B, &mut VisualTestContext) {
    // An account's thread reports its sync state from its own thread, which gpui's test
    // scheduler sees as an outside wakeup.
    cx.executor().allow_parking();
    cx.update(crate::init);
    let connect: baa_mail::Connect = Box::new(|_, _| Err(baa_mail::Error::Other("no server in these tests".into())));
    cx.update(|cx| crate::start_mail(cx, |notify| Mail::open_with(None, Box::new(MemorySecrets::default()), notify, connect)))
        .unwrap();
    cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), store, window, cx))
}

/// The process's `Mail`, for a test to seed (`baa_mail::testing`).
pub(crate) fn mail(b: &B, cx: &mut VisualTestContext) -> std::sync::Arc<Mail> {
    b.read_with(cx, |b, _| b.mail.as_ref().expect("mail on").mail.clone())
}

/// The active Space as mail knows it.
pub(crate) fn mail_space(b: &B, cx: &mut VisualTestContext) -> baa_mail::MailSpace {
    b.read_with(cx, |b, _| b.mail_space().expect("a Space with mail").1)
}

/// A window with mail in front and one account of the active Space (`dan@example.com`, id 1),
/// whose folders are Inbox, Drafts, Sent, Archive and Trash in that order.
pub(crate) fn with_account(cx: &mut TestAppContext) -> (B, &mut VisualTestContext, std::sync::Arc<Mail>, Vec<baa_mail::FolderId>) {
    let (b, cx) = with_mail(cx);
    let mail = mail(&b, cx);
    let space = mail_space(&b, cx);
    let folders = mail.seed_account(&baa_mail::testing::account(1, Some(space), "dan@example.com"));
    b.update_in(cx, |b, window, cx| b.show_mail(true, window, cx));
    cx.run_until_parked();
    (b, cx, mail, folders)
}

pub(crate) fn store_with_mail(on: bool) -> Store {
    let store = Store::open(":memory:").unwrap();
    let mut settings = Settings::default();
    settings.mail.enabled = on;
    store.save_settings(&settings).unwrap();
    store
}

pub(crate) fn with_mail(cx: &mut TestAppContext) -> (B, &mut VisualTestContext) {
    window(cx, store_with_mail(true))
}

pub(crate) fn open(b: &B, cx: &mut VisualTestContext, url: &str) -> TabId {
    b.update_in(cx, |b, window, cx| {
        b.open_url(url, window, cx);
        b.active_tab().unwrap()
    })
}

fn visible(b: &B, cx: &mut VisualTestContext, tab: TabId) -> bool {
    b.read_with(cx, |b, cx| b.pages[&tab].0.read(cx).visible.get())
}

pub(crate) fn drawn(cx: &mut VisualTestContext, selector: &'static str) -> bool {
    cx.run_until_parked();
    cx.debug_bounds(selector).is_some()
}

fn section(b: &B, cx: &mut VisualTestContext) -> Option<Section> {
    b.read_with(cx, |b, _| b.library.as_ref().map(|l| l.section))
}

fn show_mail(b: &B, cx: &mut VisualTestContext, on: bool) {
    b.update_in(cx, |b, window, cx| b.show_mail(on, window, cx));
}

fn mail_over_a_tab(cx: &mut TestAppContext) -> (B, &mut VisualTestContext, TabId) {
    let (b, cx) = with_mail(cx);
    let tab = open(&b, cx, "https://a.com/");
    show_mail(&b, cx, true);
    (b, cx, tab)
}

fn browser_page_is_back(b: &B, cx: &mut VisualTestContext, tab: TabId, what: &str) {
    b.read_with(cx, |b, _| {
        assert!(b.library.is_none(), "{what}: the Archive is closed");
        assert_eq!(b.front(), Some(Pane::Tab(tab)), "{what}: the tab is in front");
    });
    assert!(visible(b, cx, tab), "{what}: the page is on screen");
    assert!(!drawn(cx, "mail-view"), "{what}: no mail in the page area");
}

#[gpui::test]
fn choosing_mail_in_the_archive_shows_the_mail_nav_and_the_mail_view(cx: &mut TestAppContext) {
    let (b, cx) = with_mail(cx);
    let tab = open(&b, cx, "https://a.com/");
    assert!(visible(&b, cx, tab));

    b.update_in(cx, |b, window, cx| b.open_library(Section::Mail, window, cx));
    assert_eq!(section(&b, cx), Some(Section::Mail), "the Archive is on Mail");
    b.read_with(cx, |b, _| assert!(b.mail.as_ref().unwrap().state.account.is_none(), "on All inboxes"));
    assert!(drawn(cx, "mail-nav"), "the mail nav is the Archive's body");
    assert!(drawn(cx, "mail-view"), "the mail view is in the page area");
    assert!(drawn(cx, "library-mail"), "the rail has the Mail item");
    assert!(!visible(&b, cx, tab), "a native page under mail would sit over it");
}

#[gpui::test]
fn closing_the_archive_shows_the_browser_page_each_way_it_closes(cx: &mut TestAppContext) {
    use gpui::TouchPhase::{Ended, Moved, Started};
    let (b, cx, tab) = mail_over_a_tab(cx);
    let ways = ["the back arrow", "cmd-shift-a", "cmd-shift-m", "a swipe down over the bottom", "a cmd-swipe over the sidebar"];
    for way in ways {
        assert_eq!(section(&b, cx), Some(Section::Mail), "{way}: starts on Mail");
        match way {
            "the back arrow" => b.update_in(cx, |b, window, cx| b.close_library(window, cx)),
            "cmd-shift-a" => cx.simulate_keystrokes("cmd-shift-a"),
            "cmd-shift-m" => cx.simulate_keystrokes("cmd-shift-m"),
            "a swipe down over the bottom" => b.update_in(cx, |b, window, cx| {
                b.on_bottom_bar_scroll(&scroll(Started, 0., 0.), window, cx);
                b.on_bottom_bar_scroll(&scroll(Moved, 0., 30.), window, cx);
                b.on_bottom_bar_scroll(&scroll(Moved, 0., 30.), window, cx);
                b.on_bottom_bar_scroll(&scroll(Ended, 0., 0.), window, cx);
            }),
            _ => chrome_swipe(&b, cx, -300., true),
        }
        browser_page_is_back(&b, cx, tab, way);
        show_mail(&b, cx, true);
    }
}

#[gpui::test]
fn another_rail_item_shows_that_section_and_choosing_mail_again_brings_mail_back_as_it_was(cx: &mut TestAppContext) {
    let (b, cx, _mail, folders) = with_account(cx);
    let tab = open(&b, cx, "https://a.com/");
    b.update_in(cx, |b, window, cx| {
        b.open_library(Section::Mail, window, cx);
        b.open_list(Some(folders[1]), Filter::All, cx);
    });

    for other in [Section::Downloads, Section::Archive, Section::Chat] {
        b.update_in(cx, |b, window, cx| match other {
            Section::Chat => b.open_chat(window, cx),
            other => b.open_library(other, window, cx),
        });
        assert_eq!(section(&b, cx), Some(other), "{other:?} is shown");
        b.read_with(cx, |b, _| assert_eq!(b.front(), Some(Pane::Tab(tab)), "{other:?}: the browser page is in front"));
        assert!(visible(&b, cx, tab), "{other:?}: the page is on screen");
        assert!(!drawn(cx, "mail-view") && !drawn(cx, "mail-nav"), "{other:?}: no mail");

        b.update_in(cx, |b, window, cx| b.open_library(Section::Mail, window, cx));
        assert!(drawn(cx, "mail-view") && drawn(cx, "mail-nav"), "Mail is back after {other:?}");
        b.read_with(cx, |b, _| assert!(b.mail.as_ref().unwrap().state.folder == Some(folders[1]), "the folder is still the one chosen"));
    }
}

#[gpui::test]
fn the_browser_sidebar_never_shows_mail_navigation(cx: &mut TestAppContext) {
    let (b, cx, _mail, _folders) = with_account(cx);
    open(&b, cx, "https://a.com/");
    b.read_with(cx, |b, _| assert!(b.library.is_none() && b.mail.is_some()));
    assert!(!drawn(cx, "mail-nav"), "the browser sidebar has no mail nav");
    b.update_in(cx, |b, window, cx| b.open_library(Section::Downloads, window, cx));
    assert!(!drawn(cx, "mail-nav"), "nor does another section of the Archive");

    show_mail(&b, cx, true);
    assert!(drawn(cx, "mail-nav"), "the mail nav is drawn with mail showing");
    assert_eq!(section(&b, cx), Some(Section::Mail), "and only as the Archive's Mail section");
}

#[gpui::test]
fn cmd_shift_m_opens_the_archive_on_mail_and_collapses_it_back_to_the_same_tab(cx: &mut TestAppContext) {
    let (b, cx) = with_mail(cx);
    let tab = open(&b, cx, "https://a.com/");
    assert!(!drawn(cx, "mail-view"));

    cx.simulate_keystrokes("cmd-shift-m");
    b.read_with(cx, |b, _| {
        assert!(b.mail_in_front());
        assert!(b.active_page().is_none(), "page actions leave the hidden tab alone");
    });
    assert_eq!(section(&b, cx), Some(Section::Mail), "the Archive is open on Mail");
    assert!(!visible(&b, cx, tab), "a native page under mail would sit over it");
    assert!(drawn(cx, "mail-view") && drawn(cx, "mail-nav"));
    let browsers = |cx: &mut VisualTestContext| cx.windows().iter().filter(|w| w.downcast::<Browser<MockHost>>().is_some()).count();
    let windows = browsers(cx);
    cx.simulate_keystrokes("cmd-n");
    cx.run_until_parked();
    assert_eq!(browsers(cx), windows, "in mail, ⌘N is New Message (here, with no account, Settings › Mail), not New Window");

    cx.simulate_keystrokes("cmd-shift-m");
    browser_page_is_back(&b, cx, tab, "cmd-shift-m again");
    assert!(!drawn(cx, "mail-nav"));
    cx.simulate_keystrokes("cmd-n");
    cx.run_until_parked();
    assert_eq!(browsers(cx), windows + 1, "outside mail it is New Window again");

    cx.dispatch_action(ShowMail);
    assert_eq!(section(&b, cx), Some(Section::Mail), "Open Mail in the command bar opens the Archive on Mail too");
    assert!(drawn(cx, "mail-view"));
}

#[gpui::test]
fn each_space_comes_back_showing_what_it_was_left_on(cx: &mut TestAppContext) {
    let (b, cx) = with_mail(cx);
    let a_tab = open(&b, cx, "https://a.com/");
    cx.dispatch_action(NewSpace);
    cx.simulate_keystrokes("escape");
    let b_tab = open(&b, cx, "https://b.com/");
    let (a, other) = b.read_with(cx, |b, _| {
        let spaces: Vec<SpaceId> = b.session.spaces().iter().map(|s| s.id).collect();
        (spaces[0], spaces[1])
    });
    b.update_in(cx, |b, window, cx| {
        b.activate_space(a, window, cx);
        b.show_mail(true, window, cx);
        b.activate_space(other, window, cx);
    });
    b.read_with(cx, |b, _| assert_eq!(b.front(), Some(Pane::Tab(b_tab)), "Space B was left in the browser"));
    assert!(section(&b, cx).is_none() && !drawn(cx, "mail-view"), "no Archive open in Space B");

    b.update_in(cx, |b, window, cx| b.activate_space(a, window, cx));
    b.read_with(cx, |b, _| assert!(b.mail_in_front(), "Space A was left in mail"));
    assert_eq!(section(&b, cx), Some(Section::Mail), "the Archive is open on Mail in Space A");
    assert!(drawn(cx, "mail-view"));

    show_mail(&b, cx, false);
    b.read_with(cx, |b, _| assert_eq!(b.front(), Some(Pane::Tab(a_tab))));
}

#[gpui::test]
fn a_session_saved_by_the_current_main_with_a_space_on_mail_opens_the_archive_on_mail(cx: &mut TestAppContext) {
    let store = store_with_mail(true);
    let mut session = Session::first_run();
    let space = session.active_space().unwrap();
    session.open_tab(space, "https://a.com/", 1);
    session.set_showing(space, Showing::Mail);
    store.save_session(&session).unwrap();

    let (b, cx) = window(cx, store);
    b.read_with(cx, |b, _| assert!(b.mail_in_front(), "the Space is on mail"));
    assert_eq!(section(&b, cx), Some(Section::Mail), "the Archive is open on Mail");
    assert!(drawn(cx, "mail-view") && drawn(cx, "mail-nav"));
}

#[gpui::test]
fn a_split_of_mail_and_a_tab_keeps_both_on_screen_while_the_archive_follows_focus(cx: &mut TestAppContext) {
    let (b, cx, tab) = mail_over_a_tab(cx);
    cx.dispatch_action(SplitView);
    let space = b.read_with(cx, |b, _| {
        assert_eq!(b.panes_on_screen(), [Pane::Mail, Pane::Tab(tab)], "mail split with the most recent tab");
        assert_eq!(b.front(), Some(Pane::Tab(tab)), "the new pane has focus, as a tab's would");
        b.session.active_space().unwrap()
    });
    assert!(section(&b, cx).is_none(), "the Archive is collapsed while the tab pane has focus");
    assert!(visible(&b, cx, tab) && drawn(cx, "mail-view"), "both panes are drawn");

    show_mail(&b, cx, true);
    assert_eq!(section(&b, cx), Some(Section::Mail), "a click on the mail pane opens the Archive on Mail");
    assert!(visible(&b, cx, tab), "mail in front keeps its split on screen");
    let saved = b.read_with(cx, |b, _| b.store().unwrap().load_session().unwrap());
    let s = saved.space(space).unwrap();
    assert_eq!((s.split.as_ref().unwrap().panes.clone(), s.showing), (vec![Pane::Mail, Pane::Tab(tab)], Showing::Mail));

    b.update_in(cx, |b, window, cx| b.on_page_focused(tab, window, cx));
    assert!(section(&b, cx).is_none(), "a click on the tab pane collapses the Archive");
    b.read_with(cx, |b, _| assert_eq!(b.panes_on_screen(), [Pane::Mail, Pane::Tab(tab)], "the split is still on screen"));
    assert!(visible(&b, cx, tab) && drawn(cx, "mail-view"));
}

fn assert_no_mail_in_the_archive(b: &B, cx: &mut VisualTestContext, what: &str) {
    b.update_in(cx, |b, window, cx| b.open_library(Section::Mail, window, cx));
    assert_eq!(section(b, cx), Some(Section::Downloads), "{what}: Mail lands on Downloads");
    assert!(!drawn(cx, "library-mail"), "{what}: no Mail item in the rail");
    cx.simulate_keystrokes("cmd-shift-m");
    b.read_with(cx, |b, _| assert!(!b.mail_in_front(), "{what}: cmd-shift-m shows no mail"));
    assert_eq!(section(b, cx), Some(Section::Downloads), "{what}: the Archive stays on Downloads");
}

#[gpui::test]
fn an_incognito_space_or_window_has_no_mail_item(cx: &mut TestAppContext) {
    let (b, cx) = with_mail(cx);
    open(&b, cx, "https://a.com/");
    let private = b.update_in(cx, |b, window, cx| {
        let id = b.session.add_ephemeral_space("Incognito", b.host.ephemeral_profile());
        b.activate_space(id, window, cx);
        assert!(b.mail.is_some() && !b.mail_here() && b.mail_space().is_none());
        b.show_mail(true, window, cx);
        assert!(!b.mail_in_front());
        id
    });
    assert_no_mail_in_the_archive(&b, cx, "incognito Space");
    b.read_with(cx, |b, _| assert!(b.store().unwrap().load_session().unwrap().space(private).is_none(), "nothing of it is saved"));

    let h = cx.cx.update(|cx| crate::open_window::<MockHost>(true, cx)).expect("a window opens");
    let pcx = VisualTestContext::from_window(h.into(), &cx.cx).into_mut();
    let p = h.root(pcx).unwrap();
    pcx.run_until_parked();
    p.read_with(pcx, |p, _| assert!(p.private && p.settings.mail.enabled && p.mail.is_none() && !p.mail_here()));
    assert_no_mail_in_the_archive(&p, pcx, "private window");
}

#[gpui::test]
fn with_mail_off_the_archive_has_no_mail_item_and_a_mail_session_loads_as_the_browser(cx: &mut TestAppContext) {
    let store = store_with_mail(false);
    let mut session = Session::first_run();
    let space = session.active_space().unwrap();
    let a = session.open_tab(space, "https://a.com/", 1);
    session.open_tab(space, "https://b.com/", 2);
    session.set_showing(space, Showing::Mail);
    session.add_to_split(space, a, usize::MAX, 3);
    session.set_showing(space, Showing::Mail);
    store.save_session(&session).unwrap();

    let (b, cx) = window(cx, store);
    b.read_with(cx, |b, _| {
        assert!(b.mail.is_none());
        let s = b.active_space().unwrap();
        assert_eq!((s.showing, s.split.is_none()), (Showing::Browser, true), "a split of mail and one tab is no split");
    });
    assert!(!drawn(cx, "mail-view") && !drawn(cx, "mail-nav"));
    assert!(section(&b, cx).is_none(), "the Archive is closed");
    assert_no_mail_in_the_archive(&b, cx, "mail off");
}

#[test]
fn deleting_a_space_names_its_mail_accounts() {
    use crate::mail::delete_warning;
    assert_eq!(delete_warning(&[]), "");
    assert!(delete_warning(&["me@example.com".into()]).contains("Its mail account me@example.com is removed"));
    assert!(delete_warning(&["a@x.com".into(), "b@y.com".into()]).contains("Its mail accounts a@x.com, b@y.com are removed"));
}

#[gpui::test]
fn the_mock_host_records_a_document_and_what_was_set_on_it(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (b, cx) = cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx));
    b.update_in(cx, |b, window, cx| {
        assert!(b.mail.is_none(), "nothing turns mail on yet");
        let host = b.host.clone();
        let profile = b.active_space().unwrap().profile;
        let tabs = host.created.borrow().len();

        let message = host.create_document(profile, DocumentKind::Message { remote_images: false }, window, cx);
        let editor = host.create_document(profile, DocumentKind::Editor, window, cx);
        assert_eq!(*host.documents.borrow(), [(profile, DocumentKind::Message { remote_images: false }), (profile, DocumentKind::Editor)]);
        assert_eq!(host.created.borrow().len(), tabs, "a document is not a tab");

        message.read(cx).set_document("<p>Hei</p>");
        message.read(cx).set_document("<p>Hei igjen</p>");
        assert_eq!(*message.read(cx).set_documents.borrow(), ["<p>Hei</p>", "<p>Hei igjen</p>"]);
        assert!(editor.read(cx).set_documents.borrow().is_empty());

        let tab = host.create_page(profile, "https://example.com/", true, window, cx);
        tab.read(cx).set_document("<p>not here</p>");
        assert!(tab.read(cx).set_documents.borrow().is_empty(), "a page that has a URL takes no document");
        tab.read(cx).fill_code(7, "123456");
        assert_eq!(*tab.read(cx).filled.borrow(), [(7, "123456".to_string())]);

        host.sites.borrow_mut().insert("mail.google.com".into(), "google.com".into());
        assert_eq!((host.site_of("mail.google.com"), host.site_of("example.com")), ("google.com".into(), "example.com".into()));
    });
}

fn scroll(phase: gpui::TouchPhase, dx: f32, dy: f32) -> gpui::ScrollWheelEvent {
    gpui::ScrollWheelEvent {
        position: gpui::point(gpui::px(100.), gpui::px(700.)),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(dx), gpui::px(dy))),
        modifiers: gpui::Modifiers::none(),
        touch_phase: phase,
    }
}

#[gpui::test]
fn a_swipe_up_over_the_bottom_bar_opens_the_archive_in_the_third_space(cx: &mut TestAppContext) {
    use gpui::TouchPhase::{Ended, Moved, Started};
    cx.update(crate::init);
    let (b, cx) = cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx));
    cx.dispatch_action(crate::actions::NewSpace);
    cx.simulate_keystrokes("escape");
    cx.dispatch_action(crate::actions::NewSpace);
    cx.simulate_keystrokes("escape");
    b.read_with(cx, |b, _| {
        let s = b.session();
        assert_eq!(s.spaces().iter().position(|sp| Some(sp.id) == s.active_space()), Some(2), "in the third Space");
    });
    let swipe = |cx: &mut gpui::VisualTestContext, dx: f32, dy: f32| {
        b.update_in(cx, |b, window, cx| {
            b.on_bottom_bar_scroll(&scroll(Started, 0., 0.), window, cx);
            b.on_bottom_bar_scroll(&scroll(Moved, dx, dy), window, cx);
            b.on_bottom_bar_scroll(&scroll(Moved, dx, dy), window, cx);
            b.on_bottom_bar_scroll(&scroll(Ended, 0., 0.), window, cx);
        })
    };
    swipe(cx, 20., 0.);
    b.read_with(cx, |b, _| assert!(b.library.is_none(), "a horizontal swipe does nothing"));
    swipe(cx, 0., -30.);
    b.read_with(cx, |b, _| assert!(b.library.is_some(), "fingers up"));
    swipe(cx, 0., 30.);
    b.read_with(cx, |b, _| assert!(b.library.is_none(), "fingers down"));
}

/// A two-finger horizontal swipe over the chrome, `dx` px a step over two steps; `cmd` with ⌘ held.
fn chrome_swipe(b: &B, cx: &mut VisualTestContext, dx: f32, cmd: bool) {
    use gpui::TouchPhase::{Ended, Moved, Started};
    let modifiers = gpui::Modifiers { platform: cmd, ..Default::default() };
    b.update_in(cx, |b, window, cx| {
        for (phase, dx) in [(Started, 0.), (Moved, dx), (Moved, dx), (Ended, 0.)] {
            b.on_chrome_scroll(&gpui::ScrollWheelEvent { modifiers, ..scroll(phase, dx, 0.) }, window, cx);
        }
    });
    cx.run_until_parked();
}

fn spaces(b: &B, cx: &mut VisualTestContext) -> Vec<SpaceId> {
    b.read_with(cx, |b, _| b.session.spaces().iter().map(|s| s.id).collect())
}

fn active(b: &B, cx: &mut VisualTestContext) -> Option<SpaceId> {
    b.read_with(cx, |b, _| b.session.active_space())
}

/// Space A on mail over a tab, Space B (which can have mail) left on its tab, and an incognito
/// Space C; A is active.
fn three_spaces(cx: &mut TestAppContext) -> (B, &mut VisualTestContext, [TabId; 2]) {
    let (b, cx) = with_mail(cx);
    let a_tab = open(&b, cx, "https://a.com/");
    cx.dispatch_action(NewSpace);
    cx.simulate_keystrokes("escape");
    let b_tab = open(&b, cx, "https://b.com/");
    let a = spaces(&b, cx)[0];
    b.update_in(cx, |b, window, cx| {
        b.session.add_ephemeral_space("Incognito", b.host.ephemeral_profile());
        b.activate_space(a, window, cx);
        b.show_mail(true, window, cx);
    });
    cx.run_until_parked();
    (b, cx, [a_tab, b_tab])
}

#[gpui::test]
fn in_mail_a_swipe_switches_space_and_stays_in_mail_where_the_next_space_can_have_it(cx: &mut TestAppContext) {
    let (b, cx, [_, b_tab]) = three_spaces(cx);
    let ids = spaces(&b, cx);

    chrome_swipe(&b, cx, -300., false);
    assert_eq!(active(&b, cx), Some(ids[1]), "fingers left: the next Space");
    b.read_with(cx, |b, _| assert!(b.mail_in_front(), "Space B, left on its tab, shows its mail"));
    assert_eq!(section(&b, cx), Some(Section::Mail), "the Archive stays open on Mail");
    assert!(drawn(cx, "mail-view") && drawn(cx, "mail-nav"));

    chrome_swipe(&b, cx, -300., false);
    assert_eq!(active(&b, cx), Some(ids[2]), "on to the incognito Space");
    b.read_with(cx, |b, _| assert!(!b.mail_in_front(), "it cannot have mail"));
    assert!(section(&b, cx).is_none() && !drawn(cx, "mail-view"), "its browser, with the Archive collapsed");

    b.update_in(cx, |b, window, cx| b.activate_space(ids[1], window, cx));
    assert_eq!(section(&b, cx), Some(Section::Mail), "Space B is still on mail");
    show_mail(&b, cx, false);
    b.read_with(cx, |b, _| assert_eq!(b.front(), Some(Pane::Tab(b_tab)), "and its tab is as it was left"));
}

#[gpui::test]
fn the_fold_shows_the_spaces_a_dot_switches_and_a_swipe_on_downloads_moves_on(cx: &mut TestAppContext) {
    let (b, cx, _) = three_spaces(cx);
    let ids = spaces(&b, cx);
    assert_eq!(section(&b, cx), Some(Section::Mail));
    assert!(drawn(cx, "library-bottom"), "the Fold's bottom row");
    let dot = |id: SpaceId| -> &'static str { format!("space-{}", id.0).leak() };
    for id in &ids {
        assert!(drawn(cx, dot(*id)), "a dot for each Space");
    }
    let at = cx.debug_bounds(dot(ids[1])).unwrap().center();
    cx.simulate_click(at, gpui::Modifiers::none());
    cx.run_until_parked();
    assert_eq!(active(&b, cx), Some(ids[1]), "the click switches Space");
    let pill = b.read_with(cx, |b, _| b.motion.target(("space-pill", 0), crate::motion::Feel::Glide));
    assert_eq!(pill, Some(crate::browser::SPACE_DOT), "the pill goes to the second dot");
    assert_eq!(section(&b, cx), Some(Section::Mail), "from mail, to the next Space's mail");

    b.update_in(cx, |b, window, cx| b.open_library(Section::Downloads, window, cx));
    chrome_swipe(&b, cx, -300., false);
    assert_eq!(active(&b, cx), Some(ids[2]), "fingers left: the next Space");
    assert_eq!(section(&b, cx), Some(Section::Downloads), "the Fold stays open on Downloads");
    assert!(drawn(cx, dot(ids[2])), "with the dots under it");
}

#[gpui::test]
fn cmd_shift_a_collapses_the_archive_from_each_section_and_reopens_it_on_the_last(cx: &mut TestAppContext) {
    let (b, cx, tab) = mail_over_a_tab(cx);
    for last in [Section::Mail, Section::Downloads, Section::Archive, Section::Chat, Section::Mail] {
        b.update_in(cx, |b, window, cx| match last {
            Section::Chat => b.open_chat(window, cx),
            last => b.open_library(last, window, cx),
        });
        assert_eq!(section(&b, cx), Some(last));
        cx.simulate_keystrokes("cmd-shift-a");
        browser_page_is_back(&b, cx, tab, &format!("cmd-shift-a on {last:?}"));
        cx.simulate_keystrokes("cmd-shift-a");
        assert_eq!(section(&b, cx), Some(last), "cmd-shift-a opens it on {last:?} again");
        b.read_with(cx, |b, _| assert_eq!(b.mail_in_front(), last == Section::Mail, "mail is in front only on Mail"));
    }
}

#[gpui::test]
fn a_cmd_swipe_opens_the_archive_on_its_last_section_and_collapses_it(cx: &mut TestAppContext) {
    let (b, cx, [a_tab, _]) = three_spaces(cx);
    let a = active(&b, cx);
    chrome_swipe(&b, cx, -300., true);
    browser_page_is_back(&b, cx, a_tab, "a cmd-swipe in mail");
    assert_eq!(active(&b, cx), a, "it switches no Space");

    chrome_swipe(&b, cx, 300., true);
    assert_eq!(section(&b, cx), Some(Section::Mail), "either way, it opens on the last section, Mail");
    b.read_with(cx, |b, _| assert!(b.mail_in_front()));

    b.update_in(cx, |b, window, cx| b.open_library(Section::Downloads, window, cx));
    chrome_swipe(&b, cx, -300., true);
    cx.dispatch_action(GoToSpace(2));
    assert!(section(&b, cx).is_none(), "in the incognito Space, in its browser");
    chrome_swipe(&b, cx, 300., true);
    assert_eq!(section(&b, cx), Some(Section::Downloads), "from any Space, on Downloads, the last chosen");
    chrome_swipe(&b, cx, 20., true);
    assert_eq!(section(&b, cx), Some(Section::Downloads), "a short one does nothing");
}

#[gpui::test]
fn after_a_cmd_swipe_out_of_mail_a_swipe_into_a_space_left_on_mail_shows_its_browser(cx: &mut TestAppContext) {
    let (b, cx, [_, b_tab]) = three_spaces(cx);
    let [a, b_space, _] = spaces(&b, cx)[..] else { panic!() };
    chrome_swipe(&b, cx, -300., false);
    assert_eq!((active(&b, cx), section(&b, cx)), (Some(b_space), Some(Section::Mail)), "from mail, B shows its mail");
    chrome_swipe(&b, cx, 300., true);
    browser_page_is_back(&b, cx, b_tab, "a cmd-swipe out");
    chrome_swipe(&b, cx, 300., false);
    assert_eq!(active(&b, cx), Some(a));
    assert!(section(&b, cx).is_none(), "A, left on mail, shows its browser: the Fold stays closed");
    chrome_swipe(&b, cx, -300., false);
    browser_page_is_back(&b, cx, b_tab, "and so does B");
}

#[gpui::test]
fn the_last_section_mail_opens_on_downloads_in_a_space_that_cannot_have_mail(cx: &mut TestAppContext) {
    let (b, cx, _) = three_spaces(cx);
    let c = spaces(&b, cx)[2];
    cx.simulate_keystrokes("cmd-shift-a");
    b.update_in(cx, |b, window, cx| b.activate_space(c, window, cx));
    cx.simulate_keystrokes("cmd-shift-a");
    assert_eq!(section(&b, cx), Some(Section::Downloads));
    b.read_with(cx, |b, _| assert!(!b.mail_in_front()));
}

fn bar_labels(b: &B, cx: &mut VisualTestContext) -> Vec<String> {
    b.read_with(cx, |b, _| b.bar.as_ref().unwrap().rows(&b.session, &b.settings, b.mail_here()).into_iter().map(|r| r.label).collect())
}

#[gpui::test]
fn cmd_t_from_mail_types_into_the_bar_and_opens_the_tab_in_front(cx: &mut TestAppContext) {
    let (b, cx, _) = mail_over_a_tab(cx);
    for focus_search in [false, true] {
        if focus_search {
            b.update_in(cx, |b, window, cx| b.focus_mail_field(crate::mail::Field::Search, window, cx));
        }
        cx.simulate_keystrokes("cmd-t");
        cx.simulate_input("c.example");
        assert_eq!(b.read_with(cx, |b, _| b.bar.as_ref().map(|x| x.field.text.clone())), Some("c.example".into()));
        b.read_with(cx, |b, _| assert_eq!(b.mail.as_ref().unwrap().search.text, "", "nothing went to mail's search"));
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        b.read_with(cx, |b, _| {
            let tab = b.front().and_then(|p| p.tab()).and_then(|t| b.session.tab(t)).expect("a tab in front");
            assert_eq!(tab.url, "https://c.example/");
            assert!(b.library.is_none(), "the Fold closed with mail");
        });
        show_mail(&b, cx, true);
    }
}

#[gpui::test]
fn the_bar_runs_every_bound_action_by_name_where_it_would_run(cx: &mut TestAppContext) {
    let (b, cx, _mail, _) = with_account(cx);
    cx.simulate_keystrokes("cmd-t");
    cx.simulate_input("new message");
    assert_eq!(bar_labels(&b, cx).get(1).map(String::as_str), Some("New Message"), "a Mail shortcut, with mail in front");
    let keys = |b: &B, cx: &mut VisualTestContext, label: &str| {
        b.read_with(cx, |b, _| b.bar.as_ref().unwrap().keyed.iter().find(|k| k.label == label).map(|k| k.keys.join("")))
    };
    assert_eq!(keys(&b, cx, "Reply").as_deref(), Some("⌘R"));
    assert_eq!(keys(&b, cx, "Reload").as_deref(), Some(""), "in mail ⌘R is Reply's");
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    b.read_with(cx, |b, _| assert!(b.mail.as_ref().unwrap().front_draft().is_some(), "it ran in the mail view"));

    show_mail(&b, cx, false);
    cx.simulate_keystrokes("cmd-t");
    cx.simulate_input("new message");
    assert!(!bar_labels(&b, cx).iter().any(|l| l == "New Message"), "not without mail in front");
    cx.simulate_keystrokes("escape");

    cx.simulate_keystrokes("cmd-t");
    cx.simulate_input("fold");
    let labels = bar_labels(&b, cx);
    let i = labels.iter().position(|l| l == "Open or Close the Fold").expect("the Fold by name");
    for _ in 0..i {
        cx.simulate_keystrokes("down");
    }
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(section(&b, cx).is_some(), "the Fold opened");
}
