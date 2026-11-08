use serde_json::{json, Value};

use super::*;
use crate::{ConsoleLevel, Device, Media, MediaAction};

/// (tab, id, method) of each `baa_tab_cdp`.
fn sent() -> Vec<(TabId, u64, String)> {
    calls("tab_cdp ")
        .iter()
        .map(|call| {
            let mut parts = call.splitn(4, ' ').skip(1);
            let tab = parts.next().unwrap().parse().unwrap();
            let id = parts.next().unwrap().parse().unwrap();
            let message: Value = serde_json::from_str(parts.next().unwrap()).unwrap();
            assert_eq!(message["id"], id, "the id given is the message's");
            (tab, id, message["method"].as_str().unwrap().to_string())
        })
        .collect()
}

fn reply(env: &Env, tab: TabId, id: u64, json: &str) {
    let json = CString::new(json).unwrap();
    (env.callbacks.cdp)(env.ctx(), tab, id, json.as_ptr());
}

#[gpui::test]
fn replies_reach_the_call_that_asked_on_the_right_tab(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let (a, b) = (env.page(1, "https://a.test", cx), env.page(1, "https://b.test", cx));
    let (tab_a, tab_b) = (env.tab_of(&a, cx), env.tab_of(&b, cx));
    let got = Rc::new(RefCell::new(Vec::new()));
    for (page, name) in [(&a, "a"), (&b, "b")] {
        let got = got.clone();
        page.read_with(cx, |page, _| page.eval_value("1+1", Box::new(move |r, _| got.borrow_mut().push((name, r)))));
    }
    assert_eq!(sent(), [(tab_a, 1, "Runtime.evaluate".into()), (tab_b, 1, "Runtime.evaluate".into())], "ids are per tab");

    reply(&env, tab_b, 1, r#"{"id":1,"result":{"result":{"type":"number","value":3}}}"#);
    reply(&env, tab_a, 1, r#"{"id":1,"result":{"result":{},"exceptionDetails":{"text":"Uncaught","exception":{"description":"Error: no"}}}}"#);
    reply(&env, tab_a, 1, r#"{"id":1,"result":{"result":{"type":"number","value":9}}}"#);
    cx.run_until_parked();
    assert_eq!(*got.borrow(), [("b", Ok(json!(3))), ("a", Err("Error: no".into()))], "each done once");
}

#[gpui::test]
fn console_is_off_until_asked_and_comes_back_after_the_session_ends(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);
    env.state(tab, &State { url: "https://a.test/", ..State::default() });
    cx.run_until_parked();
    assert!(sent().is_empty(), "normal browsing has no CDP session");

    page.read_with(cx, |page, _| {
        page.watch_console();
        page.watch_console();
    });
    assert_eq!(sent(), [(tab, 1, "Runtime.enable".into())]);

    reply(&env, tab, 0, r#"{"method":"Runtime.consoleAPICalled","params":{"type":"error","args":[{"type":"string","value":"bad"}]}}"#);
    reply(&env, tab, 0, r#"{"method":"Baa.detached","params":{}}"#);
    cx.run_until_parked();
    assert!(seen.borrow().contains(&PageEvent::Console { level: ConsoleLevel::Error, text: "bad".into() }));

    env.state(tab, &State { url: "https://b.test/", ..State::default() });
    cx.run_until_parked();
    assert_eq!(sent().last(), Some(&(tab, 2, "Runtime.enable".into())), "enabled again after the next navigation");
}

#[gpui::test]
fn emulation_input_and_screenshots_go_through_the_allowlist(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let shot = Rc::new(RefCell::new(None));
    let sink = shot.clone();
    page.read_with(cx, |page, _| {
        page.set_emulation(None);
        page.set_emulation(Some(Device::Phone));
        page.click(10., 20.);
        page.insert_text("hi");
        page.press_key("Enter");
        page.screenshot(Box::new(move |r, _| *sink.borrow_mut() = Some(r)));
    });
    let methods: Vec<String> = sent().into_iter().map(|(_, _, m)| m).collect();
    assert_eq!(
        methods,
        [
            "Emulation.setDeviceMetricsOverride",
            "Emulation.setTouchEmulationEnabled",
            "Emulation.setUserAgentOverride",
            "Input.dispatchMouseEvent",
            "Input.dispatchMouseEvent",
            "Input.dispatchMouseEvent",
            "Input.insertText",
            "Input.dispatchKeyEvent",
            "Input.dispatchKeyEvent",
            "Page.captureScreenshot",
        ]
    );
    reply(&env, tab, 10, r#"{"id":10,"result":{"data":"iVBORw0KGgo="}}"#);
    cx.run_until_parked();
    assert_eq!(*shot.borrow(), Some(Ok("iVBORw0KGgo=".to_string())));
}

#[gpui::test]
fn media_and_mute_come_from_the_engine(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    let media = ffi::Media {
        title: c"Song".as_ptr(),
        artist: c"Band".as_ptr(),
        artwork_url: c"".as_ptr(),
        playing: true,
        actions: ffi::MEDIA_PAUSE | ffi::MEDIA_NEXT,
    };
    (env.callbacks.media)(env.ctx(), tab, &media);
    env.state(tab, &State { muted: true, ..State::default() });
    cx.run_until_parked();
    let expected = Media {
        title: "Song".into(),
        artist: "Band".into(),
        artwork: None,
        playing: true,
        actions: vec![MediaAction::Pause, MediaAction::Next],
    };
    assert!(seen.borrow().contains(&PageEvent::Media(expected)));
    assert!(seen.borrow().contains(&PageEvent::Muted(true)));

    page.read_with(cx, |page, _| page.set_media_action(MediaAction::PictureInPicture));
    assert_eq!(calls("tab_media_action"), [format!("tab_media_action {tab} {}", ffi::MEDIA_PICTURE_IN_PICTURE)]);
}
