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
            "https://duckduckgo.com/?q=rust+lang",
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
