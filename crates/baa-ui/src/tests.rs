//! Browser view driven through real keystrokes against `MockHost`.

use baa_model::{Store, Zone};
use baa_web::{PageEvent, WebHost};
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::{actions::*, mock::MockHost, Browser};

fn browser(cx: &mut TestAppContext) -> (Entity<Browser<MockHost>>, &mut VisualTestContext) {
    cx.update(crate::init);
    cx.add_window_view(|window, cx| {
        Browser::new(
            MockHost::default(),
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
            Browser::new(MockHost::default(), Store::open(path).unwrap(), window, cx)
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

    // Leaving it closes it only after the linger delay.
    cx.simulate_mouse_move(point(px(700.), px(300.)), None, Modifiers::none());
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
