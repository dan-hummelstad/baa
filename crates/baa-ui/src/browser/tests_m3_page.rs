//! WP-U3 / WP-U4: page fullscreen, find count, crashed page, groups <-> folders.

use baa_model::{Store, Zone};
use baa_web::PageEvent;
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::{mock::{MockHost, MockPage}, Browser};

fn browser(cx: &mut TestAppContext) -> (Entity<Browser<MockHost>>, &mut VisualTestContext) {
    cx.update(crate::init);
    cx.add_window_view(|window, cx| {
        Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx)
    })
}

fn open_page(b: &Entity<Browser<MockHost>>, url: &str, cx: &mut VisualTestContext) -> Entity<MockPage> {
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input(url);
    cx.simulate_keystrokes("enter");
    b.read_with(cx, |b, _| b.pages.get(&b.active_tab().unwrap()).unwrap().0.clone())
}

fn emit(page: &Entity<MockPage>, event: PageEvent, cx: &mut VisualTestContext) {
    page.update(cx, |_, cx| cx.emit(event));
}

fn sync(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) {
    b.update_in(cx, |b, window, cx| b.sync(window, cx));
}

#[gpui::test]
fn fullscreen_ends_when_the_tab_changes(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let page = open_page(&b, "https://a.example/", cx);
    emit(&page, PageEvent::Fullscreen(true), cx);
    let tab = b.read_with(cx, |b, _| b.active_tab());
    assert_eq!(b.read_with(cx, |b, _| b.fullscreen), tab);
    open_page(&b, "https://b.example/", cx);
    assert_eq!(b.read_with(cx, |b, _| b.fullscreen), None);
    assert_eq!(page.read_with(cx, |p, _| p.exited_fullscreen.get()), 1);
}

#[gpui::test]
fn page_leaving_fullscreen_clears_it_without_telling_it_back(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let page = open_page(&b, "https://a.example/", cx);
    emit(&page, PageEvent::Fullscreen(true), cx);
    emit(&page, PageEvent::Fullscreen(false), cx);
    assert_eq!(b.read_with(cx, |b, _| b.fullscreen), None);
    assert_eq!(page.read_with(cx, |p, _| p.exited_fullscreen.get()), 0);
}

#[gpui::test]
fn find_result_is_kept_for_the_bar(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let page = open_page(&b, "https://a.example/", cx);
    emit(&page, PageEvent::FindResult { active: 3, total: 12 }, cx);
    assert_eq!(b.read_with(cx, |b, _| b.page_state.find_result), Some((3, 12)));
}

#[gpui::test]
fn a_crashed_page_reloads_from_its_overlay(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let page = open_page(&b, "https://a.example/", cx);
    emit(&page, PageEvent::Crashed(true), cx);
    b.update(cx, |b, cx| b.reload_crashed(cx));
    assert_eq!(page.read_with(cx, |p, _| p.reloads.get()), 1);
    emit(&page, PageEvent::Crashed(false), cx);
    assert!(b.read_with(cx, |b, _| b.page_state.crashed.is_empty()));
}

#[gpui::test]
fn a_tab_in_a_folder_becomes_a_group_once(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let page = open_page(&b, "https://a.example/", cx);
    let (tab, folder) = b.update(cx, |b, _| {
        let tab = b.active_tab().unwrap();
        let space = b.session.active_space().unwrap();
        let folder = b.session.add_folder(space, "Work");
        b.session.move_to_folder(tab, folder, 0);
        (tab, folder)
    });
    let _ = tab;
    sync(&b, cx);
    sync(&b, cx);
    assert_eq!(*page.read_with(cx, |p, _| p.groups.borrow().clone()), [Some((folder.0, "Work".to_string()))]);
    b.update(cx, |b, _| b.session.folder_mut(folder).unwrap().name = "Job".into());
    sync(&b, cx);
    let groups = page.read_with(cx, |p, _| p.groups.borrow().clone());
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[1], Some((folder.0, "Job".to_string())));
}

#[gpui::test]
fn an_extension_group_becomes_a_pinned_folder(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let one = open_page(&b, "https://a.example/", cx);
    let key = 1u64 << 63 | 9;
    emit(&one, PageEvent::GroupChanged { group: Some(key), title: "News".into() }, cx);
    assert!(one.read_with(cx, |p, _| p.groups.borrow().is_empty()), "no echo");
    let (folder, space) = b.read_with(cx, |b, _| {
        let space = b.session.active_space().unwrap();
        let tab = b.session.tab(b.active_tab().unwrap()).unwrap();
        assert_eq!(tab.zone, Zone::Pinned);
        let f = b.session.folder(tab.folder.unwrap()).unwrap();
        assert_eq!(f.name, "News");
        (f.id, space)
    });
    let two = open_page(&b, "https://b.example/", cx);
    emit(&two, PageEvent::GroupChanged { group: Some(key), title: "News".into() }, cx);
    b.read_with(cx, |b, _| {
        assert_eq!(b.session.folders(space).count(), 1);
        assert_eq!(b.session.folder_tabs(folder).count(), 2);
    });
    assert!(two.read_with(cx, |p, _| p.groups.borrow().is_empty()));
    emit(&two, PageEvent::GroupChanged { group: None, title: String::new() }, cx);
    b.read_with(cx, |b, _| {
        assert_eq!(b.session.folder_tabs(folder).count(), 1);
        assert_eq!(b.session.tab(b.active_tab().unwrap()).unwrap().zone, Zone::Pinned);
    });
}
