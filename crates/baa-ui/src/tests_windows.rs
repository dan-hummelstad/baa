//! More than one window: incognito windows apart, regular ones on one session.

use baa_model::{Store, TabId};
use baa_web::PageEvent;
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::{actions::*, mock::MockHost, Browser};

type B = Entity<Browser<MockHost>>;

fn browser(cx: &mut TestAppContext) -> (B, &mut VisualTestContext) {
    cx.update(crate::init);
    cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx))
}

/// Opens a window the way ⌘N / ⌘⇧N do, from outside any window's update.
pub(crate) fn open(cx: &mut VisualTestContext, private: bool) -> (B, &'static mut VisualTestContext) {
    let h = cx.cx.update(|cx| crate::open_window::<MockHost>(private, cx)).expect("a window opens");
    let other = VisualTestContext::from_window(h.into(), &cx.cx).into_mut();
    let b = h.root(other).unwrap();
    other.run_until_parked();
    (b, other)
}

fn go(cx: &mut VisualTestContext, url: &str) {
    cx.simulate_keystrokes("secondary-t");
    type_url(cx, url);
}

/// Into the command bar already open, as a new window opens with it.
fn type_url(cx: &mut VisualTestContext, url: &str) {
    cx.simulate_input(url);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
}

fn tabs(b: &B, cx: &mut VisualTestContext) -> Vec<String> {
    b.read_with(cx, |b, _| {
        let s = b.session();
        s.tabs(s.active_space().unwrap(), baa_model::Zone::Today).map(|t| t.url.clone()).collect()
    })
}

fn active(b: &B, cx: &mut VisualTestContext) -> Option<TabId> {
    b.read_with(cx, |b, _| b.active_tab())
}

#[gpui::test]
fn incognito_is_a_window_of_its_own_that_keeps_nothing(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let icon = |url: &str| PageEvent::FaviconImage { url: url.into(), png: vec![1, 2, 3].into() };
    let saved = |b: &B, cx: &mut VisualTestContext| b.read_with(cx, |b, _| b.store().unwrap().load_favicons().unwrap().len());

    let (p, pcx) = open(cx, true);
    p.read_with(pcx, |p, _| {
        assert!(p.private && p.store().is_none());
        assert_eq!(p.session().spaces().len(), 1);
        assert!(p.active_space().unwrap().ephemeral);
        assert!(!p.dark && crate::theme::to_oklch(p.gradient_to.2)[0] < 0.35, "Incognito keeps its dark in baa's light look");
    });
    b.read_with(cx, |b, _| assert!(b.session().spaces().iter().all(|s| !s.ephemeral), "no incognito Space in the regular window"));

    type_url(pcx, "x.example");
    let profile = p.read_with(pcx, |p, _| p.active_space().unwrap().profile);
    p.read_with(pcx, |p, _| assert_eq!(p.host.created.borrow().last().unwrap().0, profile));
    let page = p.read_with(pcx, |p, _| p.pages.get(&p.active_tab().unwrap()).unwrap().0.clone());
    page.update(pcx, |_, cx| cx.emit(icon("https://x.example/favicon.ico")));
    pcx.run_until_parked();
    p.read_with(pcx, |p, _| assert!(p.favicons.contains_key("https://x.example/favicon.ico"), "it still shows"));
    assert_eq!(saved(&b, cx), 0, "an incognito icon is never written");
    assert!(tabs(&b, cx).is_empty(), "nor shared with the regular window");

    // A Space added there would be saved nowhere.
    p.update_in(pcx, |p, window, cx| p.new_space(window, cx));
    p.read_with(pcx, |p, _| assert_eq!(p.session().spaces().len(), 1));

    // Closing the window ends its profile.
    drop(page);
    drop(p);
    pcx.update(|window, _| window.remove_window());
    cx.run_until_parked();
    b.read_with(cx, |b, _| assert!(b.host.deleted.borrow().contains(&profile)));
}

#[gpui::test]
fn regular_windows_share_tabs_and_keep_their_own_selection(cx: &mut TestAppContext) {
    let (a, cx) = browser(cx);
    go(cx, "a.example");
    let a_tab = active(&a, cx);
    let (b, bcx) = open(cx, false);
    assert_eq!(tabs(&b, bcx), ["https://a.example/"]);
    assert_eq!(active(&b, bcx), None, "a new window starts on the command bar, not on the other's tab");

    type_url(bcx, "b.example");
    cx.run_until_parked();
    assert_eq!(tabs(&a, cx), ["https://b.example/", "https://a.example/"], "a tab opened in one is in the other");
    assert_eq!(tabs(&b, bcx), tabs(&a, cx));
    assert_eq!(active(&a, cx), a_tab, "and the other keeps showing its own");

    // A tab closed in one goes in both.
    bcx.dispatch_action(CloseTab);
    bcx.run_until_parked();
    cx.run_until_parked();
    assert_eq!(tabs(&a, cx), ["https://a.example/"]);
    assert_eq!(tabs(&b, bcx), ["https://a.example/"]);
}

#[gpui::test]
fn a_page_lives_in_the_window_that_shows_it(cx: &mut TestAppContext) {
    let (a, cx) = browser(cx);
    go(cx, "a.example");
    let tab = active(&a, cx).unwrap();
    let page = a.read_with(cx, |a, _| a.pages[&tab].0.clone());
    let (b, bcx) = open(cx, false);

    // Picked in the second window: the same page moves there.
    b.update_in(bcx, |b, window, cx| b.activate_tab(tab, window, cx));
    cx.run_until_parked();
    b.read_with(bcx, |b, _| assert_eq!(b.pages[&tab].0, page));
    a.read_with(cx, |a, _| assert!(!a.pages.contains_key(&tab)));
    a.read_with(cx, |a, cx| assert!(a.holder(tab, cx).is_some(), "the first shows it as open in another window"));

    // Anything else happening there leaves it where it is.
    a.update_in(cx, |a, window, cx| a.sync(window, cx));
    b.read_with(bcx, |b, _| assert!(b.pages.contains_key(&tab)));

    // "Show Here" takes it back.
    a.update_in(cx, |a, window, cx| a.bring_here(tab, window, cx));
    a.read_with(cx, |a, _| assert_eq!(a.pages[&tab].0, page));
    b.read_with(bcx, |b, _| assert!(!b.pages.contains_key(&tab)));

    // Once the window holding it closes, the other makes its page again.
    b.update_in(bcx, |b, window, cx| b.bring_here(tab, window, cx));
    cx.run_until_parked();
    drop(b);
    bcx.update(|window, _| window.remove_window());
    cx.run_until_parked();
    a.read_with(cx, |a, _| assert!(a.pages.contains_key(&tab)));
}

#[gpui::test]
fn cmd_n_opens_a_window_except_in_the_chat(cx: &mut TestAppContext) {
    let (a, cx) = browser(cx);
    let windows = |cx: &mut VisualTestContext| cx.cx.windows().len();
    cx.dispatch_action(NewWindow);
    cx.run_until_parked();
    assert_eq!(windows(cx), 2);
    a.update(cx, |a, cx| a.set_claude_enabled(true, cx));
    a.update_in(cx, |a, window, cx| a.toggle_agent_panel(window, cx));
    cx.dispatch_action(NewWindow);
    cx.run_until_parked();
    assert_eq!(windows(cx), 2, "⌘N in the chat is a new chat");
    cx.dispatch_action(NewIncognitoWindow);
    cx.run_until_parked();
    assert_eq!(windows(cx), 3);
    // ⇧⌘W closes the window, ⌘W only a tab.
    drop(a);
    cx.dispatch_action(CloseWindow);
    cx.cx.run_until_parked();
    assert_eq!(cx.cx.windows().len(), 2);
}

#[gpui::test]
fn windows_are_named_by_what_they_show(cx: &mut TestAppContext) {
    let (a, cx) = browser(cx);
    a.read_with(cx, |a, _| assert_eq!(a.window_title(), "Personal"));
    go(cx, "a.example");
    a.read_with(cx, |a, _| assert_eq!(a.window_title(), "https://a.example/"));
    let (p, pcx) = open(cx, true);
    p.read_with(pcx, |p, _| assert_eq!(p.window_title(), "Incognito — Incognito"));
}

#[gpui::test]
fn a_swipe_with_one_space_springs_back(cx: &mut TestAppContext) {
    use gpui::{point, px, ScrollDelta, ScrollWheelEvent, TouchPhase};
    let (a, cx) = browser(cx);
    go(cx, "a.example");
    let (p, pcx) = open(cx, true);
    for (b, cx) in [(a, cx), (p, pcx)] {
        let space = b.read_with(cx, |b, _| b.session().active_space());
        cx.dispatch_action(NextSpace);
        cx.dispatch_action(PrevSpace);
        cx.dispatch_action(GoToSpace(3));
        // Fingers left: towards a next Space there is not.
        for phase in [TouchPhase::Started, TouchPhase::Moved, TouchPhase::Moved, TouchPhase::Ended] {
            let delta = ScrollDelta::Pixels(point(px(-200.), px(0.)));
            let e = ScrollWheelEvent { position: point(px(100.), px(100.)), delta, touch_phase: phase, ..Default::default() };
            b.update_in(cx, |b, window, cx| b.on_chrome_scroll(&e, window, cx));
        }
        b.read_with(cx, |b, _| {
            assert_eq!(b.session().active_space(), space);
            assert_eq!(b.slide.target(), 0.0, "the sidebar springs back, not left part-way");
        });
    }
}

#[gpui::test]
fn a_chat_childs_client_goes_to_the_window_that_started_it(cx: &mut TestAppContext) {
    let (a, cx) = browser(cx);
    a.update(cx, |a, cx| a.set_claude_enabled(true, cx));
    let (b, bcx) = open(cx, false);
    let (p, pcx) = open(cx, true);
    let issue = |w: &B, cx: &mut VisualTestContext, child: i32| {
        w.update(cx, |w, cx| {
            let nonce = crate::agent::issue_nonce(false, cx.background_executor().now());
            w.agent.nonce = Some(crate::agent::Nonce { child: Some(child), ..nonce });
        })
    };
    issue(&b, bcx, 100);
    issue(&p, pcx, 200);
    let owner = |cx: &mut VisualTestContext, ancestors: &'static [i32]| cx.update(|_, cx| crate::agent_window::<MockHost>(ancestors, cx).map(|w| w.window_id()));
    let front = cx.update(|_, cx| crate::front_window::<MockHost>(cx).map(|w| w.window_id()));
    assert_eq!(front, Some(a.read_with(cx, |a, _| a.window.window_id())), "not a window that holds a nonce");
    assert_eq!(owner(cx, &[101, 100, 50]), Some(b.read_with(bcx, |b, _| b.window.window_id())));
    assert_eq!(owner(cx, &[201, 200]), Some(p.read_with(pcx, |p, _| p.window.window_id())), "incognito windows have chats too");
    assert_eq!(owner(cx, &[300, 50]), front, "anyone else is served by the front window");
    b.update(bcx, |b, _| b.agent.nonce = None);
    assert_eq!(owner(cx, &[101, 100]), front);
}

fn claude_busy(w: &B, cx: &mut VisualTestContext) {
    w.update(cx, |w, cx| {
        let conn = w.agent_connect(None, None, None, vec![]);
        w.agent.state.insert(conn, crate::agent::Conn::Open);
        w.agent.nonce = Some(crate::agent::issue_nonce(false, cx.background_executor().now()));
    });
}

fn claude_idle(w: &B, cx: &mut VisualTestContext) -> bool {
    w.read_with(cx, |w, _| !w.agent.connected() && w.agent.state.is_empty() && w.agent.nonce.is_none())
}

#[gpui::test]
fn turning_claude_off_stops_it_in_every_window(cx: &mut TestAppContext) {
    let (a, cx) = browser(cx);
    a.update(cx, |a, cx| a.set_claude_enabled(true, cx));
    let (b, bcx) = open(cx, false);
    let (p, pcx) = open(cx, true);
    claude_busy(&a, cx);
    claude_busy(&b, bcx);
    claude_busy(&p, pcx);
    a.update(cx, |a, cx| a.set_claude_enabled(false, cx));
    cx.run_until_parked();
    assert!(claude_idle(&a, cx) && claude_idle(&b, bcx));
    assert!(claude_idle(&p, pcx), "an incognito window, which never adopts settings, stops too");
    p.read_with(pcx, |p, _| assert!(!p.settings.claude.enabled && p.settings.claude.token.is_empty()));
}

#[gpui::test]
fn report_a_problem_opens_the_issue_in_a_normal_window(cx: &mut TestAppContext) {
    let (a, cx) = browser(cx);
    let (p, pcx) = open(cx, true);
    pcx.dispatch_action(ReportProblem);
    pcx.run_until_parked();
    cx.run_until_parked();
    let new = "https://github.com/dan-hummelstad/baa/issues/new?title=&body=";
    assert!(tabs(&a, cx).first().is_some_and(|u| u.starts_with(new)), "{:?}", tabs(&a, cx));
    assert!(tabs(&p, pcx).is_empty(), "never the incognito window, where nobody is signed in");
    a.read_with(cx, |a, _| assert!(a.host.supplied.borrow().is_empty()));
}

#[gpui::test]
fn importing_is_refused_in_an_incognito_window(cx: &mut TestAppContext) {
    use baa_model::import::{Source, SourceKind};
    let (a, cx) = browser(cx);
    let dir = std::env::temp_dir().join(format!("baa-ui-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sidebar = dir.join("StorableSidebar.json");
    std::fs::write(&sidebar, r#"{"sidebar":{"containers":[{"spaces":[{"title":"Work","containerIDs":[]}]}]}}"#).unwrap();
    let source = || Source { name: "Chrome", history: dir.join("no-such-History"), kind: SourceKind::Chromium };
    let counts = |b: &B, cx: &mut VisualTestContext| b.read_with(cx, |b, _| (b.session().spaces().len(), b.session().profiles().len()));
    let toast = |b: &B, cx: &mut VisualTestContext| b.read_with(cx, |b, _| b.toast.as_ref().map(|t| t.to_string()));

    let (p, pcx) = open(cx, true);
    let before = counts(&p, pcx);
    p.update_in(pcx, |p, window, cx| p.import_arc_from(Some(sidebar.clone()), window, cx));
    assert_eq!(counts(&p, pcx), before, "no Space and no profile from the incognito counter");
    assert_eq!(toast(&p, pcx).as_deref(), Some(crate::tools::IMPORT_PRIVATE));
    p.update(pcx, |p, cx| {
        p.toast = None;
        p.import_history(source(), cx);
    });
    pcx.run_until_parked();
    assert_eq!(toast(&p, pcx).as_deref(), Some(crate::tools::IMPORT_PRIVATE));

    let before = counts(&a, cx);
    a.update_in(cx, |a, window, cx| a.import_arc_from(Some(sidebar.clone()), window, cx));
    assert_eq!(counts(&a, cx), (before.0 + 1, before.1 + 1));
    a.update(cx, |a, cx| a.import_history(source(), cx));
    cx.run_until_parked();
    assert_eq!(toast(&a, cx).as_deref(), Some("Couldn't import history from Chrome"), "it ran, and found no file");
    std::fs::remove_dir_all(&dir).ok();
}
