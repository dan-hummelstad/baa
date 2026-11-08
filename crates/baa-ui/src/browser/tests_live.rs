//! The live folder menu: last fetch, filters and repos.

use baa_model::{
    live::{GitHub, GitHubFilter},
    LiveItem, LiveSource, Store,
};
use baa_web::MenuItem;
use gpui::{Entity, TestAppContext, VisualTestContext};

use super::MenuKind;
use crate::{mock::MockHost, Browser};

fn browser(cx: &mut TestAppContext) -> (Entity<Browser<MockHost>>, &mut VisualTestContext) {
    cx.update(crate::init);
    cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx))
}

/// The menu rows under `path` (submenu labels), as (label, checked, id).
fn rows(menu: &[MenuItem], path: &[&str]) -> Vec<(String, bool, usize)> {
    let mut level = menu.to_vec();
    for name in path {
        level = level
            .into_iter()
            .find_map(|m| match m {
                MenuItem::Submenu { label, items } if label == *name => Some(items),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no submenu {name}"));
    }
    level
        .into_iter()
        .filter_map(|m| match m {
            MenuItem::Action { label, checked, id, .. } => Some((label, checked, id)),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn the_live_folder_menu_filters_and_hides_repos(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    // Made in the session directly: Browser::new_live_folder would fetch from GitHub.
    let folder = b.update(cx, |b, _| {
        let space = b.session.active_space().unwrap();
        let gh = GitHub { filters: vec![GitHubFilter::CreatedByMe], ..Default::default() };
        let id = b.session.add_live_folder(space, "Pull Requests", LiveSource::GitHub(gh));
        let item = |url: &str| LiveItem { url: url.into(), title: url.into() };
        b.live_items.insert(id, vec![item("https://github.com/o/r/pull/1"), item("https://github.com/o/s/pull/2")]);
        b.live_fetched.insert(id, baa_model::now());
        b.show_live_items(id);
        id
    });
    let open = |b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext| {
        b.update_in(cx, |b, window, cx| b.open_menu(MenuKind::Folder(folder), gpui::point(gpui::px(0.), gpui::px(0.)), window, cx));
        cx.run_until_parked();
        b.read_with(cx, |b, _| b.host.menus.borrow().last().unwrap().clone())
    };

    let menu = open(&b, cx);
    let live = rows(&menu, &["Pull Requests"]);
    let labels: Vec<_> = live.iter().map(|(l, c, _)| (l.as_str(), *c)).collect();
    assert_eq!(labels, [
        ("Last fetch: just now", false),
        ("Refresh", false),
        ("Created by Me", true),
        ("Review Requests", false),
        ("Team Review Requests", false),
    ]);
    assert_eq!(rows(&menu, &[])[0].0, "Rename Folder");
    let repos = rows(&menu, &["Pull Requests", "Repositories"]);
    assert_eq!(repos.iter().map(|(l, c, _)| (l.as_str(), *c)).collect::<Vec<_>>(), [("o/r", true), ("o/s", true)]);

    // Unticking o/r hides its pull request without a fetch.
    b.update(cx, |b, _| b.host.menu_pick.set(Some(repos[0].2)));
    open(&b, cx);
    b.read_with(cx, |b, _| {
        let urls: Vec<_> = b.session.folder_tabs(folder).map(|t| t.url.as_str()).collect();
        assert_eq!(urls, ["https://github.com/o/s/pull/2"]);
    });
    let menu = open(&b, cx);
    let repos = rows(&menu, &["Pull Requests", "Repositories"]);
    assert!(!repos[0].1, "o/r is listed unticked, to bring it back");

    // The last filter cannot be unticked: the folder would be all of GitHub.
    let created = rows(&menu, &["Pull Requests"])[2].2;
    b.update(cx, |b, _| b.host.menu_pick.set(Some(created)));
    open(&b, cx);
    b.read_with(cx, |b, _| {
        let Some(LiveSource::GitHub(gh)) = &b.session.folder(folder).unwrap().live else { panic!() };
        assert_eq!(gh.filters, [GitHubFilter::CreatedByMe]);
    });
}
