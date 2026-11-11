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
    assert!(!answer(&rx).unwrap().0);
    assert_eq!(cdp(&b, cx, gh), ["click 1 2"]);
    assert!(b.read_with(cx, |b, _| b.agent.driven.contains(&gh)));

    // Once was once.
    let rx = call(&b, cx, "press_key", json!({"key": "Enter"}));
    assert_eq!(asks(&b, cx), 1);
    decide(&b, cx, Answer::Session);
    assert!(!answer(&rx).unwrap().0);
    let rx = call(&b, cx, "press_key", json!({"key": "Tab"}));
    assert_eq!(asks(&b, cx), 0, "for this session: no second sheet");
    assert!(!answer(&rx).unwrap().0);
    assert_eq!(cdp(&b, cx, gh), ["click 1 2", "press_key Enter", "press_key Tab"]);

    // Deny is remembered for the origin, so a page cannot nag until the user gives in.
    let other = b.update_in(cx, |b, window, cx| {
        b.open_url("https://example.com/", window, cx);
        b.active_tab().unwrap()
    });
    let rx = call(&b, cx, "click", json!({"tab": other.0, "x": 5, "y": 5}));
    decide(&b, cx, Answer::Deny);
    assert!(answer(&rx).unwrap().0);
    let rx = call(&b, cx, "click", json!({"tab": other.0, "x": 5, "y": 5}));
    assert_eq!(asks(&b, cx), 0);
    assert!(answer(&rx).unwrap().0);
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
    assert!(answer(&rx).unwrap().0);

    let rx = call(&b, cx, "type", json!({"text": "hello", "selector": "#q"}));
    answer_eval(&b, cx, json!({"secret": false}));
    assert!(!answer(&rx).unwrap().0);
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
    assert!(!answer(&rx).unwrap().0);
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
fn chat_is_a_library_section_toggled_by_cmd_j(cx: &mut TestAppContext) {
    use crate::library::{Section, LIBRARY_WIDTH};
    let (b, cx) = browser(cx);
    let section = |b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext| b.read_with(cx, |b, _| b.library.as_ref().map(|l| l.section));
    // From a collapsed sidebar: it comes out docked, at the Library's width, on Chat, and the
    // collapsed setting is untouched, so closing goes back to it.
    cx.dispatch_action(crate::actions::ToggleSidebar);
    b.update_in(cx, |b, window, cx| b.toggle_agent_panel(window, cx));
    assert_eq!(section(&b, cx), Some(Section::Chat));
    b.read_with(cx, |b, _| assert!(!b.sidebar_open && b.docked() && b.sidebar_w.target() == LIBRARY_WIDTH));
    b.update_in(cx, |b, window, cx| b.toggle_agent_panel(window, cx));
    assert_eq!(section(&b, cx), None, "⌘J on Chat closes the Library");
    b.read_with(cx, |b, _| assert!(!b.sidebar_open && !b.docked() && b.sidebar_w.target() == 0.));
    // From another section, ⌘J switches to Chat rather than closing.
    b.update(cx, |b, cx| b.open_library(Section::Downloads, cx));
    b.update_in(cx, |b, window, cx| b.toggle_agent_panel(window, cx));
    assert_eq!(section(&b, cx), Some(Section::Chat));
    // ⌘⇧L closes it, and opens it again on the section it was last on.
    cx.dispatch_action(crate::actions::ToggleLibrary);
    assert_eq!(section(&b, cx), None);
    cx.dispatch_action(crate::actions::ToggleLibrary);
    assert_eq!(section(&b, cx), Some(Section::Chat));
    // The sidebar's Library button does the same.
    cx.dispatch_action(crate::actions::ToggleLibrary);
    b.update_in(cx, |b, window, cx| b.open_library_last(window, cx));
    assert_eq!(section(&b, cx), Some(Section::Chat));
}

// The handshake (pairing token, connect sheet, the panel's nonce).

const CLAUDE: &str = "/Users/me/.local/share/claude/versions/2.1.318";
const BAD: &str = "A process tried to connect to Claude with a bad token";
const INIT: &str = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#;

/// A new client on agent.sock, before its hello; pid and path as the app shell found them.
fn connect(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) -> u64 {
    b.update(cx, |b, _| b.agent_connect(None, Some(4242), Some(CLAUDE.into())))
}

fn send(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext, conn: u64, line: &str) -> Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    b.update_in(cx, |b, window, cx| b.agent_line(conn, line, tx, window, cx));
    cx.run_until_parked();
    rx
}

fn hello(key: &str, secret: &str) -> String {
    json!({"baa": "hello", key: secret}).to_string()
}

fn token(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) -> String {
    b.read_with(cx, |b, _| b.settings.claude.token.clone())
}

fn toast(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) -> Option<String> {
    b.read_with(cx, |b, _| b.toast.as_ref().map(|t| t.to_string()))
}

fn last_activity(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) -> (&'static str, String, bool) {
    b.read_with(cx, |b, _| b.agent.activity.last().map(|a| (a.tool, a.target.clone(), a.refused)).unwrap())
}

fn served(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext, conn: u64) -> bool {
    b.read_with(cx, |b, _| matches!(b.agent.state.get(&conn), Some(crate::agent::Conn::Open)))
}

fn closed(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext, conn: u64) -> bool {
    b.read_with(cx, |b, _| !b.agent.state.contains_key(&conn))
}

#[gpui::test]
fn a_paired_client_is_served_only_after_allow(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    enabled_with(&b, cx, "https://github.com/");
    let conn = connect(&b, cx);
    let t = token(&b, cx);
    let hello_rx = send(&b, cx, conn, &hello("token", &t));
    assert_eq!(asks(&b, cx), 1, "a right token raises the connect sheet");
    b.read_with(cx, |b, _| {
        let ask = b.agent.asks.front().unwrap();
        assert_eq!(ask.connect, Some((CLAUDE.to_string(), Some(4242))));
        assert!(ask.every_time, "no \"for this session\" for a connection");
    });
    // Not even initialize is answered while the sheet is up.
    let init_rx = send(&b, cx, conn, INIT);
    let tools_rx = send(&b, cx, conn, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
    assert!(hello_rx.try_recv().is_err() && init_rx.try_recv().is_err() && tools_rx.try_recv().is_err());
    assert!(!b.read_with(cx, |b, _| b.agent.connected()), "no strip before Allow");

    // Within 500 ms of the sheet an answer is ignored.
    cx.executor().advance_clock(ACTIVATION_DELAY / 2);
    b.update_in(cx, |b, window, cx| b.answer_agent_ask(Answer::Once, window, cx));
    assert_eq!(asks(&b, cx), 1);
    assert!(!served(&b, cx, conn));

    decide(&b, cx, Answer::Once);
    assert!(served(&b, cx, conn));
    // What waited is answered in order, on the connection's writer.
    let first: Value = serde_json::from_str(&hello_rx.try_recv().unwrap()).unwrap();
    assert_eq!(first["id"], 0);
    assert_eq!(first["result"]["serverInfo"]["name"], "baa");
    let second: Value = serde_json::from_str(&hello_rx.try_recv().unwrap()).unwrap();
    assert_eq!(second["id"], 1);
    assert!(b.read_with(cx, |b, _| b.agent.connected() && b.agent.status.as_deref() == Some("connected (2.1.318)")));
    assert_eq!(last_activity(&b, cx), ("connect", "2.1.318".into(), false));
    assert!(toast(&b, cx).is_none());
}

#[gpui::test]
fn a_bad_hello_is_closed_with_a_toast_and_no_sheet(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    enabled_with(&b, cx, "https://github.com/");
    let t = token(&b, cx);
    let mut wrong = t.clone();
    wrong.replace_range(..1, if t.starts_with('0') { "1" } else { "0" });
    let firsts = [
        hello("token", &wrong),
        hello("token", ""),
        hello("token", &t[..32]),
        INIT.to_string(),
        json!({"baa": "hello"}).to_string(),
        json!({"baa": "hello", "token": t, "nonce": t}).to_string(),
    ];
    for first in firsts {
        b.update(cx, |b, _| b.toast = None);
        let conn = connect(&b, cx);
        let rx = send(&b, cx, conn, &first);
        assert_eq!(asks(&b, cx), 0, "{first}");
        assert!(closed(&b, cx, conn), "{first}");
        assert_eq!(toast(&b, cx).as_deref(), Some(BAD));
        let (tool, _, refused) = last_activity(&b, cx);
        assert!(tool == "connect" && refused);
        // Whatever it sends next goes nowhere.
        let after = send(&b, cx, conn, INIT);
        assert!(rx.try_recv().is_err() && after.try_recv().is_err(), "{first}");
    }
    assert!(!b.read_with(cx, |b, _| b.agent.connected()));
    let logged = b.read_with(cx, |b, _| b.agent.activity.iter().map(|a| a.target.clone()).collect::<Vec<_>>().join(" "));
    assert!(!logged.contains(&t[..16]) && !logged.contains(&wrong[..16]), "the activity log never holds a secret");
}

#[gpui::test]
fn deny_closes_the_connection(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    enabled_with(&b, cx, "https://github.com/");
    let conn = connect(&b, cx);
    let t = token(&b, cx);
    let rx = send(&b, cx, conn, &hello("token", &t));
    send(&b, cx, conn, INIT);
    decide(&b, cx, Answer::Deny);
    assert_eq!(asks(&b, cx), 0);
    assert!(closed(&b, cx, conn) && !b.read_with(cx, |b, _| b.agent.connected()));
    assert!(rx.try_recv().is_err(), "initialize was never answered");
    assert_eq!(last_activity(&b, cx), ("connect", "denied".into(), true));
    assert!(toast(&b, cx).is_none(), "the user said no; nothing to warn about");
    assert!(send(&b, cx, conn, INIT).try_recv().is_err());
}

#[gpui::test]
fn the_panel_nonce_skips_the_sheet_once(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    enabled_with(&b, cx, "https://github.com/");
    let nonce = crate::agent::secret();
    b.update(cx, |b, cx| b.agent.nonce = Some((nonce.clone(), cx.background_executor().now())));
    cx.executor().advance_clock(std::time::Duration::from_secs(30));

    let conn = connect(&b, cx);
    send(&b, cx, conn, &hello("nonce", &nonce));
    assert_eq!(asks(&b, cx), 0, "baa started this client itself");
    assert!(served(&b, cx, conn));
    assert!(send(&b, cx, conn, INIT).try_recv().is_ok(), "served at once");
    assert!(b.read_with(cx, |b, _| b.agent.nonce.is_none() && b.agent.status.as_deref() == Some("connected (claude)")));

    // Used up: the same nonce again is a bad hello.
    let again = connect(&b, cx);
    send(&b, cx, again, &hello("nonce", &nonce));
    assert_eq!(asks(&b, cx), 0);
    assert!(closed(&b, cx, again));
    assert_eq!(toast(&b, cx).as_deref(), Some(BAD));

    // The token is not a nonce, nor the other way round.
    b.update(cx, |b, cx| b.agent.nonce = Some((nonce.clone(), cx.background_executor().now())));
    let t = token(&b, cx);
    let swapped = connect(&b, cx);
    send(&b, cx, swapped, &hello("nonce", &t));
    assert!(closed(&b, cx, swapped));
    let swapped = connect(&b, cx);
    send(&b, cx, swapped, &hello("token", &nonce));
    assert!(closed(&b, cx, swapped));
    assert_eq!(asks(&b, cx), 0);

    // Stop forgets it.
    b.update(cx, |b, cx| b.stop_agent(cx));
    assert!(b.read_with(cx, |b, _| b.agent.nonce.is_none()));
}

#[gpui::test]
fn an_expired_nonce_is_refused(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    enabled_with(&b, cx, "https://github.com/");
    let nonce = crate::agent::secret();
    b.update(cx, |b, cx| b.agent.nonce = Some((nonce.clone(), cx.background_executor().now())));
    cx.executor().advance_clock(crate::agent::NONCE_FOR);
    let conn = connect(&b, cx);
    send(&b, cx, conn, &hello("nonce", &nonce));
    assert_eq!(asks(&b, cx), 0);
    assert!(closed(&b, cx, conn) && b.read_with(cx, |b, _| b.agent.nonce.is_none()));
    assert_eq!(last_activity(&b, cx), ("connect", "expired nonce".into(), true));
    assert_eq!(toast(&b, cx).as_deref(), Some(BAD));
}

#[gpui::test]
fn turning_claude_on_makes_a_new_token_and_off_clears_it(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    assert!(token(&b, cx).is_empty());
    enabled_with(&b, cx, "https://github.com/");
    let first = token(&b, cx);
    assert!(first.len() == 64 && first.bytes().all(|c| c.is_ascii_hexdigit()), "{first}");
    b.update(cx, |b, cx| b.set_claude_enabled(true, cx));
    assert_eq!(token(&b, cx), first, "on while on keeps the token a pasted command holds");
    b.update(cx, |b, cx| b.set_claude_enabled(false, cx));
    assert!(token(&b, cx).is_empty());
    b.update(cx, |b, cx| b.set_claude_enabled(true, cx));
    let second = token(&b, cx);
    assert!(second.len() == 64 && second != first);
    // The old command no longer pairs.
    let conn = connect(&b, cx);
    send(&b, cx, conn, &hello("token", &first));
    assert_eq!(asks(&b, cx), 0);
    assert!(closed(&b, cx, conn));
}

// serve_agent's reader threads cannot run under the deterministic test scheduler, so these hand
// agent_line the lines and check what happens to the real socket.
#[gpui::test]
fn bad_hellos_and_deny_shut_the_socket(cx: &mut TestAppContext) {
    use std::{io::Read as _, os::unix::net::UnixStream};
    let (b, cx) = browser(cx);
    enabled_with(&b, cx, "https://github.com/");
    let t = token(&b, cx);
    let eof = |mut client: UnixStream| {
        let mut buf = Vec::new();
        client.read_to_end(&mut buf).is_ok() && buf.is_empty()
    };

    let (client, server) = UnixStream::pair().unwrap();
    let conn = b.update(cx, |b, _| b.agent_connect(Some(server), None, None));
    send(&b, cx, conn, &hello("token", "00"));
    assert!(eof(client), "a bad token is closed without a word");

    let (client, server) = UnixStream::pair().unwrap();
    let conn = b.update(cx, |b, _| b.agent_connect(Some(server), Some(7), Some("/x/claude".into())));
    send(&b, cx, conn, &hello("token", &t));
    send(&b, cx, conn, INIT);
    decide(&b, cx, Answer::Deny);
    assert!(eof(client), "deny closes the socket");

    // Stop with a sheet up: the sheet goes and the socket closes.
    let (client, server) = UnixStream::pair().unwrap();
    let conn = b.update(cx, |b, _| b.agent_connect(Some(server), None, None));
    send(&b, cx, conn, &hello("token", &t));
    assert_eq!(asks(&b, cx), 1);
    b.update(cx, |b, cx| b.stop_agent(cx));
    assert_eq!(asks(&b, cx), 0);
    assert!(eof(client));
}

#[gpui::test]
/// The transcript and the field being typed in both wrap; the field once cut long text off.
fn long_chat_messages_wrap_inside_the_panel(cx: &mut TestAppContext) {
    use crate::agent_panel::Entry;
    let (b, cx) = browser(cx);
    b.update(cx, |b, cx| b.set_claude_enabled(true, cx));
    b.update_in(cx, |b, window, cx| b.toggle_agent_panel(window, cx));
    let long = "word ".repeat(80);
    b.update(cx, |b, cx| {
        let t = &mut b.agent.panel.chat.transcript;
        t.push(Entry::User(long.clone()));
        t.push(Entry::Assistant(long.clone()));
        b.agent.panel.field.text = long.clone();
        cx.notify();
    });
    cx.run_until_parked();
    let panel = cx.debug_bounds("agent-panel-body").expect("the panel is drawn");
    for id in ["chat-user", "chat-reply", "chat-field"] {
        let r = cx.debug_bounds(id).unwrap_or_else(|| panic!("{id} is drawn"));
        assert!(r.size.height > gpui::px(40.), "{id} is one line: {r:?}");
        assert!(r.right() <= panel.right() + gpui::px(1.), "{id} runs past the panel: {r:?} in {panel:?}");
    }
}
