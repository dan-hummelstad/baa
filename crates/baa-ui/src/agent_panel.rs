//! The Claude chat panel (⌘J): runs the user's `claude` CLI against baa's own tools (PLAN-m4-features D1).
//! The child is an ordinary MCP client of `baa mcp`; it gets no credentials from here and
//! inherits the environment as it is.

use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    time::SystemTime,
};

use baa_model::SidebarSide;
use baa_web::WebHost;
use futures::{channel::mpsc, StreamExt};
use gpui::{div, prelude::*, px, AnyElement, Context, FocusHandle, ScrollHandle, SharedString, Window};
use serde_json::{json, Value};

use crate::{
    browser::{Browser, INSET},
    icons::icon,
    input::{Key, TextField},
    settings::Tab,
    theme::{r, surface, Theme},
};

pub(crate) const PANEL_WIDTH: f32 = 360.0;

const INSTALL: &str = "curl -fsSL https://claude.ai/install.sh | bash";

#[derive(Debug, PartialEq)]
pub(crate) enum Entry {
    User(String),
    Assistant(String),
    ToolUse { name: String, summary: String },
    Notice(String),
    /// The turn ended; not drawn.
    Done,
}

enum Msg {
    Line(String),
    Stderr(String),
    Closed,
}

#[derive(Default)]
pub(crate) struct Panel {
    pub open: bool,
    focus: Option<FocusHandle>,
    field: TextField,
    transcript: Vec<Entry>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    /// A message is out and its turn has not ended.
    busy: bool,
    not_found: bool,
    not_logged_in: bool,
    /// Bumped by every kill, so a reader task of an older child stops.
    generation: u64,
    scroll: ScrollHandle,
}

/// One stream-json line from the child as transcript entries; anything unknown is nothing.
fn parse_line(line: &str) -> Vec<Entry> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return Vec::new() };
    match v["type"].as_str() {
        Some("assistant") => v["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|b| match b["type"].as_str()? {
                "text" => Some(Entry::Assistant(b["text"].as_str()?.trim().to_string())).filter(|e| *e != Entry::Assistant(String::new())),
                "tool_use" => Some(Entry::ToolUse {
                    name: b["name"].as_str()?.trim_start_matches("mcp__baa__").to_string(),
                    summary: summarize(&b["input"]),
                }),
                _ => None,
            })
            .collect(),
        Some("result") => {
            let mut out = Vec::new();
            if v["is_error"].as_bool() == Some(true) {
                let text = v["result"].as_str().unwrap_or("Claude stopped with an error.");
                out.push(Entry::Notice(text.to_string()));
            }
            out.push(Entry::Done);
            out
        }
        _ => Vec::new(),
    }
}

/// `key=value` pairs of a tool's input on one line.
fn summarize(input: &Value) -> String {
    let Some(map) = input.as_object() else { return String::new() };
    let s: Vec<String> = map
        .iter()
        .map(|(k, v)| format!("{k}={}", v.as_str().map_or_else(|| v.to_string(), str::to_string)))
        .collect();
    let s = s.join(" ");
    match s.char_indices().nth(80) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
}

fn needs_login(text: &str) -> bool {
    let t = text.to_lowercase();
    ["login", "log in", "/login", "api key"].iter().any(|k| t.contains(k))
}

/// Arguments after the binary. `--mcp-config`, `--allowedTools` and `--tools` are variadic, so each is
/// followed by another flag, never by a positional; the prompt comes over stdin. `nonce` goes in the
/// MCP server's own environment, so only `baa mcp` gets it, not `claude`'s other children.
fn argv(exe: &Path, nonce: &str) -> Vec<String> {
    let config = json!({"mcpServers": {"baa": {"type": "stdio", "command": exe.to_string_lossy(), "args": ["mcp"], "env": {"BAA_AGENT_NONCE": nonce}}}});
    [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--mcp-config",
        &config.to_string(),
        "--strict-mcp-config",
        "--allowedTools",
        "mcp__baa__*",
        "--tools",
        "",
        "--permission-prompt-tool",
        "mcp__baa__approve",
        "--setting-sources",
        "user",
    ]
    .map(String::from)
    .into()
}

fn user_line(text: &str) -> String {
    json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": text}]}}).to_string() + "\n"
}

/// Mirrors `data_dir` in crates/baa/src/main.rs.
fn agent_dir() -> PathBuf {
    let var = |name| std::env::var_os(name).map(PathBuf::from);
    let base = var("BAA_DATA_DIR").unwrap_or_else(|| {
        var("HOME").map(|h| h.join("Library/Application Support")).unwrap_or_else(std::env::temp_dir).join("baa")
    });
    base.join("agent")
}

/// Off the main thread: a login shell is slow. The absolute, existing path `command -v claude` prints.
fn resolve_claude(configured: Option<String>, found: Option<String>) -> Option<String> {
    let exists = |p: &String| Path::new(p).is_absolute() && Path::new(p).exists();
    if let Some(p) = configured.filter(|p| !p.is_empty()) {
        return Some(p);
    }
    if let Some(p) = found.filter(exists) {
        return Some(p);
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let out = Command::new(shell).args(["-lc", "command -v claude"]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().map(|l| l.trim().to_string()).find(exists)
}

impl<H: WebHost> Browser<H> {
    pub(crate) fn toggle_agent_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.agent.panel.open {
            return self.close_agent_panel(window, cx);
        }
        let panel = &mut self.agent.panel;
        panel.open = true;
        let focus = panel.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        window.focus(&focus, cx);
        self.agent_w.set_target(PANEL_WIDTH);
        cx.notify();
    }

    pub(crate) fn close_agent_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.agent.panel.open = false;
        self.agent_w.set_target(0.0);
        self.focus_page(window, cx);
        cx.notify();
    }

    /// Kills the child and reaps it off the main thread.
    pub(crate) fn kill_agent_child(&mut self) {
        let p = &mut self.agent.panel;
        p.generation += 1;
        p.stdin = None;
        p.busy = false;
        self.agent.nonce = None;
        if let Some(mut child) = p.child.take() {
            let _ = child.kill();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }

    fn send_agent_message(&mut self, cx: &mut Context<Self>) {
        let p = &mut self.agent.panel;
        let text = p.field.text.trim().to_string();
        if text.is_empty() || p.busy || !self.settings.claude.enabled {
            return;
        }
        p.field.text.clear();
        p.busy = true;
        p.not_logged_in = false;
        p.transcript.push(Entry::User(text.clone()));
        p.scroll.scroll_to_bottom();
        if p.stdin.is_some() {
            return self.write_agent_line(&text, cx);
        }
        let generation = p.generation;
        let claude = self.settings.claude.clone();
        let found = cx.background_spawn(async move { resolve_claude(claude.claude_path, claude.found_path) });
        cx.spawn(async move |this, cx| {
            let path = found.await;
            let _ = this.update(cx, |this, cx| {
                if this.agent.panel.generation == generation {
                    this.start_agent_child(path, &text, cx);
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn write_agent_line(&mut self, text: &str, cx: &mut Context<Self>) {
        let p = &mut self.agent.panel;
        let ok = p.stdin.as_mut().is_some_and(|s| s.write_all(user_line(text).as_bytes()).and_then(|_| s.flush()).is_ok());
        if !ok {
            self.kill_agent_child();
            self.agent.panel.transcript.push(Entry::Notice("Claude is no longer running. Send again to restart it.".into()));
        }
        cx.notify();
    }

    fn start_agent_child(&mut self, path: Option<String>, first: &str, cx: &mut Context<Self>) {
        let p = &mut self.agent.panel;
        let Some(path) = path else {
            p.not_found = true;
            p.busy = false;
            p.field.text = first.to_string();
            p.transcript.pop();
            return cx.notify();
        };
        p.not_found = false;
        let c = &mut self.settings.claude;
        if c.claude_path.as_deref().is_none_or(str::is_empty) && c.found_path.as_deref() != Some(&path) {
            c.found_path = Some(path.clone());
            self.save_settings(cx);
        }
        match self.spawn_agent(&path, cx) {
            Ok(()) => self.write_agent_line(first, cx),
            Err(e) => {
                let p = &mut self.agent.panel;
                p.busy = false;
                p.transcript.push(Entry::Notice(format!("Could not start claude: {e}")));
                cx.notify();
            }
        }
    }

    fn spawn_agent(&mut self, claude: &str, cx: &mut Context<Self>) -> std::io::Result<()> {
        let exe = std::env::current_exe()?;
        let dir = agent_dir();
        std::fs::create_dir_all(&dir)?;
        // The hello of the `baa mcp` this child starts: lets it in without the connect sheet, once.
        let nonce = crate::agent::secret();
        let mut child = Command::new(claude)
            .args(argv(&exe, &nonce))
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
        let (tx, mut rx) = mpsc::unbounded();
        if let Some(out) = stdout {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    let _ = tx.unbounded_send(Msg::Line(line));
                }
                let _ = tx.unbounded_send(Msg::Closed);
            });
        }
        if let Some(mut err) = stderr {
            std::thread::spawn(move || {
                let mut s = String::new();
                let _ = err.read_to_string(&mut s);
                let tail: Vec<&str> = s.lines().rev().take(3).collect();
                let _ = tx.unbounded_send(Msg::Stderr(tail.into_iter().rev().collect::<Vec<_>>().join("\n")));
            });
        }
        self.agent.nonce = Some((nonce, cx.background_executor().now()));
        let p = &mut self.agent.panel;
        p.stdin = child.stdin.take();
        p.child = Some(child);
        let generation = p.generation;
        cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let alive = this.update(cx, |this, cx| {
                    if this.agent.panel.generation != generation {
                        return false;
                    }
                    this.on_agent_msg(msg, cx);
                    true
                });
                if !matches!(alive, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        Ok(())
    }

    fn on_agent_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Line(line) => {
                let p = &mut self.agent.panel;
                for e in parse_line(&line) {
                    match e {
                        Entry::Done => p.busy = false,
                        Entry::Notice(ref t) if needs_login(t) => p.not_logged_in = true,
                        Entry::Notice(_) | Entry::Assistant(_) | Entry::ToolUse { .. } | Entry::User(_) => {}
                    }
                    if e != Entry::Done && !matches!(&e, Entry::Notice(t) if needs_login(t)) {
                        p.transcript.push(e);
                    }
                }
                p.scroll.scroll_to_bottom();
            }
            Msg::Stderr(tail) => {
                // ponytail: any stderr text is shown, the exit code is not read.
                if !tail.trim().is_empty() {
                    if needs_login(&tail) {
                        self.agent.panel.not_logged_in = true;
                    }
                    self.agent.panel.transcript.push(Entry::Notice(tail));
                }
            }
            Msg::Closed => {
                self.agent.nonce = None;
                let p = &mut self.agent.panel;
                p.generation += 1;
                p.stdin = None;
                p.busy = false;
                if let Some(mut child) = p.child.take() {
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
            }
        }
        cx.notify();
    }

    fn on_panel_key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.agent.panel.field.handle(&event.keystroke, cx);
        match key {
            Key::Submit if !event.is_held => self.send_agent_message(cx),
            Key::Cancel => self.close_agent_panel(window, cx),
            _ => {}
        }
        cx.notify();
    }

    pub(crate) fn render_agent_panel(&self, theme: &Theme, _window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = &self.agent.panel;
        // Drawn while it slides shut too.
        if !p.open && self.agent_w.value() < 0.5 {
            return None;
        }
        let focus = p.focus.clone()?;
        let btn = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .h(px(24.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(r(6.))
                .bg(surface::key())
                .text_size(px(12.))
                .text_color(surface::ink())
                .hover(|d| d.bg(surface::line()))
                .child(label)
        };
        let running = p.child.is_some() || self.agent.connected();
        let header = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(12.))
            .py(px(8.))
            .child(div().flex_1().text_size(px(13.)).font_weight(gpui::FontWeight::SEMIBOLD).child("Claude"))
            .when(running, |d| d.child(btn("agent-stop", "Stop").on_click(cx.listener(|this, _, _, cx| this.stop_agent(cx)))))
            .child(
                div()
                    .id("agent-close")
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(r(6.))
                    .hover(|d| d.bg(surface::key()))
                    .on_click(cx.listener(|this, _, window, cx| this.close_agent_panel(window, cx)))
                    .child(icon("x", 13., surface::icon().into())),
            );

        let transcript = div()
            .id("agent-transcript")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&p.scroll)
            .px(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .text_size(px(12.5))
            .children(p.transcript.iter().enumerate().filter_map(|(i, e)| {
                let row = div().id(("agent-entry", i));
                Some(match e {
                    Entry::User(t) => row.p(px(8.)).rounded(r(8.)).bg(surface::key()).child(SharedString::from(t.clone())),
                    Entry::Assistant(t) => row.text_color(surface::ink()).child(SharedString::from(t.clone())),
                    Entry::ToolUse { name, summary } => row
                        .text_size(px(11.5))
                        .text_color(surface::muted())
                        .child(SharedString::from(format!("{name} {summary}"))),
                    Entry::Notice(t) => row.text_color(theme.accent).child(SharedString::from(t.clone())),
                    Entry::Done => return None,
                })
            }));

        let log = (!self.agent.activity.is_empty()).then(|| {
            div()
                .id("agent-activity")
                .max_h(px(110.))
                .overflow_y_scroll()
                .px(px(12.))
                .py(px(6.))
                .border_t_1()
                .border_color(surface::border())
                .text_size(px(11.))
                .text_color(surface::muted())
                .children(self.agent.activity.iter().rev().take(30).enumerate().map(|(i, a)| {
                    let at = a.at.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                    // UTC: no timezone database here.
                    let (h, m, s) = (at / 3600 % 24, at / 60 % 60, at % 60);
                    let mark = if a.refused { " · refused" } else { "" };
                    div().id(("agent-act", i)).truncate().child(SharedString::from(format!("{} · {} · {h:02}:{m:02}:{s:02}{mark}", a.tool, a.target)))
                }))
        });

        let note = |text: &'static str| div().text_size(px(12.5)).text_color(surface::ink()).child(text);
        let foot = div().px(px(12.)).py(px(10.)).border_t_1().border_color(surface::border()).flex().flex_col().gap(px(6.));
        let foot = if !self.settings.claude.enabled {
            foot.child(note("Claude is off")).child(btn("agent-open-settings", "Open Settings").on_click(cx.listener(|this, _, _, cx| {
                this.open_settings_at(Some(Tab::Claude), None, cx)
            })))
        } else if p.not_found {
            foot.child(note("Install Claude Code"))
                .child(div().text_size(px(11.5)).font_family("Menlo").text_color(surface::muted()).child(INSTALL))
                .child(note("or choose its path in Settings → Claude"))
                .child(btn("agent-open-settings", "Choose path in Settings").on_click(cx.listener(|this, _, _, cx| {
                    this.open_settings_at(Some(Tab::Claude), None, cx)
                })))
        } else {
            foot.when(p.not_logged_in, |d| d.child(note("Run `claude` once in Terminal to log in")))
                .child(
                    div()
                        .h(px(30.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .rounded(r(7.))
                        .border_1()
                        .border_color(theme.accent)
                        .bg(surface::key())
                        .text_size(px(12.5))
                        .child(p.field.render("Ask Claude", surface::ink().into(), surface::muted().into(), gpui::Hsla { a: 0.25, ..theme.accent })),
                )
        };

        // Docked like the sidebar: a fixed-width sheet in a slot the browser widens, clipped
        // while it slides. Its outer padding stands in for the row's inset on that side.
        let right = self.side() != SidebarSide::Right;
        Some(
            div()
                .id("agent-panel")
                .w(px(PANEL_WIDTH))
                .h_full()
                .when(right, |d| d.pr(px(INSET)).pl(px(10.)))
                .when(!right, |d| d.pl(px(INSET)).pr(px(10.)))
                .pb(px(4.))
                .flex()
                .flex_col()
                .track_focus(&focus)
                .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| this.on_panel_key(e, window, cx)))
                .child(header)
                .child(transcript)
                .children(log)
                .child(foot)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_text_and_tool_use_strip_the_prefix() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":" Hi "},{"type":"tool_use","name":"mcp__baa__list_tabs","input":{"space":"Work"}}]}}"#;
        assert_eq!(
            parse_line(line),
            [Entry::Assistant("Hi".into()), Entry::ToolUse { name: "list_tabs".into(), summary: "space=Work".into() }]
        );
    }

    #[test]
    fn result_errors_become_notices_and_end_the_turn() {
        let line = r#"{"type":"result","is_error":true,"result":"Invalid API key · Please run /login"}"#;
        let e = parse_line(line);
        assert_eq!(e.len(), 2);
        assert!(matches!(&e[0], Entry::Notice(t) if needs_login(t)));
        assert_eq!(e[1], Entry::Done);
        assert_eq!(parse_line(r#"{"type":"result","is_error":false,"result":"ok"}"#), [Entry::Done]);
        assert!(!needs_login("rate limited"));
    }

    #[test]
    fn garbage_and_other_types_are_ignored() {
        assert!(parse_line("not json").is_empty());
        assert!(parse_line(r#"{"type":"system","subtype":"init"}"#).is_empty());
    }

    #[test]
    fn argv_keeps_variadic_flags_apart_and_has_no_positional_prompt() {
        let a = argv(Path::new("/Applications/baa.app/Contents/MacOS/baa"), "n0nce");
        let at = |f: &str| a.iter().position(|x| x == f).unwrap_or_else(|| panic!("{f} missing"));
        assert_eq!(a[at("--tools") + 1], "");
        assert_eq!(a[at("--permission-prompt-tool") + 1], "mcp__baa__approve");
        at("--strict-mcp-config");
        // Every arg is a flag or follows one, so nothing positional can be swallowed by a variadic.
        for (i, x) in a.iter().enumerate() {
            assert!(x.starts_with('-') || a[i - 1].starts_with("--"), "positional {x:?}");
        }
        let cfg: Value = serde_json::from_str(&a[at("--mcp-config") + 1]).unwrap();
        assert_eq!(cfg["mcpServers"]["baa"]["args"][0], "mcp");
        assert_eq!(cfg["mcpServers"]["baa"]["env"]["BAA_AGENT_NONCE"], "n0nce", "only the MCP server gets the nonce");
    }

    #[test]
    fn user_line_is_one_json_line() {
        let l = user_line("a\nb");
        assert_eq!(l.matches('\n').count(), 1);
        assert_eq!(serde_json::from_str::<Value>(&l).unwrap()["message"]["content"][0]["text"], "a\nb");
    }
}
