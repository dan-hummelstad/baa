use super::*;

fn row(label: &str, command: c_int, depth: u8, kind: u8) -> Row {
    Row { label: label.into(), command, depth, kind, enabled: true, checked: false }
}

fn sample_rows() -> Vec<Row> {
    vec![
        row("Open link", 100, 0, 0),
        row("", -1, 0, ffi::MENU_SEPARATOR),
        row("Share", -1, 0, ffi::MENU_SUBMENU),
        row("Mail", 200, 1, 0),
        row("More", -1, 1, ffi::MENU_SUBMENU),
        row("Deep", 201, 2, 0),
        row("Copy", 101, 0, 0),
        row("Inspect", 300, 0, ffi::MENU_INSPECT),
    ]
}

#[test]
fn menu_rows_become_a_tree_with_the_row_index_as_id() {
    let action = |label: &str, id| MenuItem::Action { label: label.into(), enabled: true, checked: false, id };
    assert_eq!(
        menu_tree(&sample_rows()),
        [
            action("Open link", 0),
            MenuItem::Separator,
            MenuItem::Submenu {
                label: "Share".into(),
                items: vec![
                    action("Mail", 3),
                    MenuItem::Submenu { label: "More".into(), items: vec![action("Deep", 5)] },
                ],
            },
            action("Copy", 6),
            action("Inspect", 7),
        ]
    );
    assert_eq!(menu_tree(&[]), []);
    assert_eq!(menu_tree(&[row("lost", 1, 1, 0)]), [], "a row below nothing ends the menu");
}

#[test]
fn a_picked_row_runs_unless_it_is_inspect_on_a_tab() {
    let rows = sample_rows();
    assert_eq!(menu_answer(&rows, Some(6), PageKind::Tab), (101, false));
    assert_eq!(menu_answer(&rows, Some(5), PageKind::Popup), (201, false));
    assert_eq!(menu_answer(&rows, Some(7), PageKind::Tab), (-1, true), "baa's DevTools, not Chrome's");
    assert_eq!(menu_answer(&rows, Some(7), PageKind::Popup), (300, false), "Chrome's window");
    assert_eq!(menu_answer(&rows, Some(7), PageKind::DevTools), (300, false));
    assert_eq!(menu_answer(&rows, None, PageKind::Tab), (-1, false));
    assert_eq!(menu_answer(&rows, Some(99), PageKind::Tab), (-1, false));
}

#[gpui::test]
fn a_menu_for_a_page_that_is_gone_is_closed_at_once_and_never_by_the_callback(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let label = c"Copy";
    let items = [ffi::MenuItem {
        label: label.as_ptr(),
        command: 101,
        depth: 0,
        kind: 0,
        enabled: true,
        checked: false,
    }];
    (env.callbacks.context_menu)(env.ctx(), 0xdead0 as *mut Tab, 9, 1., 2., items.as_ptr(), 1);
    assert_eq!(calls("context_menu_done"), Vec::<String>::new(), "not inside the callback");
    cx.run_until_parked();
    assert_eq!(calls("context_menu_done"), ["context_menu_done 9 -1"]);
}

#[gpui::test]
fn a_download_is_listed_in_place_and_announced_once(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);
    let send = |tab: *mut Tab, id: u32, received: i64, state: c_int| {
        let download = ffi::Download {
            id,
            url: c"https://a.test/f.zip".as_ptr(),
            path: c"/tmp/f.zip".as_ptr(),
            received_bytes: received,
            total_bytes: 10,
            state,
        };
        (env.callbacks.download)(env.ctx(), tab, &download);
    };

    send(tab, 4, 0, 0);
    send(tab, 4, 5, 0);
    send(std::ptr::null_mut(), 5, 0, 0);
    send(tab, 4, 10, 1);
    cx.run_until_parked();

    assert_eq!(*seen.borrow(), [PageEvent::DownloadStarted], "once for id 4; id 5 has no page");
    let items = env.host.downloads().read_with(cx, |list, _| list.items.clone());
    assert_eq!(items.len(), 2);
    assert_eq!((items[0].id, items[0].received_bytes, items[0].state), (4, 10, DownloadState::Complete));
    assert_eq!(items[1].state, DownloadState::InProgress);

    env.host.cancel_download(5u64);
    assert_eq!(calls("download_cancel"), ["download_cancel 5"]);
}
