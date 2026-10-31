//! Browser view driven through real keystrokes against `MockHost`.

use baa_model::{Store, Zone};
use baa_web::{PageEvent, WebHost};
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::{actions::*, mock::MockHost, Browser};

fn browser(cx: &mut TestAppContext) -> (Entity<Browser<MockHost>>, &mut VisualTestContext) {
    cx.update(crate::init);
    cx.add_window_view(|window, cx| {
        Browser::new(
            MockHost::new(cx),
            Store::open(":memory:").unwrap(),
            window,
            cx,
        )
    })
}

fn urls(b: &Entity<Browser<MockHost>>, zone: Zone, cx: &mut VisualTestContext) -> Vec<String> {
    b.read_with(cx, |b, _| {
        let s = b.session();
        s.tabs(s.active_space().unwrap(), zone)
            .map(|t| t.url.clone())
            .collect()
    })
}

#[gpui::test]
fn command_bar_opens_tabs_and_creates_pages(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("example.com");
    cx.simulate_keystrokes("enter");
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("rust lang");
    cx.simulate_keystrokes("enter");

    assert_eq!(
        urls(&b, Zone::Today, cx),
        [
            "https://www.google.com/search?q=rust+lang",
            "https://example.com/"
        ]
    );
    b.read_with(cx, |b, cx| {
        assert!(b.bar.is_none());
        assert_eq!(b.host.created.borrow().len(), 2);
        let visible: Vec<bool> = b
            .pages
            .values()
            .map(|(p, _)| p.read(cx).visible.get())
            .collect();
        assert_eq!(visible.iter().filter(|v| **v).count(), 1);
    });
}

#[gpui::test]
fn address_mode_navigates_in_place(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("a.example");
    cx.simulate_keystrokes("enter");
    cx.simulate_keystrokes("secondary-l");
    // Prefilled URL is selected: typing replaces it.
    cx.simulate_input("b.example");
    cx.simulate_keystrokes("enter");
    assert_eq!(urls(&b, Zone::Today, cx), ["https://b.example/"]);
    b.read_with(cx, |b, cx| {
        let (page, _) = b.pages.values().next().unwrap();
        assert_eq!(page.read(cx).url.borrow().as_str(), "https://b.example/");
    });
}

#[gpui::test]
fn close_reopen_pin_and_tab_navigation(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    for host in ["a.example", "b.example", "c.example"] {
        cx.simulate_keystrokes("secondary-t");
        cx.simulate_input(host);
        cx.simulate_keystrokes("enter");
    }
    cx.dispatch_action(GoToTab(2));
    cx.dispatch_action(TogglePin);
    assert_eq!(urls(&b, Zone::Pinned, cx), ["https://a.example/"]);

    cx.dispatch_action(LastTab);
    cx.dispatch_action(CloseTab);
    assert_eq!(urls(&b, Zone::Today, cx), ["https://c.example/"]);
    cx.dispatch_action(ReopenClosedTab);
    assert_eq!(
        urls(&b, Zone::Today, cx),
        ["https://c.example/", "https://b.example/"]
    );
    b.read_with(cx, |b, _| assert_eq!(b.pages.len(), 3));
}

#[gpui::test]
fn spaces_isolate_profiles_and_delete_cleanly(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    cx.dispatch_action(NewSpace);
    // New Space opens the rename bar.
    cx.simulate_input("💼 Work");
    cx.simulate_keystrokes("enter");
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("work.example");
    cx.simulate_keystrokes("enter");
    b.read_with(cx, |b, _| {
        let s = b.session();
        let work = s.space(s.active_space().unwrap()).unwrap();
        assert_eq!((work.icon.as_str(), work.name.as_str()), ("💼", "Work"));
        assert_eq!(s.spaces().len(), 2);
        assert_ne!(s.spaces()[0].profile, work.profile);
        assert_eq!(b.host.created.borrow()[0].0, work.profile);
    });

    cx.dispatch_action(GoToSpace(0));
    b.read_with(cx, |b, _| {
        assert_eq!(b.session().active_space(), Some(b.session().spaces()[0].id))
    });

    cx.dispatch_action(NextSpace);
    let work_profile = b.read_with(cx, |b, _| b.active_space().unwrap().profile);
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("delete space");
    cx.simulate_keystrokes("down enter");
    b.read_with(cx, |b, _| {
        assert_eq!(b.session().spaces().len(), 1);
        assert!(b.pages.is_empty(), "work page dropped");
        assert_eq!(*b.host.deleted.borrow(), [work_profile]);
    });
}

#[gpui::test]
fn incognito_space_uses_ephemeral_profile(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    cx.dispatch_action(NewIncognitoSpace);
    cx.simulate_input("x.example");
    cx.simulate_keystrokes("enter");
    b.read_with(cx, |b, _| {
        let space = b.active_space().unwrap();
        assert!(space.ephemeral);
        assert_eq!(b.host.created.borrow()[0].0, b.host.ephemeral_profile());
    });
}

#[gpui::test]
fn zoom_steps_the_ladder_per_tab(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    for host in ["a.example", "b.example"] {
        cx.simulate_keystrokes("secondary-t");
        cx.simulate_input(host);
        cx.simulate_keystrokes("enter");
    }
    let zooms = |b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext| {
        b.read_with(cx, |b, cx| {
            let mut z: Vec<f64> = b
                .pages
                .values()
                .map(|(p, _)| p.read(cx).zoom.get())
                .collect();
            z.sort_by(f64::total_cmp);
            z
        })
    };
    cx.dispatch_action(ZoomIn);
    cx.dispatch_action(ZoomIn);
    assert_eq!(zooms(&b, cx), [1.0, 1.25]);
    for _ in 0..4 {
        cx.dispatch_action(ZoomOut);
    }
    assert_eq!(zooms(&b, cx), [0.8, 1.0]);
    cx.dispatch_action(ZoomReset);
    assert_eq!(zooms(&b, cx), [1.0, 1.0]);
}

#[gpui::test]
fn page_events_update_the_session(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("a.example");
    cx.simulate_keystrokes("enter");
    let page = b.read_with(cx, |b, _| b.pages.values().next().unwrap().0.clone());
    page.update(cx, |_, cx| {
        cx.emit(PageEvent::TitleChanged("A".into()));
        cx.emit(PageEvent::UrlChanged("https://a.example/next".into()));
        cx.emit(PageEvent::LoadingChanged(true));
        cx.emit(PageEvent::OpenRequested {
            url: "https://popup.example".into(),
            from_user_gesture: true,
            background: true,
        });
    });
    cx.run_until_parked();
    b.read_with(cx, |b, _| {
        let s = b.session();
        let tabs: Vec<_> = s.tabs(s.active_space().unwrap(), Zone::Today).collect();
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].url, "https://popup.example");
        assert_eq!(
            (tabs[1].title.as_str(), tabs[1].url.as_str()),
            ("A", "https://a.example/next")
        );
    });
    page.update(cx, |_, cx| cx.emit(PageEvent::Closed));
    cx.run_until_parked();
    assert_eq!(urls(&b, Zone::Today, cx), ["https://popup.example"]);
}

#[gpui::test]
fn session_survives_restart_and_restores_lazily(cx: &mut TestAppContext) {
    let path = std::env::temp_dir().join(format!("baa-ui-restart-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    cx.update(crate::init);
    let open = |cx: &mut TestAppContext| {
        let path = path.clone();
        cx.add_window_view(move |window, cx| {
            Browser::new(MockHost::new(cx), Store::open(path).unwrap(), window, cx)
        })
        .0
    };
    let first = open(cx);
    let vcx = &mut VisualTestContext::from_window(*cx.windows().last().unwrap(), cx);
    for host in ["a.example", "b.example"] {
        vcx.simulate_keystrokes("secondary-t");
        vcx.simulate_input(host);
        vcx.simulate_keystrokes("enter");
    }
    drop(first);

    let second = open(cx);
    second.read_with(cx, |b, _| {
        let s = b.session();
        let urls: Vec<_> = s
            .tabs(s.active_space().unwrap(), Zone::Today)
            .map(|t| t.url.as_str())
            .collect();
        assert_eq!(urls, ["https://b.example/", "https://a.example/"]);
        // Only the active tab gets a page at launch.
        assert_eq!(
            *b.host.created.borrow(),
            [(
                b.active_space().unwrap().profile,
                "https://b.example/".to_string()
            )]
        );
    });
    let _ = std::fs::remove_file(&path);
}

#[gpui::test]
fn collapsed_sidebar_reveals_from_the_window_edge(cx: &mut TestAppContext) {
    use gpui::{point, px, Modifiers};
    let (b, cx) = browser(cx);
    cx.dispatch_action(ToggleSidebar);
    cx.run_until_parked();

    cx.simulate_mouse_move(point(px(2.), px(300.)), None, Modifiers::none());
    assert!(
        b.read_with(cx, |b, _| b.reveal_hovered()),
        "edge hover opens the card"
    );

    // The inset gap at the edge and a little past the card keep it out.
    for x in [0., 5., 280.] {
        cx.simulate_mouse_move(point(px(x), px(300.)), None, Modifiers::none());
        assert!(b.read_with(cx, |b, _| b.reveal_hovered()), "still out at x = {x}");
    }

    // Leaving it closes it only after the linger delay.
    cx.simulate_mouse_move(point(px(700.), px(300.)), None, Modifiers::none());
    assert!(!b.read_with(cx, |b, _| b.reveal_hovered()));

    // Further out than the edge zone does not open it.
    cx.simulate_mouse_move(point(px(40.), px(300.)), None, Modifiers::none());
    assert!(!b.read_with(cx, |b, _| b.reveal_hovered()));
}

#[gpui::test]
fn links_that_open_a_tab_peek_and_expand_into_one(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("a.example");
    cx.simulate_keystrokes("enter");
    let page = b.read_with(cx, |b, _| b.pages.values().next().unwrap().0.clone());
    page.update(cx, |_, cx| {
        cx.emit(PageEvent::OpenRequested {
            url: "https://peek.example".into(),
            from_user_gesture: true,
            background: false,
        })
    });
    cx.run_until_parked();
    // A preview, not a tab.
    assert_eq!(urls(&b, Zone::Today, cx), ["https://a.example/"]);
    let peek = b.read_with(cx, |b, _| b.peek.as_ref().unwrap().page.clone());
    assert_eq!(peek.read_with(cx, |p, _| p.radius.get()), 12.0);
    assert!(peek.read_with(cx, |p, _| p.floating.get()), "over the page's native view");

    cx.dispatch_action(ExpandPeek);
    cx.run_until_parked();
    assert_eq!(urls(&b, Zone::Today, cx), ["https://peek.example", "https://a.example/"]);
    b.read_with(cx, |b, cx| {
        assert!(b.peek.is_none());
        // The same live page, now a tab, with the page card's corners.
        let tab = b.active_tab().unwrap();
        assert_eq!(b.pages[&tab].0, peek);
        assert_eq!(peek.read(cx).radius.get(), 8.0);
        assert_eq!(peek.read(cx).moved.get(), 1);
        assert!(!peek.read(cx).floating.get());
    });
}

#[gpui::test]
fn escape_closes_a_peek_and_background_links_stay_behind(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("a.example");
    cx.simulate_keystrokes("enter");
    let page = b.read_with(cx, |b, _| b.pages.values().next().unwrap().0.clone());
    page.update(cx, |_, cx| {
        cx.emit(PageEvent::OpenRequested {
            url: "https://later.example".into(),
            from_user_gesture: true,
            background: true,
        });
        cx.emit(PageEvent::OpenRequested {
            url: "https://peek.example".into(),
            from_user_gesture: true,
            background: false,
        });
    });
    cx.run_until_parked();
    b.read_with(cx, |b, _| {
        let s = b.session();
        assert_eq!(
            s.tab(b.active_tab().unwrap()).unwrap().url,
            "https://a.example/",
            "a background tab does not take over"
        );
        assert!(b.peek.is_some());
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(b.read_with(cx, |b, _| b.peek.is_none()));
    assert_eq!(urls(&b, Zone::Today, cx), ["https://later.example", "https://a.example/"]);
}

#[gpui::test]
fn external_links_open_in_lil_baa_and_move_into_the_browser(cx: &mut TestAppContext) {
    use crate::lil_baa::LilBaa;
    let (b, vcx) = browser(cx);
    b.update(vcx, |b, cx| b.open_external("https://elsewhere.example", cx));
    vcx.run_until_parked();
    assert!(urls(&b, Zone::Today, vcx).is_empty(), "not a tab yet");
    let lil = cx
        .windows()
        .into_iter()
        .find_map(|w| w.downcast::<LilBaa<MockHost>>())
        .expect("a lil-baa window");
    lil.update(cx, |lil, window, cx| lil.open_in_browser(window, cx)).unwrap();
    cx.run_until_parked();
    assert!(lil.update(cx, |_, _, _| ()).is_err(), "the small window closed");
    let vcx = &mut VisualTestContext::from_window(b.read_with(cx, |b, _| b.window_handle()), cx);
    assert_eq!(urls(&b, Zone::Today, vcx), ["https://elsewhere.example"]);
    b.read_with(vcx, |b, cx| {
        let page = &b.pages[&b.active_tab().unwrap()].0;
        assert_eq!(page.read(cx).moved.get(), 1, "the live page moved windows");
    });
}

#[gpui::test]
fn a_new_space_is_named_in_place(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    cx.dispatch_action(NewSpace);
    cx.run_until_parked();
    assert!(b.read_with(cx, |b, _| b.renaming.is_some() && b.bar.is_none()));
    cx.simulate_input("Work");
    cx.simulate_keystrokes("enter");
    b.read_with(cx, |b, _| {
        assert!(b.renaming.is_none());
        assert_eq!(b.active_space().unwrap().name, "Work");
    });
}

#[gpui::test]
fn a_two_finger_swipe_switches_spaces(cx: &mut TestAppContext) {
    use gpui::{point, px, Modifiers, ScrollDelta, ScrollWheelEvent, TouchPhase};
    let (b, cx) = browser(cx);
    cx.dispatch_action(NewSpace);
    cx.simulate_keystrokes("escape");
    let spaces = b.read_with(cx, |b, _| {
        b.session().spaces().iter().map(|s| s.id).collect::<Vec<_>>()
    });
    let swipe = |phase, dx: f32| ScrollWheelEvent {
        position: point(px(100.), px(300.)),
        delta: ScrollDelta::Pixels(point(px(dx), px(0.))),
        modifiers: Modifiers::none(),
        touch_phase: phase,
    };
    b.update_in(cx, |b, window, cx| {
        b.on_chrome_scroll(&swipe(TouchPhase::Started, 0.), window, cx);
        b.on_chrome_scroll(&swipe(TouchPhase::Moved, 120.), window, cx);
        b.on_chrome_scroll(&swipe(TouchPhase::Ended, 0.), window, cx);
    });
    assert_eq!(b.read_with(cx, |b, _| b.session().active_space()), Some(spaces[0]));
}

#[gpui::test]
fn typing_fetches_suggestions_once_it_pauses(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    b.read_with(cx, |b, _| {
        *b.host.suggest_body.borrow_mut() = Some(r#"["gh",["gh cli","gh pages"]]"#.into())
    });
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("gh");
    cx.executor().advance_clock(std::time::Duration::from_millis(200));
    cx.run_until_parked();
    b.read_with(cx, |b, _| {
        // One request for the settled text, not one per keystroke.
        assert_eq!(b.host.fetched.borrow().len(), 1);
        assert!(b.host.fetched.borrow()[0].contains("q=gh"));
        assert_eq!(b.bar.as_ref().unwrap().suggestions, ["gh cli", "gh pages"]);
    });
    cx.simulate_keystrokes("down enter");
    assert_eq!(urls(&b, Zone::Today, cx), ["https://www.google.com/search?q=gh+cli"]);
}

#[gpui::test]
fn folders_are_named_in_place_and_favorites_follow_every_space(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("a.example");
    cx.simulate_keystrokes("enter");
    let tab = b.read_with(cx, |b, _| b.active_tab().unwrap());

    b.update_in(cx, |b, window, cx| b.new_folder(Some(tab), window, cx));
    cx.simulate_input("Reading");
    cx.simulate_keystrokes("enter");
    b.read_with(cx, |b, _| {
        let s = b.session();
        let folder = s.folders(s.active_space().unwrap()).next().unwrap();
        assert_eq!(folder.name, "Reading");
        assert_eq!(s.folder_tabs(folder.id).next().unwrap().id, tab);
        assert!(b.renaming.is_none());
    });

    b.update_in(cx, |b, window, cx| {
        let space = b.session.active_space().unwrap();
        b.session.move_tab(tab, space, Zone::Favorite, 0);
        b.new_space(window, cx);
        b.close_overlays(window, cx);
        b.activate_tab(tab, window, cx);
    });
    b.read_with(cx, |b, _| {
        let s = b.session();
        assert_eq!(s.active_space(), Some(s.spaces()[1].id), "a favourite opens where you are");
        assert_eq!(b.active_tab(), Some(tab));
    });
}

#[gpui::test]
fn rows_grow_in_and_collapse_out(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("a.example");
    cx.simulate_keystrokes("enter");
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("b.example");
    cx.simulate_keystrokes("enter");
    let tab = b.read_with(cx, |b, _| {
        let tab = b.active_tab().unwrap();
        assert!(b.motion.get(("tab-in", tab.0), crate::motion::Feel::Glide) > 0.9, "a new row starts hidden");
        tab
    });
    cx.simulate_keystrokes("secondary-w");
    b.read_with(cx, |b, _| {
        let ghost = b.ghosts.iter().find(|g| g.id == tab).expect("the closed row lingers");
        assert_eq!((ghost.list.1, ghost.index), (Zone::Today, 0));
    });
}

#[gpui::test]
fn copying_the_url_flashes_a_check(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("a.example");
    cx.simulate_keystrokes("enter");
    cx.dispatch_action(CopyUrl);
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("https://a.example/")
    );
    let copied = |cx: &mut VisualTestContext| {
        b.read_with(cx, |b, _| b.motion.springs_target(("copied", 0), crate::motion::Feel::Glide))
    };
    assert_eq!(copied(cx), Some(1.0));
    cx.executor().advance_clock(std::time::Duration::from_millis(1300));
    cx.run_until_parked();
    assert_eq!(copied(cx), Some(0.0), "the check goes away on its own");
}

fn open_page(b: &Entity<Browser<MockHost>>, url: &str, cx: &mut VisualTestContext) -> Entity<crate::mock::MockPage> {
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input(url);
    cx.simulate_keystrokes("enter");
    b.read_with(cx, |b, _| b.pages.get(&b.active_tab().unwrap()).unwrap().0.clone())
}

#[gpui::test]
fn page_dialogs_take_the_keyboard_and_answer_the_page(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let page = open_page(&b, "a.example", cx);
    page.update(cx, |_, cx| {
        cx.emit(PageEvent::Dialog {
            kind: baa_web::DialogKind::Prompt,
            message: "Name?".into(),
            default_text: "old".into(),
        })
    });
    cx.run_until_parked();
    // The default text is selected, so typing replaces it.
    cx.simulate_input("sam");
    cx.simulate_keystrokes("enter");
    page.update(cx, |_, cx| {
        cx.emit(PageEvent::Dialog {
            kind: baa_web::DialogKind::Confirm,
            message: "Sure?".into(),
            default_text: String::new(),
        })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    page.read_with(cx, |p, _| assert_eq!(*p.answers.borrow(), ["dialog true sam", "dialog false "]));
    b.read_with(cx, |b, _| assert!(b.active_dialog().is_none()));
}

#[gpui::test]
fn permission_requests_wait_for_an_answer(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let page = open_page(&b, "a.example", cx);
    page.update(cx, |_, cx| {
        for id in [7, 8] {
            cx.emit(PageEvent::PermissionRequested {
                id,
                origin: "https://a.example".into(),
                kinds: vec![baa_web::Permission::Camera],
            })
        }
        cx.emit(PageEvent::PermissionDismissed(8));
    });
    cx.run_until_parked();
    b.update(cx, |b, cx| b.answer_permission(true, cx));
    page.read_with(cx, |p, _| assert_eq!(*p.answers.borrow(), ["permission 7 true"]));
    b.read_with(cx, |b, _| assert!(b.asks.values().all(|a| a.permissions.is_empty())));
}

#[gpui::test]
fn playing_tabs_can_be_muted(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let page = open_page(&b, "a.example", cx);
    page.update(cx, |_, cx| cx.emit(PageEvent::AudibleChanged(true)));
    cx.run_until_parked();
    b.read_with(cx, |b, _| assert_eq!(b.audible.len(), 1));
    cx.dispatch_action(MuteTab);
    page.read_with(cx, |p, _| assert!(p.muted.get()));
    cx.dispatch_action(MuteTab);
    page.read_with(cx, |p, _| assert!(!p.muted.get()));
}

#[gpui::test]
fn visits_are_remembered_and_searchable(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let page = open_page(&b, "a.example", cx);
    page.update(cx, |_, cx| {
        cx.emit(PageEvent::UrlChanged("https://a.example/".into()));
        cx.emit(PageEvent::TitleChanged("Alpha".into()));
    });
    cx.run_until_parked();
    cx.dispatch_action(ShowHistory);
    cx.run_until_parked();
    b.read_with(cx, |b, _| {
        let bar = b.bar.as_ref().unwrap();
        assert_eq!(bar.mode, crate::command_bar::Mode::History);
        assert_eq!(bar.history[0].title, "Alpha");
    });
    cx.simulate_input("zzz");
    b.read_with(cx, |b, _| assert!(b.bar.as_ref().unwrap().history.is_empty()));
}

#[gpui::test]
fn downloads_show_in_the_header_and_settings_open(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let list = b.read_with(cx, |b, _| b.host.downloads());
    list.update(cx, |list, cx| {
        list.upsert(baa_web::Download {
            id: 1,
            url: "https://a.example/f.zip".into(),
            path: "/tmp/f.zip".into(),
            received_bytes: 5,
            total_bytes: 10,
            state: baa_web::DownloadState::InProgress,
        });
        cx.notify();
    });
    cx.run_until_parked();
    b.read_with(cx, |b, _| assert_eq!(b.downloads_seen, 1));
    cx.dispatch_action(OpenSettings);
    b.read_with(cx, |b, _| assert!(b.settings_open));
}

#[gpui::test]
fn finished_downloads_are_kept_in_the_library(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let list = b.read_with(cx, |b, _| b.host.downloads());
    for state in [baa_web::DownloadState::InProgress, baa_web::DownloadState::Complete, baa_web::DownloadState::Complete] {
        list.update(cx, |list, cx| {
            list.upsert(baa_web::Download {
                id: 1,
                url: "https://a.example/f.zip".into(),
                path: "/tmp/f.zip".into(),
                received_bytes: 10,
                total_bytes: 10,
                state,
            });
            cx.notify();
        });
        cx.run_until_parked();
    }
    b.read_with(cx, |b, _| {
        assert_eq!(b.saved_downloads.len(), 1, "saved once, when it finished");
        assert_eq!(b.store().unwrap().downloads().unwrap()[0].path, "/tmp/f.zip");
    });
    cx.dispatch_action(ShowDownloads);
    b.read_with(cx, |b, _| {
        assert_eq!(b.library.as_ref().map(|l| l.section), Some(crate::library::Section::Downloads));
        assert_eq!(b.sidebar_width(), crate::library::LIBRARY_WIDTH);
    });
    cx.dispatch_action(ShowDownloads);
    b.read_with(cx, |b, _| assert!(b.library.is_none()));
}

#[gpui::test]
fn an_opening_folder_pushes_today_down_with_it(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    for url in ["a.example", "b.example", "c.example"] {
        cx.simulate_keystrokes("secondary-t");
        cx.simulate_input(url);
        cx.simulate_keystrokes("enter");
    }
    let folder = b.update_in(cx, |b, window, cx| {
        let space = b.session.active_space().unwrap();
        let tabs: Vec<_> = b.session.tabs(space, Zone::Today).map(|t| t.id).collect();
        b.new_folder(Some(tabs[1]), window, cx);
        b.close_overlays(window, cx);
        let folder = b.session.folders(space).next().unwrap().id;
        b.session.move_to_folder(tabs[2], folder, 1);
        b.activate_tab(tabs[0], window, cx);
        for tab in &tabs {
            b.motion.from(("tab-in", tab.0), crate::motion::Feel::Glide, 0.0, 0.0);
        }
        b.set_folder_open(folder, false);
        b.folder_motion.clear();
        folder
    });
    cx.run_until_parked();
    let gap = |cx: &mut VisualTestContext| {
        let rows = cx.debug_bounds("folder-rows");
        let today = cx.debug_bounds("today-rule").unwrap();
        rows.map(|r| (f32::from(today.top() - r.bottom()), f32::from(r.size.height)))
    };
    for t in [0.25, 0.5, 0.75, 1.0] {
        b.update(cx, |b, cx| {
            b.set_folder_open(folder, true);
            b.folder_motion.insert(folder, crate::motion::Spring::new(t));
            b.folder_motion.get_mut(&folder).unwrap().set_target(1.0);
            cx.notify();
        });
        cx.run_until_parked();
        let (below, height) = gap(cx).expect("the folder's rows render");
        assert_eq!(below, 20., "TODAY stays the same distance under the rows at t={t}");
        assert!((height - 88. * t).abs() < 1.5, "the rows are {height}px tall at t={t}, not grown with the spring");
    }
}

#[gpui::test]
fn clicking_away_keeps_the_name_being_typed(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("a.example");
    cx.simulate_keystrokes("enter");
    let tab = b.read_with(cx, |b, _| b.active_tab().unwrap());
    b.update_in(cx, |b, window, cx| b.new_folder(Some(tab), window, cx));
    cx.simulate_input("Reading");
    cx.run_until_parked();
    cx.simulate_click(gpui::point(gpui::px(700.), gpui::px(400.)), gpui::Modifiers::none());
    b.read_with(cx, |b, _| {
        let s = b.session();
        assert!(b.renaming.is_none(), "the field closes");
        assert_eq!(s.folders(s.active_space().unwrap()).next().unwrap().name, "Reading");
    });
}

#[gpui::test]
fn an_extension_popup_acts_on_the_tab_it_opened_over(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let ext = baa_web::Extension {
        id: "ubo".into(),
        name: "uBO Lite".into(),
        icon: None,
        popup: Some("chrome-extension://ubo/popup.html".into()),
        options: None,
    };
    b.update(cx, |b, _| *b.host.extensions.borrow_mut() = vec![ext.clone()]);
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("a.example");
    cx.simulate_keystrokes("enter");
    b.update_in(cx, |b, window, cx| b.open_extension(&ext, window, cx));
    b.read_with(cx, |b, _| {
        assert_eq!(
            *b.host.popups.borrow(),
            [("chrome-extension://ubo/popup.html".to_string(), "https://a.example/".to_string())]
        );
    });
    // The popup's tabs.update on that tab lands on the tab, not in the popup.
    let popup = b.read_with(cx, |b, _| b.peek.as_ref().unwrap().page.clone());
    popup.update(cx, |_, cx| cx.emit(PageEvent::ActiveTabLoad(Some("https://b.example/".into()))));
    cx.run_until_parked();
    b.read_with(cx, |b, cx| {
        assert_eq!(*b.active_page().unwrap().read(cx).url.borrow(), "https://b.example/");
        assert_eq!(*popup.read(cx).url.borrow(), "chrome-extension://ubo/popup.html");
    });

    // The card shows once the content's size holds still, already at that size;
    // later sizes apply at once, as in Chrome, never animating the page.
    let size = |w: f32, h: f32| PageEvent::PreferredSize(gpui::size(gpui::px(w), gpui::px(h)));
    let shown = |cx: &mut gpui::VisualTestContext| b.read_with(cx, |b, _| (b.peek.as_ref().unwrap().shown, b.peek.as_ref().unwrap().size));
    popup.update(cx, |_, cx| cx.emit(size(25., 25.)));
    cx.executor().advance_clock(std::time::Duration::from_millis(30));
    popup.update(cx, |_, cx| cx.emit(size(320., 410.)));
    cx.executor().advance_clock(std::time::Duration::from_millis(30));
    assert_eq!(shown(cx), (false, Some((320., 410.))), "still settling");
    cx.executor().advance_clock(std::time::Duration::from_millis(100));
    assert_eq!(shown(cx), (true, Some((320., 410.))));
    popup.update(cx, |_, cx| cx.emit(size(320., 300.)));
    cx.run_until_parked();
    assert_eq!(shown(cx), (true, Some((320., 300.))));

    // Its options page (or any page it opens) is a baa tab, and the popup closes.
    popup.update(cx, |_, cx| {
        cx.emit(PageEvent::OpenRequested {
            url: "chrome-extension://ubo/dashboard.html".into(),
            from_user_gesture: true,
            background: false,
        })
    });
    cx.run_until_parked();
    b.read_with(cx, |b, cx| {
        assert!(b.peek.is_none());
        assert_eq!(*b.active_page().unwrap().read(cx).url.borrow(), "chrome-extension://ubo/dashboard.html");
    });

    b.update(cx, |b, cx| b.toggle_extension_pin("ubo", cx));
    b.read_with(cx, |b, _| assert_eq!(b.settings.unpinned_extensions, ["ubo"]));
    b.update(cx, |b, cx| b.toggle_extension_pin("ubo", cx));
    b.read_with(cx, |b, _| assert!(b.settings.unpinned_extensions.is_empty()));
}

#[gpui::test]
fn tabs_chrome_opens_itself_open_in_baa(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let tabs = b.read_with(cx, |b, _| b.host.chrome_tabs.clone());
    tabs.update(cx, |tabs, cx| {
        tabs.0.push("chrome-extension://op/welcome.html".into());
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(urls(&b, Zone::Today, cx), ["chrome-extension://op/welcome.html"]);
    assert!(tabs.read_with(cx, |tabs, _| tabs.0.is_empty()));
}

#[gpui::test]
fn chromes_extensions_page_is_baas_own_tab(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let info = baa_web::ExtensionInfo {
        id: "ubo".into(),
        name: "uBO Lite".into(),
        enabled: true,
        ..Default::default()
    };
    b.update(cx, |b, _| {
        *b.host.extension_details.borrow_mut() = vec![info.clone()];
        *b.host.extensions.borrow_mut() = vec![baa_web::Extension {
            id: "ubo".into(),
            name: "uBO Lite".into(),
            icon: None,
            popup: None,
            options: None,
        }];
    });
    let created = b.read_with(cx, |b, _| b.host.created.borrow().len());
    b.update_in(cx, |b, window, cx| b.open_url("chrome://extensions/?id=ubo", window, cx));
    cx.run_until_parked();
    b.read_with(cx, |b, _| {
        assert_eq!(b.active_tab_url().as_deref(), Some("baa://extensions"));
        assert_eq!(b.host.created.borrow().len(), created, "no web page for it");
        assert_eq!(b.extensions_page.list.as_deref(), Some(&[info][..]));
        assert_eq!(b.extensions_page.open.as_deref(), Some("ubo"));
    });
    // Opening it again goes back to the same tab.
    b.update_in(cx, |b, window, cx| b.open_url("chrome://extensions/", window, cx));
    assert_eq!(urls(&b, Zone::Today, cx), ["baa://extensions"]);

    // Switched off, it leaves the toolbar; Chrome is told.
    b.update(cx, |b, cx| b.change_extension("ubo", baa_web::ExtensionChange::Enable(false), cx));
    cx.run_until_parked();
    b.read_with(cx, |b, _| {
        assert_eq!(*b.host.extension_changes.borrow(), [("ubo".to_string(), baa_web::ExtensionChange::Enable(false))]);
        assert!(!b.extension_enabled("ubo"));
    });
}

#[gpui::test]
fn chrome_settings_open_skinned_in_a_sheet(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    b.update_in(cx, |b, window, cx| b.open_url("chrome://settings/privacy", window, cx));
    cx.run_until_parked();
    let sheet = b.read_with(cx, |b, _| b.host.chrome_sheet.borrow().clone()).expect("shown");
    assert_eq!(sheet.url, "chrome://settings/privacy");
    assert!(sheet.css.contains("--cr-fallback-color-primary"), "skinned");
    assert!(urls(&b, Zone::Today, cx).is_empty(), "a sheet, not a tab");
    b.update_in(cx, |b, window, cx| b.close_overlays(window, cx));
    cx.run_until_parked();
    assert!(b.read_with(cx, |b, _| b.host.chrome_sheet.borrow().is_none()), "hidden again");
}

#[gpui::test]
fn a_tab_menu_is_native_and_runs_the_picked_item(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("a.example");
    cx.simulate_keystrokes("enter");
    let tab = b.read_with(cx, |b, _| b.active_tab().unwrap());
    let labels = |b: &Browser<MockHost>| -> Vec<String> {
        b.host.menus.borrow().last().unwrap().iter().filter_map(|i| match i {
            baa_web::MenuItem::Action { label, .. } => Some(label.clone()),
            _ => None,
        }).collect()
    };
    // Dismissed: nothing happens.
    b.update_in(cx, |b, window, cx| b.open_menu(crate::browser::MenuKind::Tab(tab), gpui::point(gpui::px(10.), gpui::px(10.)), window, cx));
    cx.run_until_parked();
    let close = b.read_with(cx, |b, _| labels(b).iter().position(|l| l == "Close Tab").unwrap());
    assert!(b.read_with(cx, |b, _| b.session.tab(tab).is_some() && !b.menu_open));
    // Picking "Close Tab" closes it.
    b.update_in(cx, |b, window, cx| {
        b.host.menu_pick.set(Some(close));
        b.open_menu(crate::browser::MenuKind::Tab(tab), gpui::point(gpui::px(10.), gpui::px(10.)), window, cx)
    });
    cx.run_until_parked();
    assert!(b.read_with(cx, |b, _| b.session.tab(tab).is_none()));
}

#[gpui::test]
fn devtools_dock_beside_the_tab_and_go_with_it(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("a.example");
    cx.simulate_keystrokes("enter");
    let tab = b.read_with(cx, |b, _| b.active_tab().unwrap());
    let docked = |cx: &mut gpui::VisualTestContext| b.read_with(cx, |b, cx| b.devtools.get(&tab).map(|(p, _)| p.read(cx).url.borrow().clone()));

    // Asked once it docks; asked again it closes, as ⌥⌘I toggles in Chrome.
    b.update_in(cx, |b, window, cx| b.toggle_devtools(None, window, cx));
    assert_eq!(docked(cx).as_deref(), Some("devtools://https://a.example/"));
    b.update_in(cx, |b, window, cx| b.toggle_devtools(None, window, cx));
    assert_eq!(docked(cx), None);

    // The page menu's Inspect always opens it.
    let page = b.read_with(cx, |b, _| b.active_page().unwrap().clone());
    for _ in 0..2 {
        page.update(cx, |_, cx| cx.emit(PageEvent::DevToolsRequested { x: 5, y: 5 }));
        cx.run_until_parked();
        assert!(docked(cx).is_some());
    }

    // DevTools' own dock-side picker moves it (and remembers), without reopening it.
    let tools = b.read_with(cx, |b, _| b.devtools.get(&tab).unwrap().0.clone());
    tools.update(cx, |_, cx| cx.emit(PageEvent::DevToolsDock("bottom".into())));
    cx.run_until_parked();
    assert_eq!(b.read_with(cx, |b, _| b.settings.devtools_dock), baa_model::DevToolsDock::Bottom);
    assert_eq!(b.read_with(cx, |b, _| b.devtools.get(&tab).unwrap().0.entity_id()), tools.entity_id());
    // Where page and DevTools meet, neither rounds its corners.
    cx.run_until_parked();
    b.read_with(cx, |b, cx| {
        assert_eq!(b.active_page().unwrap().read(cx).square.get(), 4 | 8);
        assert_eq!(tools.read(cx).square.get(), 1 | 2);
    });
    // Undocking there takes it to a window.
    tools.update(cx, |_, cx| cx.emit(PageEvent::DevToolsDock("undocked".into())));
    cx.run_until_parked();
    assert_eq!(docked(cx), None);
    b.update_in(cx, |b, window, cx| {
        b.set_devtools_dock(baa_model::DevToolsDock::Right, window, cx);
        b.toggle_devtools(None, window, cx);
    });

    // Moving it to a window takes it out of the page area, and sticks.
    b.update_in(cx, |b, window, cx| b.set_devtools_dock(baa_model::DevToolsDock::Window, window, cx));
    assert_eq!(docked(cx), None);
    assert_eq!(b.read_with(cx, |b, _| b.settings.devtools_dock), baa_model::DevToolsDock::Window);
    b.update_in(cx, |b, window, cx| {
        b.set_devtools_dock(baa_model::DevToolsDock::Right, window, cx);
        b.toggle_devtools(None, window, cx);
    });
    assert!(docked(cx).is_some());

    // It goes with its tab.
    b.update_in(cx, |b, window, cx| {
        b.session.close_tab(tab, 0);
        b.sync(window, cx);
    });
    assert_eq!(b.read_with(cx, |b, _| b.devtools.len()), 0);

    // In its own window, nothing docks.
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("b.example");
    cx.simulate_keystrokes("enter");
    b.update_in(cx, |b, window, cx| {
        b.settings.devtools_dock = baa_model::DevToolsDock::Window;
        b.toggle_devtools(None, window, cx);
    });
    assert_eq!(b.read_with(cx, |b, _| b.devtools.len()), 0);
}
