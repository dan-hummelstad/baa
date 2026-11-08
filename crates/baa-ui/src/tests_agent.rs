//! Claude's tool server (PLAN-m4-features D4) on the mock host: scope, approvals, secret fields.

use std::sync::mpsc::Receiver;

use baa_model::{Store, TabId};
use baa_web::WebHost as _;
use gpui::{Entity, TestAppContext, VisualTestContext};
use serde_json::{json, Value};

use crate::{agent::Answer, mock::MockHost, prompts::ACTIVATION_DELAY, Browser};

fn browser(cx: &mut TestAppContext) -> (Entity<Browser<MockHost>>, &mut VisualTestContext) {
    cx.update(crate::init);
    cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx))
}

/// Turns Claude on for the active Space and opens `url` there.
fn enabled_with(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext, url: &str) -> TabId {
    b.update_in(cx, |b, window, cx| {
        b.set_claude_enabled(true, cx);
        b.open_url(url, window, cx);
        b.active_tab().unwrap()
    })
}

fn call(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext, tool: &str, args: Value) -> Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let msg = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}});
    b.update_in(cx, |b, window, cx| b.agent_message(1, &msg.to_string(), tx, window, cx));
    cx.run_until_parked();
    rx
}

/// The tool's answer: (refused, text).
fn answer(rx: &Receiver<String>) -> Option<(bool, String)> {
    let v: Value = serde_json::from_str(&rx.try_recv().ok()?).unwrap();
    let r = &v["result"];
    Some((r["isError"].as_bool().unwrap_or(false), r["content"][0]["text"].as_str().unwrap_or_default().to_string()))
}

fn decide(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext, a: Answer) {
    cx.executor().advance_clock(ACTIVATION_DELAY);
    b.update_in(cx, |b, window, cx| b.answer_agent_ask(a, window, cx));
    cx.run_until_parked();
}

fn cdp(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext, tab: TabId) -> Vec<String> {
    b.read_with(cx, |b, cx| b.pages[&tab].0.read(cx).cdp.borrow().clone())
}

fn asks(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) -> usize {
    b.read_with(cx, |b, _| b.agent.asks.len())
}

#[gpui::test]
fn claude_sees_only_web_pages_of_ticked_spaces(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let web = enabled_with(&b, cx, "https://github.com/");
    let (chrome, hidden, private, private_space) = b.update_in(cx, |b, window, cx| {
        let here = b.session.active_space().unwrap();
        let chrome = b.session.open_tab(here, "chrome://flags", baa_model::now());
        let other = b.session.add_space("Work", "🌿", None);
        let hidden = b.session.open_tab(other, "https://secret.example/", baa_model::now());
        let profile = b.host.ephemeral_profile();
        let private_space = b.session.add_ephemeral_space("Incognito", profile);
        let private = b.session.open_tab(private_space, "https://private.example/", baa_model::now());
        // Even ticked, an ephemeral Space stays out.
        b.settings.claude.allowed_spaces.push(private_space);
        b.session.set_active_space(here);
        b.activate_tab(web, window, cx);
        (chrome, hidden, private, private_space)
    });

    let (refused, text) = answer(&call(&b, cx, "list_tabs", json!({}))).unwrap();
    assert!(!refused);
    let tabs: Vec<Value> = serde_json::from_str(&text).unwrap();
    let ids: Vec<u64> = tabs.iter().map(|t| t["id"].as_u64().unwrap()).collect();
    assert_eq!(ids, [web.0], "chrome://, another Space's and incognito tabs are not listed");

    let (_, text) = answer(&call(&b, cx, "list_spaces", json!({}))).unwrap();
    assert!(!text.contains(&format!("\"id\": {}", private_space.0)) && !text.contains("Incognito"));

    for (tab, why) in [(chrome, "http"), (hidden, "No such tab"), (private, "No such tab")] {
        let (refused, text) = answer(&call(&b, cx, "read_page", json!({"tab": tab.0}))).unwrap();
        assert!(refused && text.contains(why), "{tab:?}: {text}");
        let (refused, _) = answer(&call(&b, cx, "click", json!({"tab": tab.0, "x": 1, "y": 1}))).unwrap();
        assert!(refused);
    }
    assert_eq!(asks(&b, cx), 0, "nothing out of scope reaches the user as a sheet");

    for url in ["baa://settings", "file:///etc/passwd", "chrome-extension://x/a.html", "devtools://devtools/x", "javascript:alert(1)"] {
        let (refused, _) = answer(&call(&b, cx, "open_tab", json!({"url": url}))).unwrap();
        assert!(refused, "{url}");
        let (refused, _) = answer(&call(&b, cx, "navigate", json!({"tab": web.0, "url": url}))).unwrap();
        assert!(refused, "{url}");
    }
    assert_eq!(asks(&b, cx), 0);
}

#[gpui::test]
fn acting_asks_per_origin_once_session_and_deny(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let gh = enabled_with(&b, cx, "https://github.com/");

    // Reading needs no sheet.
    let rx = call(&b, cx, "read_page", json!({}));
    assert_eq!(asks(&b, cx), 0);
    drop(rx);

    let rx = call(&b, cx, "click", json!({"x": 1, "y": 2}));
    assert_eq!(asks(&b, cx), 1);
    assert!(answer(&rx).is_none() && cdp(&b, cx, gh).is_empty(), "nothing happens before the user answers");
    // Too soon: a click aimed at what was there before.
    b.update_in(cx, |b, window, cx| b.answer_agent_ask(Answer::Once, window, cx));
    assert_eq!(asks(&b, cx), 1);
    decide(&b, cx, Answer::Once);
    assert_eq!(answer(&rx).unwrap().0, false);
    assert_eq!(cdp(&b, cx, gh), ["click 1 2"]);
    assert!(b.read_with(cx, |b, _| b.agent.driven.contains(&gh)));

    // Once was once.
    let rx = call(&b, cx, "press_key", json!({"key": "Enter"}));
    assert_eq!(asks(&b, cx), 1);
    decide(&b, cx, Answer::Session);
    assert_eq!(answer(&rx).unwrap().0, false);
    let rx = call(&b, cx, "press_key", json!({"key": "Tab"}));
    assert_eq!(asks(&b, cx), 0, "for this session: no second sheet");
    assert_eq!(answer(&rx).unwrap().0, false);
    assert_eq!(cdp(&b, cx, gh), ["click 1 2", "press_key Enter", "press_key Tab"]);

    // Deny is remembered for the origin, so a page cannot nag until the user gives in.
    let other = b.update_in(cx, |b, window, cx| {
        b.open_url("https://example.com/", window, cx);
        b.active_tab().unwrap()
    });
    let rx = call(&b, cx, "click", json!({"tab": other.0, "x": 5, "y": 5}));
    decide(&b, cx, Answer::Deny);
    assert_eq!(answer(&rx).unwrap().0, true);
    let rx = call(&b, cx, "click", json!({"tab": other.0, "x": 5, "y": 5}));
    assert_eq!(asks(&b, cx), 0);
    assert_eq!(answer(&rx).unwrap().0, true);
    assert!(cdp(&b, cx, other).is_empty());

    // Turning Claude off forgets every approval; a pending sheet goes with it.
    let pending = call(&b, cx, "rename_tab", json!({"tab": other.0, "title": "x"}));
    let _ = pending;
    b.update_in(cx, |b, _, cx| {
        b.set_claude_enabled(false, cx);
        assert!(b.agent.asks.is_empty() && b.agent.driven.is_empty());
        b.set_claude_enabled(true, cx);
    });
    let rx = call(&b, cx, "press_key", json!({"tab": gh.0, "key": "a"}));
    assert_eq!(asks(&b, cx), 1, "github.com asks again after off and on");
    assert!(answer(&rx).is_none());
}

#[gpui::test]
fn nothing_answers_while_claude_is_off(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    b.update_in(cx, |b, window, cx| b.open_url("https://github.com/", window, cx));
    let rx = call(&b, cx, "list_tabs", json!({}));
    assert!(rx.try_recv().is_err());
}

#[gpui::test]
fn type_refuses_secret_fields(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let tab = enabled_with(&b, cx, "https://bank.example/login");
    let answer_eval = |b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext, v: Value| {
        let (script, done) = b.read_with(cx, |b, cx| b.pages[&tab].0.read(cx).evals.borrow_mut().pop().unwrap());
        assert!(script.contains("password") && script.contains("one-time-code"));
        cx.update(|_, cx| done(Ok(v), cx));
        cx.run_until_parked();
    };

    let rx = call(&b, cx, "type", json!({"text": "hunter2"}));
    decide(&b, cx, Answer::Session);
    answer_eval(&b, cx, json!({"secret": true}));
    let (refused, text) = answer(&rx).unwrap();
    assert!(refused && text.contains("password"), "{text}");
    assert!(cdp(&b, cx, tab).is_empty(), "nothing was typed");

    // A check that did not come back clean refuses too.
    let rx = call(&b, cx, "type", json!({"text": "x"}));
    answer_eval(&b, cx, Value::Null);
    assert_eq!(answer(&rx).unwrap().0, true);

    let rx = call(&b, cx, "type", json!({"text": "hello", "selector": "#q"}));
    answer_eval(&b, cx, json!({"secret": false}));
    assert_eq!(answer(&rx).unwrap().0, false);
    assert_eq!(cdp(&b, cx, tab), ["insert_text hello"]);
}

#[gpui::test]
fn navigating_to_a_new_origin_needs_its_own_approval(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let tab = enabled_with(&b, cx, "https://github.com/");
    let rx = call(&b, cx, "press_key", json!({"key": "a"}));
    decide(&b, cx, Answer::Session);
    answer(&rx).unwrap();

    let loads = |b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext| b.read_with(cx, |b, cx| b.pages[&tab].0.read(cx).loads.borrow().clone());
    let rx = call(&b, cx, "navigate", json!({"url": "https://evil.example/"}));
    assert_eq!(asks(&b, cx), 1, "github.com's grant does not cover evil.example");
    assert!(loads(&b, cx).is_empty(), "an agent-supplied URL loads only after the sheet");
    decide(&b, cx, Answer::Once);
    assert_eq!(answer(&rx).unwrap().0, false);
    assert_eq!(loads(&b, cx), ["https://evil.example/"]);

    // The tab is on evil.example now: the next act asks again.
    let rx = call(&b, cx, "click", json!({"x": 1, "y": 1}));
    assert_eq!(asks(&b, cx), 1);
    assert!(answer(&rx).is_none() && cdp(&b, cx, tab) == ["press_key a"]);

    // open_tab, the same: nothing opens before the sheet.
    let before = b.read_with(cx, |b, _| b.host.created.borrow().len());
    let _rx = call(&b, cx, "open_tab", json!({"url": "https://another.example/"}));
    assert_eq!(b.read_with(cx, |b, _| b.host.created.borrow().len()), before);
}

#[gpui::test]
fn evaluate_is_off_by_default_and_asks_every_call(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let tab = enabled_with(&b, cx, "https://github.com/");
    let (refused, text) = answer(&call(&b, cx, "evaluate", json!({"script": "1+1"}))).unwrap();
    assert!(refused && text.contains("off"), "{text}");
    assert_eq!(asks(&b, cx), 0);

    b.update(cx, |b, _| b.settings.claude.evaluate_allowed = true);
    let rx = call(&b, cx, "press_key", json!({"key": "a"}));
    decide(&b, cx, Answer::Session);
    answer(&rx).unwrap();
    for _ in 0..2 {
        let _rx = call(&b, cx, "evaluate", json!({"script": "1+1"}));
        assert_eq!(asks(&b, cx), 1, "a session grant does not cover evaluate");
        decide(&b, cx, Answer::Session);
        let evals = b.read_with(cx, |b, cx| b.pages[&tab].0.read(cx).evals.borrow().len());
        assert!(evals >= 1);
    }
}

#[gpui::test]
fn the_approve_tool_denies_and_unknown_tools_are_refused(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    enabled_with(&b, cx, "https://github.com/");
    let (_, text) = answer(&call(&b, cx, "approve", json!({"tool_name": "Bash", "input": {}}))).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["behavior"], "deny");
    let (refused, _) = answer(&call(&b, cx, "cookies", json!({}))).unwrap();
    assert!(refused);
}

#[gpui::test]
fn clear_today_keeps_driven_tabs(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let tab = enabled_with(&b, cx, "https://github.com/");
    b.update_in(cx, |b, window, cx| {
        b.agent.driven.insert(tab);
        b.open_url("https://example.com/", window, cx);
        b.clear_today(window, cx);
        assert!(b.session.tab(tab).is_some_and(|t| t.archived_at.is_none()));
    });
}

#[gpui::test]
fn the_chat_panel_docks_opposite_the_sidebar_and_slides(cx: &mut TestAppContext) {
    use crate::agent_panel::PANEL_WIDTH;
    let (b, cx) = browser(cx);
    b.update_in(cx, |b, window, cx| b.toggle_agent_panel(window, cx));
    cx.run_until_parked();
    b.read_with(cx, |b, _| {
        assert!(b.agent.panel.open);
        assert_eq!(b.agent_w.target(), PANEL_WIDTH, "the docked slot widens by the same spring the sidebar uses");
        assert_ne!(b.side(), baa_model::SidebarSide::Right, "default sidebar is left, so the panel is right");
    });
    b.update_in(cx, |b, window, cx| b.toggle_agent_panel(window, cx));
    b.read_with(cx, |b, _| {
        assert!(!b.agent.panel.open);
        assert_eq!(b.agent_w.target(), 0.0, "closing slides the slot shut rather than dropping it");
    });
}
