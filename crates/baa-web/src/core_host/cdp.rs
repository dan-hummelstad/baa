//! The one gate to a tab's DevTools protocol (`baa_tab_cdp`). CDP can do
//! anything a debugger can, so baa-web sends only the methods below, and the
//! UI and the agent get typed `PageHandle` calls, never raw CDP.

use serde_json::{json, Value};

use crate::ConsoleLevel;

/// Every CDP method baa sends. Nothing here reads cookies, storage, network
/// traffic or files, or reaches another target.
pub(super) const ALLOWED: &[&str] = &[
    // eval_value, watch_console
    "Runtime.evaluate",
    "Runtime.enable",
    // screenshot
    "Page.captureScreenshot",
    // click, insert_text, press_key: real input, not isTrusted:false
    "Input.dispatchMouseEvent",
    "Input.insertText",
    "Input.dispatchKeyEvent",
    // set_emulation
    "Emulation.setDeviceMetricsOverride",
    "Emulation.clearDeviceMetricsOverride",
    "Emulation.setTouchEmulationEnabled",
    "Emulation.setUserAgentOverride",
];

/// The message for `baa_tab_cdp`, or `None` for a method not in [`ALLOWED`].
pub(super) fn command(id: u64, method: &str, params: Value) -> Option<String> {
    ALLOWED.contains(&method).then(|| json!({ "id": id, "method": method, "params": params }).to_string())
}

/// A message from the session, as far as baa cares.
#[derive(Debug, PartialEq)]
pub(super) enum Incoming {
    /// The reply to command `id`: its `result`, or the error's message.
    Reply(u64, Result<Value, String>),
    Console { level: ConsoleLevel, text: String },
    /// Enabled domains and emulation are gone until sent again.
    Detached,
    Other,
}

pub(super) fn parse(id: u64, json: &str) -> Incoming {
    let Ok(message) = serde_json::from_str::<Value>(json) else { return Incoming::Other };
    if id != 0 {
        let result = match message.get("error") {
            Some(error) => Err(error["message"].as_str().unwrap_or("CDP error").to_string()),
            None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
        };
        return Incoming::Reply(id, result);
    }
    let params = &message["params"];
    match message["method"].as_str() {
        Some("Runtime.consoleAPICalled") => {
            let level = match params["type"].as_str() {
                Some("error" | "assert") => ConsoleLevel::Error,
                Some("warning") => ConsoleLevel::Warning,
                Some("info") => ConsoleLevel::Info,
                Some("debug") => ConsoleLevel::Debug,
                _ => ConsoleLevel::Log,
            };
            let args = params["args"].as_array().map(Vec::as_slice).unwrap_or_default();
            let text = args.iter().map(remote_text).collect::<Vec<_>>().join(" ");
            Incoming::Console { level, text }
        }
        Some("Runtime.exceptionThrown") => {
            Incoming::Console { level: ConsoleLevel::Error, text: exception_text(&params["exceptionDetails"]) }
        }
        Some("Baa.detached") => Incoming::Detached,
        _ => Incoming::Other,
    }
}

/// `Runtime.evaluate`'s result: the value (`returnByValue`), or the exception.
pub(super) fn eval_value(result: Value) -> Result<Value, String> {
    if let Some(details) = result.get("exceptionDetails") {
        return Err(exception_text(details));
    }
    Ok(result["result"].get("value").cloned().unwrap_or(Value::Null))
}

/// `Input.dispatchKeyEvent` params for pressing `key` (a DOM key name such as
/// "Enter", or one character): down, then up.
pub(super) fn key_events(key: &str) -> [Value; 2] {
    let single = key.chars().count() == 1;
    let code = match key {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" => 13,
        "Escape" => 27,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "Delete" => 46,
        _ if single => key.chars().next().map_or(0, |c| c.to_ascii_uppercase() as u32),
        _ => 0,
    };
    let text = match key {
        "Enter" => "\r",
        "Tab" => "\t",
        _ if single => key,
        _ => "",
    };
    let down = if text.is_empty() { "rawKeyDown" } else { "keyDown" };
    [
        json!({ "type": down, "key": key, "text": text, "windowsVirtualKeyCode": code }),
        json!({ "type": "keyUp", "key": key, "windowsVirtualKeyCode": code }),
    ]
}

fn exception_text(details: &Value) -> String {
    details["exception"]["description"].as_str().or(details["text"].as_str()).unwrap_or("exception").to_string()
}

/// A console argument (a RemoteObject) as text.
fn remote_text(object: &Value) -> String {
    match object.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(value) => value.to_string(),
        None => ["unserializableValue", "description", "type"]
            .iter()
            .find_map(|k| object[*k].as_str())
            .unwrap_or_default()
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_allowed_methods_are_sent() {
        let sent = command(3, "Runtime.evaluate", json!({ "expression": "1+1" })).unwrap();
        let sent: Value = serde_json::from_str(&sent).unwrap();
        assert_eq!(sent, json!({ "id": 3, "method": "Runtime.evaluate", "params": { "expression": "1+1" } }));
        for method in ["Network.getCookies", "Storage.getCookies", "Target.attachToBrowserTarget", "Browser.close", "runtime.evaluate", ""] {
            assert_eq!(command(1, method, json!({})), None, "{method}");
        }
    }

    #[test]
    fn replies_events_and_the_end_of_a_session_are_told_apart() {
        assert_eq!(
            parse(2, r#"{"id":2,"result":{"result":{"type":"number","value":2}}}"#),
            Incoming::Reply(2, Ok(json!({ "result": { "type": "number", "value": 2 } })))
        );
        assert_eq!(parse(4, r#"{"id":4,"error":{"code":-32000,"message":"nope"}}"#), Incoming::Reply(4, Err("nope".into())));
        assert_eq!(
            parse(0, r#"{"method":"Runtime.consoleAPICalled","params":{"type":"warning","args":[{"type":"string","value":"a"},{"type":"number","value":1},{"type":"object","description":"Object"}]}}"#),
            Incoming::Console { level: ConsoleLevel::Warning, text: "a 1 Object".into() }
        );
        assert_eq!(
            parse(0, r#"{"method":"Runtime.exceptionThrown","params":{"exceptionDetails":{"text":"Uncaught","exception":{"description":"TypeError: x"}}}}"#),
            Incoming::Console { level: ConsoleLevel::Error, text: "TypeError: x".into() }
        );
        assert_eq!(parse(0, r#"{"method":"Baa.detached","params":{}}"#), Incoming::Detached);
        assert_eq!(parse(0, r#"{"method":"Runtime.executionContextCreated","params":{}}"#), Incoming::Other);
        assert_eq!(parse(0, "not json"), Incoming::Other);
    }

    #[test]
    fn an_evaluation_gives_its_value_or_its_exception() {
        assert_eq!(eval_value(json!({ "result": { "type": "number", "value": 2 } })), Ok(json!(2)));
        assert_eq!(eval_value(json!({ "result": { "type": "undefined" } })), Ok(Value::Null));
        let thrown = json!({ "result": {}, "exceptionDetails": { "text": "Uncaught", "exception": { "description": "Error: boom" } } });
        assert_eq!(eval_value(thrown), Err("Error: boom".into()));
    }

    #[test]
    fn keys_are_named_or_single_characters() {
        let [down, up] = key_events("Enter");
        assert_eq!((down["type"].as_str(), down["text"].as_str(), down["windowsVirtualKeyCode"].as_u64()), (Some("keyDown"), Some("\r"), Some(13)));
        assert_eq!(up["type"], "keyUp");
        let [down, _] = key_events("ArrowDown");
        assert_eq!((down["type"].as_str(), down["text"].as_str()), (Some("rawKeyDown"), Some("")));
        let [down, _] = key_events("a");
        assert_eq!((down["text"].as_str(), down["windowsVirtualKeyCode"].as_u64()), (Some("a"), Some(65)));
        let [down, _] = key_events("NotAKey");
        assert_eq!(down["text"], "", "an unknown name is not typed as text");
    }
}
