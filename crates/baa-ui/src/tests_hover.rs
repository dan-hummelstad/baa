//! The hovered link's URL: a pill over the page that only ever shows it.

use baa_model::Store;
use baa_web::{Dialog, DialogKind, PageEvent};
use gpui::{point, px, Bounds, Entity, Modifiers, Pixels, TestAppContext, VisualTestContext};

use crate::{actions::SplitView, mock::{MockHost, MockPage}, Browser};

fn page(cx: &mut TestAppContext) -> (Entity<Browser<MockHost>>, Entity<MockPage>, &mut VisualTestContext) {
    cx.update(crate::init);
    let (b, cx) = cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx));
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("a.example");
    cx.simulate_keystrokes("enter");
    let p = b.read_with(cx, |b, _| b.pages[&b.active_tab().unwrap()].0.clone());
    (b, p, cx)
}

fn hover(p: &Entity<MockPage>, url: &str, cx: &mut VisualTestContext) {
    p.update(cx, |_, cx| cx.emit(PageEvent::TargetUrl(url.into())));
    cx.run_until_parked();
}

fn pill(cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    cx.run_until_parked();
    cx.debug_bounds("hover-url")
}

fn size(cx: &mut VisualTestContext) -> gpui::Size<Pixels> {
    cx.update(|window, _| window.viewport_size())
}

fn width(cx: &mut VisualTestContext, selector: &'static str) -> f32 {
    f32::from(cx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} is drawn")).size.width)
}

#[gpui::test]
fn a_long_link_keeps_its_host_whole_and_cuts_the_path(cx: &mut TestAppContext) {
    let (_, p, cx) = page(cx);
    hover(&p, "https://docs.example.com/a", cx);
    let host = width(cx, "hover-host");
    assert!(host > 0.);
    hover(&p, &format!("https://docs.example.com/{}?q=1", "path/".repeat(400)), cx);
    let pill = pill(cx).expect("the pill shows");
    assert_eq!(width(cx, "hover-host"), host, "the host is not cut");
    let rest = cx.debug_bounds("hover-rest").unwrap();
    assert!(rest.size.width > px(0.) && rest.right() <= pill.right(), "the path fills the rest and is cut");
    let window = size(cx);
    assert!(pill.right() < window.width * 0.7, "the pill takes part of the page, not all of it");
    assert!(pill.bottom() > window.height * 0.8 && pill.left() < window.width * 0.5, "bottom-left: {pill:?}");
}

#[gpui::test]
fn a_host_too_long_for_the_pill_is_all_that_shows_and_stays_inside_it(cx: &mut TestAppContext) {
    let (_, p, cx) = page(cx);
    let long = format!("{}.evil.example", "a".repeat(600));
    hover(&p, &format!("https://{long}/login"), cx);
    let pill = pill(cx).expect("the pill shows");
    let host = cx.debug_bounds("hover-host").unwrap();
    assert!(host.left() >= pill.left() && host.right() <= pill.right(), "cut to the pill: {host:?} in {pill:?}");
    assert_eq!(width(cx, "hover-rest"), 0., "no room left for the path");
}

#[gpui::test]
fn other_schemes_show_whole_without_a_host(cx: &mut TestAppContext) {
    let (_, p, cx) = page(cx);
    hover(&p, "mailto:me@a.example", cx);
    assert!(pill(cx).is_some());
    assert!(cx.debug_bounds("hover-host").is_none());
}

fn alert() -> Dialog {
    Dialog {
        id: 1,
        kind: DialogKind::Alert,
        message: "hi".into(),
        default_text: String::new(),
        title: String::new(),
        ok_label: String::new(),
        cancel_label: String::new(),
        check_label: String::new(),
    }
}

#[gpui::test]
fn a_prompt_hides_it(cx: &mut TestAppContext) {
    let (_, p, cx) = page(cx);
    hover(&p, "https://b.example/", cx);
    assert!(pill(cx).is_some());
    p.update(cx, |_, cx| cx.emit(PageEvent::Dialog(alert())));
    assert!(pill(cx).is_none(), "not beside a dialog");
    hover(&p, "https://c.example/", cx);
    assert!(pill(cx).is_none(), "nor shown while one is up");
}

#[gpui::test]
fn a_permission_question_hides_it(cx: &mut TestAppContext) {
    let (_, p, cx) = page(cx);
    p.update(cx, |_, cx| {
        cx.emit(PageEvent::PermissionRequested { id: 3, origin: "https://a.example".into(), kinds: vec![baa_web::Permission::Camera] })
    });
    hover(&p, "https://b.example/", cx);
    assert!(pill(cx).is_none());
}

#[gpui::test]
fn it_clears_when_the_link_or_the_page_goes(cx: &mut TestAppContext) {
    let (b, p, cx) = page(cx);
    type Clear = fn(&Entity<MockPage>, &mut VisualTestContext);
    let clears: [(&str, Clear); 3] = [
        ("an empty URL", |p, cx| hover(p, "", cx)),
        ("a navigation", |p, cx| p.update(cx, |_, cx| cx.emit(PageEvent::UrlChanged("https://n.example/".into())))),
        ("a load starting", |p, cx| p.update(cx, |_, cx| cx.emit(PageEvent::LoadingChanged(true)))),
    ];
    for (what, clear) in clears {
        hover(&p, "https://b.example/x", cx);
        assert!(pill(cx).is_some(), "{what}: shown first");
        clear(&p, cx);
        assert!(pill(cx).is_none(), "{what} clears it");
    }

    hover(&p, "https://b.example/x", cx);
    let at = pill(cx).unwrap().center();
    cx.simulate_mouse_move(at, None, Modifiers::none());
    assert!(pill(cx).is_some(), "the pointer is still on the page");
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
    assert!(pill(cx).is_none(), "the pointer left the page");

    hover(&p, "https://b.example/x", cx);
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("other.example");
    cx.simulate_keystrokes("enter");
    assert!(b.read_with(cx, |b, _| b.hover_url.is_none()), "another tab was switched to");
    let first = b.read_with(cx, |b, _| *b.pages.iter().find(|(_, (q, _))| q == &p).unwrap().0);
    b.update_in(cx, |b, window, cx| b.activate_tab(first, window, cx));
    assert!(pill(cx).is_none(), "and it does not come back with the tab");

    hover(&p, "https://b.example/x", cx);
    b.update_in(cx, |b, window, cx| b.close_tab_id(first, window, cx));
    assert!(b.read_with(cx, |b, _| b.hover_url.is_none()), "the tab closed");
}

#[gpui::test]
fn in_split_view_it_shows_in_the_pane_of_its_tab(cx: &mut TestAppContext) {
    let (b, first, cx) = page(cx);
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("b.example");
    cx.simulate_keystrokes("enter");
    cx.dispatch_action(SplitView);
    cx.run_until_parked();
    let (left, right) = b.read_with(cx, |b, _| {
        let s = b.session();
        let tabs = s.shown_split(s.active_space().unwrap()).expect("a split").tabs.clone();
        (b.pages[&tabs[0]].0.clone(), b.pages[&tabs[1]].0.clone())
    });
    assert!(left == first || right == first);
    let half = size(cx).width * 0.5;
    hover(&right, "https://r.example/", cx);
    assert!(pill(cx).unwrap().left() > half, "in the right pane");
    hover(&left, "https://l.example/", cx);
    assert!(pill(cx).unwrap().right() < half, "in the left pane");
}

#[gpui::test]
fn a_hovered_url_is_never_loaded(cx: &mut TestAppContext) {
    let (b, p, cx) = page(cx);
    let url = "https://evil.example/x";
    hover(&p, url, cx);
    let at = pill(cx).unwrap().center();
    cx.simulate_click(at, Modifiers::none());
    cx.simulate_click(at, Modifiers::secondary_key());
    cx.run_until_parked();
    b.read_with(cx, |b, cx| {
        for (page, _) in b.pages.values() {
            assert!(!page.read(cx).loads.borrow().iter().any(|l| l.contains("evil")), "loaded");
        }
        assert!(!b.host.created.borrow().iter().any(|(_, u)| u.contains("evil")), "opened");
        assert!(b.pages.keys().all(|id| !b.session().tab(*id).unwrap().url.contains("evil")), "a tab");
    });
}
