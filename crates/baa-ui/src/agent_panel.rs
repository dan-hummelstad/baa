//! The Claude chat panel (⌘J): runs the user's `claude` CLI against baa's own tools (PLAN-m4-features D1).
//! The child is an ordinary MCP client of `baa mcp`; it gets no credentials from here and
//! inherits the environment as it is.

use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    time::SystemTime,
};

use baa_model::{ChatRecord, Store, TabId};
use baa_web::WebHost;
use futures::{channel::mpsc, StreamExt};
use gpui::{div, prelude::*, px, AnyElement, App, Context, FocusHandle, Hsla, ScrollHandle, SharedString, Window};
use serde_json::{json, Value};

use crate::{
    browser::{cut_left, host_of, shadow, tab_title, Browser},
    icons::icon,
    input::{Key, TextField},
    library::{ink_mid, ink_soft, raised, Section},
    markdown,
    settings::Tab,
    theme::{ascii, fade, r, Theme},
};

const INSTALL: &str = "curl -fsSL https://claude.ai/install.sh | bash";

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Entry {
    User(String),
    Assistant(String),
    Thinking(String),
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

/// A chat the switcher can go back to.
#[derive(Default)]
pub(crate) struct Chat {
    /// Its row in `baa.db`; 0 until its first message.
    id: i64,
    /// An incognito tab was in its context or Space; never stored.
    private: bool,
    pub(crate) transcript: Vec<Entry>,
    /// `claude`'s id for this chat, so a switch or a restart resumes it.
    session: Option<String>,
    /// When the chat last had a message, and the site it began on.
    at: u64,
    host: String,
}

/// What `baa.db` keeps of a chat: nothing for an empty or incognito one.
fn chat_record(c: &Chat) -> Option<ChatRecord> {
    if c.private || !c.transcript.iter().any(|e| matches!(e, Entry::User(_))) {
        return None;
    }
    Some(ChatRecord {
        id: c.id,
        at: c.at as i64,
        host: c.host.clone(),
        session: c.session.clone(),
        transcript: serde_json::to_string(&c.transcript).expect("entries serialize"),
    })
}

/// Writes the chat, or removes it when it is incognito (it may have turned so after a save).
fn persist_chat(store: &Store, c: &Chat) {
    let saved = match chat_record(c) {
        Some(r) => store.save_chat(&r),
        None => store.remove_chat(c.id),
    };
    if let Err(e) = saved {
        log::error!("saving chat: {e}");
    }
}

fn restore(r: ChatRecord) -> Option<Chat> {
    Some(Chat {
        id: r.id,
        private: false,
        transcript: serde_json::from_str(&r.transcript).ok()?,
        session: r.session,
        at: r.at as u64,
        host: r.host,
    })
}

impl Panel {
    pub(crate) fn focused(&self, window: &Window, cx: &App) -> bool {
        self.focus.as_ref().is_some_and(|f| f.contains_focused(window, cx))
    }
}

#[derive(Default)]
pub(crate) struct Panel {
    focus: Option<FocusHandle>,
    pub(crate) field: TextField,
    /// The open chat.
    pub(crate) chat: Chat,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    /// A message is out and its turn has not ended.
    pub(crate) busy: bool,
    /// What the thinking row says for this turn.
    quip: &'static str,
    not_found: bool,
    not_logged_in: bool,
    /// Bumped by every kill, so a reader task of an older child stops.
    generation: u64,
    scroll: ScrollHandle,
    /// The other chats, newest first: saved ones from earlier launches and this run's.
    chats: Vec<Chat>,
    switcher: bool,
    /// Tabs added with "Add context"; the active tab counts unless `hide_active`.
    context: Vec<TabId>,
    hide_active: bool,
}

/// Claude Code's way of saying "working on it": one playful verb per turn.
const QUIPS: &[&str] = &[
    "Thinking", "Pondering", "Musing", "Noodling", "Percolating", "Mulling", "Ruminating", "Cogitating",
    "Brewing", "Tinkering", "Wrangling", "Untangling", "Simmering", "Conjuring", "Sleuthing", "Grazing",
];
/// A star that twinkles through these while the turn runs.
const TWINKLE: &[&str] = &["·", "✢", "✳", "✶", "✻", "✽", "✻", "✶", "✳", "✢"];

fn quip() -> &'static str {
    // ponytail: the clock's nanoseconds as the dice; nothing here needs real randomness.
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    QUIPS[n as usize % QUIPS.len()]
}

/// "✻ Pondering…", the star twinkling and the words breathing, until the turn ends.
fn thinking(quip: &'static str, theme: &Theme) -> impl IntoElement {
    use gpui::{pulsating_between, Animation, AnimationExt};
    let star = div().w(px(12.)).flex_none().text_color(theme.accent).with_animation(
        "chat-twinkle",
        Animation::new(std::time::Duration::from_millis(1200)).repeat(),
        |d, t| d.child(TWINKLE[((t * TWINKLE.len() as f32) as usize).min(TWINKLE.len() - 1)]),
    );
    div()
        .id("chat-thinking")
        .debug_selector(|| "chat-thinking".into())
        .flex_none()
        .px(px(4.))
        .flex()
        .items_center()
        .gap(px(7.))
        .text_size(px(12.5))
        .child(star)
        .child(div().text_color(theme.ink_muted).child(format!("{quip}…")).with_animation(
            "chat-breathe",
            Animation::new(std::time::Duration::from_millis(1600)).repeat().with_easing(pulsating_between(0.45, 1.0)),
            |d, t| d.opacity(t),
        ))
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
                "thinking" => Some(Entry::Thinking(b["thinking"].as_str()?.trim().to_string())).filter(|e| *e != Entry::Thinking(String::new())),
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

/// `read_page` reads as "Read page".
fn tool_label(name: &str) -> String {
    let s = name.replace('_', " ");
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

fn needs_login(text: &str) -> bool {
    let t = text.to_lowercase();
    ["login", "log in", "/login", "api key"].iter().any(|k| t.contains(k))
}

/// Arguments after the binary. `--mcp-config`, `--allowedTools` and `--tools` are variadic, so each is
/// followed by another flag, never by a positional; the prompt comes over stdin. `nonce` goes in the
/// MCP server's own environment, so only `baa mcp` gets it, not `claude`'s other children.
fn argv(exe: &Path, nonce: &str, resume: Option<&str>) -> Vec<String> {
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
    .into_iter()
    .chain(resume.into_iter().flat_map(|id| ["--resume", id]))
    .map(String::from)
    .collect()
}

/// The session id on a stream-json line. It becomes an argument, so only a plain id is taken.
fn session_of(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line).ok()?;
    let id = v["session_id"].as_str()?;
    let plain = !id.is_empty() && !id.starts_with('-') && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    plain.then(|| id.to_string())
}

/// What Claude is told about the context chips: tab ids for its tools, never page text.
fn with_context(text: &str, tabs: &[u64]) -> String {
    if tabs.is_empty() {
        return text.to_string();
    }
    let ids: Vec<String> = tabs.iter().map(|t| format!("tab {t}")).collect();
    format!("{text}\n\n(Context: {} in baa.)", ids.join(", "))
}

/// The switcher's name for a chat: its first message, first line.
fn title_of(transcript: &[Entry]) -> String {
    transcript
        .iter()
        .find_map(|e| match e {
            Entry::User(t) => t.lines().next().map(str::to_string),
            _ => None,
        })
        .unwrap_or_else(|| "New chat".into())
}

/// "2h ago · dailyread.news".
fn meta(now: u64, then: u64, host: &str) -> String {
    let when = match now.saturating_sub(then) {
        _ if then == 0 => String::new(),
        0..60 => "Just now".into(),
        s @ 60..3600 => format!("{}m ago", s / 60),
        s @ 3600..86_400 => format!("{}h ago", s / 3600),
        _ => crate::library::ago(now as i64, then as i64),
    };
    [when.as_str(), host].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
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
        if self.library.as_ref().is_some_and(|l| l.section == Section::Chat) {
            return self.close_library(window, cx);
        }
        self.open_chat(window, cx);
    }

    /// The Library's Chat section, with the sidebar brought out if it was collapsed.
    pub(crate) fn open_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_library(Section::Chat, cx);
        let focus = self.agent.panel.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        window.focus(&focus, cx);
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
        // A stop, a switch or quitting can cut a turn short; keep what arrived.
        self.save_chat();
    }

    fn save_chat(&self) {
        if let Some(store) = self.store() {
            persist_chat(store, &self.agent.panel.chat);
        }
    }

    /// Saved chats into the switcher, at launch.
    pub(crate) fn restore_chats(&mut self) {
        let saved = self.store().and_then(|s| s.chats(50).ok()).unwrap_or_default();
        self.agent.panel.chats = saved.into_iter().filter_map(restore).collect();
    }

    fn send_agent_message(&mut self, cx: &mut Context<Self>) {
        let p = &mut self.agent.panel;
        let text = p.field.text.trim().to_string();
        if text.is_empty() || p.busy || !self.settings.claude.enabled {
            return;
        }
        p.field.text.clear();
        p.busy = true;
        p.quip = quip();
        p.not_logged_in = false;
        p.switcher = false;
        p.chat.transcript.push(Entry::User(text.clone()));
        p.scroll.scroll_to_bottom();
        p.chat.at = baa_model::now();
        let context = self.chat_context();
        let in_incognito = |space| self.session.space(space).is_some_and(|s| s.ephemeral);
        let private = self.session.active_space().is_some_and(in_incognito)
            || context.iter().any(|t| self.session.tab(*t).is_some_and(|t| in_incognito(t.space)));
        let p = &mut self.agent.panel;
        p.chat.private |= private;
        if p.chat.id == 0 {
            p.chat.id = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(1, |d| d.as_micros() as i64);
        }
        if p.chat.host.is_empty() {
            p.chat.host = context.first().and_then(|id| self.session.tab(*id)).map(|t| host_of(&t.url)).unwrap_or_default();
        }
        let text = with_context(&text, &context.iter().map(|t| t.0).collect::<Vec<_>>());
        self.save_chat();
        let p = &mut self.agent.panel;
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
            self.agent.panel.chat.transcript.push(Entry::Notice("Claude is no longer running. Send again to restart it.".into()));
        }
        cx.notify();
    }

    fn start_agent_child(&mut self, path: Option<String>, first: &str, cx: &mut Context<Self>) {
        let p = &mut self.agent.panel;
        let Some(path) = path else {
            p.not_found = true;
            p.busy = false;
            if let Some(Entry::User(t)) = p.chat.transcript.pop() {
                p.field.text = t;
            }
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
                p.chat.transcript.push(Entry::Notice(format!("Could not start claude: {e}")));
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
            .args(argv(&exe, &nonce, self.agent.panel.chat.session.as_deref()))
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
                if let Some(id) = session_of(&line) {
                    p.chat.session = Some(id);
                }
                for e in parse_line(&line) {
                    match e {
                        Entry::Done => p.busy = false,
                        Entry::Notice(ref t) if needs_login(t) => p.not_logged_in = true,
                        Entry::Notice(_) | Entry::Assistant(_) | Entry::Thinking(_) | Entry::ToolUse { .. } | Entry::User(_) => {}
                    }
                    if e != Entry::Done && !matches!(&e, Entry::Notice(t) if needs_login(t)) {
                        p.chat.transcript.push(e);
                    }
                }
                p.scroll.scroll_to_bottom();
                if !p.busy {
                    self.save_chat();
                }
            }
            Msg::Stderr(tail) => {
                // ponytail: any stderr text is shown, the exit code is not read.
                if !tail.trim().is_empty() {
                    if needs_login(&tail) {
                        self.agent.panel.not_logged_in = true;
                    }
                    self.agent.panel.chat.transcript.push(Entry::Notice(tail));
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
            Key::Cancel if self.agent.panel.switcher => self.agent.panel.switcher = false,
            Key::Cancel => self.close_library(window, cx),
            _ => {}
        }
        cx.notify();
    }

    /// The tabs this chat points Claude at: the active tab unless dismissed, then any added.
    pub(crate) fn chat_context(&self) -> Vec<TabId> {
        let p = &self.agent.panel;
        let mut out: Vec<TabId> = self.active_tab().filter(|_| !p.hide_active).into_iter().collect();
        for id in &p.context {
            if !out.contains(id) && self.session.tab(*id).is_some() {
                out.push(*id);
            }
        }
        out
    }

    pub(crate) fn add_chat_context(&mut self, id: TabId, cx: &mut Context<Self>) {
        let active = self.active_tab();
        let p = &mut self.agent.panel;
        if Some(id) == active {
            p.hide_active = false;
        } else if !p.context.contains(&id) {
            p.context.push(id);
        }
        cx.notify();
    }

    fn close_switcher(&mut self, _: &gpui::MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.agent.panel.switcher) {
            cx.notify();
        }
    }

    /// Shelves the current chat and opens chat `i` of the switcher, or a new one.
    /// ⌘N in the chat.
    pub(crate) fn new_chat(&mut self, cx: &mut Context<Self>) {
        self.switch_chat(None, cx);
    }

    fn switch_chat(&mut self, i: Option<usize>, cx: &mut Context<Self>) {
        self.kill_agent_child();
        let p = &mut self.agent.panel;
        let next = i.filter(|i| *i < p.chats.len()).map(|i| p.chats.remove(i)).unwrap_or_default();
        let cur = std::mem::replace(&mut p.chat, next);
        if cur.transcript.iter().any(|e| matches!(e, Entry::User(_))) {
            p.chats.insert(0, cur);
        }
        p.switcher = false;
        p.context.clear();
        p.hide_active = false;
        p.not_logged_in = false;
        p.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// The Library's Chat section body (pen.dev "baa · Side Panel — Chat").
    pub(crate) fn render_agent_panel(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = &self.agent.panel;
        let focus = p.focus.clone()?;
        let (mid, soft) = (ink_mid(theme), ink_soft(theme));
        // The mock's milky whites on the gradient; ASCII Chrome's navy boxes.
        let milk = |a: f32| if ascii() { theme.glass_strong } else { Hsla { a, ..gpui::white() } };
        let btn = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .h(px(24.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(r(6.))
                .bg(theme.glass_strong)
                .text_size(px(12.))
                .text_color(theme.ink)
                .hover(|d| d.bg(theme.hover))
                .child(label)
        };
        let icon_btn = |id: &'static str, name: &'static str, size: f32| {
            div()
                .id(id)
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(r(6.))
                .hover(|d| d.bg(theme.hover))
                .child(icon(name, size, theme.ink_muted))
        };
        let header = div()
            .flex_none()
            .relative()
            .mt(px(6.))
            .h(px(34.))
            .flex()
            .items_center()
            .justify_center()
            .child(
                raised(
                    div()
                        .id("chat-title")
                        .max_w(px(170.))
                        .h(px(29.))
                        .px(px(11.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .rounded(r(9.)),
                    theme,
                )
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.agent.panel.switcher ^= true;
                    cx.notify();
                }))
                .child(div().min_w_0().truncate().text_size(px(14.)).font_weight(gpui::FontWeight::BOLD).child(title_of(&p.chat.transcript)))
                .child(icon(if p.switcher { "chevron-up" } else { "chevron-down" }, 14., theme.ink_muted)),
            )
            .child(
                div()
                    .absolute()
                    .right_0()
                    .flex()
                    .gap(px(2.))
                    .child(icon_btn("agent-close", "x", 16.).on_click(cx.listener(|this, _, window, cx| this.close_library(window, cx)))),
            );

        let now = baa_model::now();
        let switcher = p.switcher.then(|| {
            let row = |id: usize, title: String, meta: String, current: bool| {
                div()
                    .id(("chat-row", id))
                    .px(px(14.))
                    .py(px(7.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .when(current, |d| d.bg(fade(theme.ink, 0.04)))
                    .hover(|d| d.bg(fade(theme.ink, 0.06)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.5))
                                    .font_weight(if current { gpui::FontWeight::SEMIBOLD } else { gpui::FontWeight::MEDIUM })
                                    .child(title),
                            )
                            .when(!meta.is_empty(), |d| d.child(div().truncate().text_size(px(10.5)).text_color(soft).child(meta))),
                    )
                    .when(current, |d| d.child(icon("check", 14., theme.ink)))
            };
            let has_current = p.chat.transcript.iter().any(|e| matches!(e, Entry::User(_)));
            raised(div().id("chat-switcher").absolute().top(px(46.)).left_0().right_0().py(px(7.)).flex().flex_col().rounded(r(14.)), theme)
                .shadow(vec![shadow(Hsla { a: 0.12, ..theme.ink }, 8., 28.)])
                .occlude()
                .child(
                    div()
                        .px(px(14.))
                        .pt(px(6.))
                        .pb(px(7.))
                        .text_size(px(9.5))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(soft)
                        .child("RECENT CHATS"),
                )
                .when(has_current, |d| d.child(row(0, title_of(&p.chat.transcript), meta(now, p.chat.at, &p.chat.host), true)))
                .children(p.chats.iter().take(8).enumerate().map(|(i, c)| {
                    row(i + 1, title_of(&c.transcript), meta(now, c.at, &c.host), false)
                        .on_click(cx.listener(move |this, _, _, cx| this.switch_chat(Some(i), cx)))
                }))
                .when(!has_current && p.chats.is_empty(), |d| {
                    d.child(div().px(px(14.)).py(px(7.)).text_size(px(12.5)).text_color(theme.ink_muted).child("No other chats yet"))
                })
                .child(div().my(px(6.)).h(px(1.)).bg(theme.divider))
                .child(
                    div()
                        .id("chat-new")
                        .px(px(14.))
                        .py(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(9.))
                        .hover(|d| d.bg(fade(theme.ink, 0.06)))
                        .on_click(cx.listener(|this, _, _, cx| this.switch_chat(None, cx)))
                        .child(icon("plus", 15., theme.ink))
                        .child(div().flex_1().text_size(px(12.5)).font_weight(gpui::FontWeight::SEMIBOLD).child("New chat"))
                        .child(div().text_size(px(10.5)).text_color(soft).child("⌘N")),
                )
        });

        let step = |name: &'static str, label: String, detail: String| {
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .text_size(px(11.5))
                .child(icon(name, 12., soft))
                .child(div().flex_none().font_weight(gpui::FontWeight::MEDIUM).text_color(mid).child(label))
                .when(!detail.is_empty(), |d| d.child(div().min_w_0().truncate().text_color(soft).child(detail)))
        };
        let transcript = div()
            .id("agent-transcript")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&p.scroll)
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(Self::close_switcher))
            .pt(px(16.))
            .pb(px(12.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .text_size(px(12.5))
            .line_height(gpui::relative(1.5))
            .children(p.chat.transcript.iter().enumerate().filter_map(|(i, e)| {
                let row = div().id(("agent-entry", i)).flex_none();
                Some(match e {
                    Entry::User(t) => row.flex().justify_end().child(
                        div()
                            .debug_selector(|| "chat-user".into())
                            .max_w(gpui::relative(0.8))
                            .px(px(13.))
                            .py(px(10.))
                            .rounded_tl(r(14.))
                            .rounded_tr(r(14.))
                            .rounded_bl(r(14.))
                            .rounded_br(r(4.))
                            .bg(milk(0.56))
                            .text_color(theme.ink)
                            .child(SharedString::from(t.clone())),
                    ),
                    Entry::Assistant(t) => row.debug_selector(|| "chat-reply".into()).px(px(4.)).text_color(theme.ink).child(markdown::render(&markdown::parse(t), theme)),
                    // Claude.ai and ChatGPT show reasoning as a quiet, indented aside, not as the answer.
                    Entry::Thinking(t) => row
                        .px(px(4.))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(step("sparkles", "Thinking".into(), String::new()))
                        .child(
                            div()
                                .ml(px(5.))
                                .pl(px(12.))
                                .border_l_1()
                                .border_color(fade(theme.ink, 0.15))
                                .text_size(px(11.5))
                                .text_color(soft)
                                // ponytail: clamped preview, no expand toggle; add one if long reasoning must be readable here.
                                .line_clamp(4)
                                .child(SharedString::from(t.clone())),
                        ),
                    Entry::ToolUse { name, summary } => row.px(px(4.)).child(step("terminal", tool_label(name), summary.clone())),
                    Entry::Notice(t) => row.px(px(4.)).text_color(theme.accent).child(SharedString::from(t.clone())),
                    Entry::Done => return None,
                })
            }))
            .when(p.busy, |d| d.child(thinking(p.quip, theme)));

        let note = |text: &'static str| div().text_size(px(12.5)).text_color(theme.ink).child(text);
        let foot = div()
            .flex_none()
            .pt(px(9.))
            .border_t_1()
            .border_color(theme.divider)
            .flex()
            .flex_col()
            .gap(px(9.))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(Self::close_switcher));
        let foot = if !self.settings.claude.enabled {
            foot.child(note("Claude is off")).child(btn("agent-open-settings", "Open Settings").on_click(cx.listener(|this, _, _, cx| {
                this.open_settings_at(Some(Tab::Claude), None, cx)
            })))
        } else if p.not_found {
            foot.child(note("Install Claude Code"))
                .child(div().text_size(px(11.5)).font_family("Menlo").text_color(theme.ink_muted).child(INSTALL))
                .child(note("or choose its path in Settings → Claude"))
                .child(btn("agent-open-settings", "Choose path in Settings").on_click(cx.listener(|this, _, _, cx| {
                    this.open_settings_at(Some(Tab::Claude), None, cx)
                })))
        } else {
            let active = self.active_tab();
            let chips = self.chat_context().into_iter().filter_map(|id| {
                let tab = self.session.tab(id)?;
                let fav = tab
                    .favicon
                    .as_ref()
                    .and_then(|f| self.favicons.get(f).cloned())
                    .map(|image| gpui::img(image).size(px(11.)).rounded(r(3.)).into_any_element())
                    .unwrap_or_else(|| icon("globe", 11., theme.ink_muted).into_any_element());
                let host = host_of(&tab.url);
                Some(
                    div()
                        .id(("chat-context", id.0))
                        .flex_none()
                        .max_w(px(170.))
                        .h(px(21.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .rounded(r(7.))
                        .bg(milk(0.54))
                        .child(fav)
                        .child(cut_left(div().min_w_0().text_size(px(11.)).font_weight(gpui::FontWeight::MEDIUM).text_color(mid)).child(
                            if host.is_empty() { SharedString::from(tab_title(tab).to_string()) } else { host.into() },
                        ))
                        .child(
                            div()
                                .id(("chat-context-x", id.0))
                                .flex_none()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let p = &mut this.agent.panel;
                                    p.context.retain(|t| *t != id);
                                    if Some(id) == active {
                                        p.hide_active = true;
                                    }
                                    cx.notify();
                                }))
                                .child(icon("x", 10., soft)),
                        ),
                )
            });
            let add = div()
                .id("chat-add-context")
                .flex_none()
                .h(px(21.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(4.))
                .rounded(r(7.))
                .bg(theme.glass)
                .hover(|d| d.bg(theme.hover))
                .text_size(px(11.))
                .text_color(theme.ink_muted)
                .on_click(cx.listener(|this, e: &gpui::ClickEvent, window, cx| {
                    this.open_menu(crate::browser::MenuKind::ChatContext, e.position(), window, cx)
                }))
                .child(icon("plus", 10., theme.ink_muted))
                .child("Add context");
            let ready = !p.field.text.trim().is_empty() && !p.busy;
            foot.when(p.not_logged_in, |d| d.child(note("Run `claude` once in Terminal to log in")))
                .child(div().flex().flex_wrap().gap(px(6.)).children(chips).child(add))
                .child(
                    div()
                        .min_h(px(38.))
                        .py(px(9.))
                        .px(px(11.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded(r(11.))
                        .border_1()
                        .border_color(if ascii() { theme.accent } else { Hsla { a: 0.77, ..gpui::white() } })
                        .bg(milk(0.68))
                        .text_size(px(12.5))
                        .child(div().debug_selector(|| "chat-field".into()).flex_1().min_w_0().child(p.field.render_wrapped(
                            "Ask baa…",
                            theme.ink,
                            soft,
                            Hsla { a: 0.25, ..theme.accent },
                        )))
                        .child(if p.busy {
                            div()
                                .id("agent-stop")
                                .flex_none()
                                .size(px(22.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(r(6.))
                                .bg(theme.ink)
                                .on_click(cx.listener(|this, _, _, cx| this.stop_agent(cx)))
                                .child(icon("square", 10., theme.glass_strong))
                        } else {
                            div()
                                .id("agent-send")
                                .flex_none()
                                .on_click(cx.listener(|this, _, _, cx| this.send_agent_message(cx)))
                                .child(icon("arrow-up", 16., if ready { theme.ink } else { theme.ink_muted }))
                        }),
                )
        };

        Some(
            div()
                .id("agent-panel")
                .debug_selector(|| "agent-panel-body".into())
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .track_focus(&focus)
                .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| this.on_panel_key(e, window, cx)))
                .child(header)
                .child(transcript)
                .child(foot)
                .children(switcher)
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
    fn thinking_is_its_own_entry_and_tools_read_as_words() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":" plan "},{"type":"thinking","thinking":""}]}}"#;
        assert_eq!(parse_line(line), vec![Entry::Thinking("plan".into())]);
        assert_eq!(tool_label("read_page"), "Read page");
        assert_eq!(tool_label(""), "");
    }

    #[test]
    fn chats_round_trip_and_incognito_ones_never_reach_the_store() {
        let store = Store::open(":memory:").unwrap();
        let chat = |id, private| Chat {
            id,
            private,
            transcript: vec![Entry::User("hi".into()), Entry::Thinking("t".into()), Entry::Assistant("**yo**".into())],
            session: Some("s1".into()),
            at: 5,
            host: "a.example".into(),
        };
        persist_chat(&store, &chat(1, false));
        persist_chat(&store, &chat(2, true));
        persist_chat(&store, &Chat { id: 3, ..Chat::default() });
        let saved = store.chats(10).unwrap();
        assert_eq!(saved.iter().map(|r| r.id).collect::<Vec<_>>(), [1], "incognito and empty chats are not written");
        let back = restore(saved.into_iter().next().unwrap()).unwrap();
        assert_eq!((back.transcript, back.session, back.host), (chat(1, false).transcript, chat(1, false).session, chat(1, false).host));
        persist_chat(&store, &chat(1, true));
        assert!(store.chats(10).unwrap().is_empty(), "a saved chat that turns incognito is removed");
    }

    #[test]
    fn garbage_and_other_types_are_ignored() {
        assert!(parse_line("not json").is_empty());
        assert!(parse_line(r#"{"type":"system","subtype":"init"}"#).is_empty());
    }

    #[test]
    fn argv_keeps_variadic_flags_apart_and_has_no_positional_prompt() {
        let a = argv(Path::new("/Applications/baa.app/Contents/MacOS/baa"), "n0nce", Some("5f1c-aa"));
        assert_eq!(a[a.len() - 2..], ["--resume", "5f1c-aa"]);
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
    fn only_plain_session_ids_become_arguments() {
        assert_eq!(session_of(r#"{"type":"system","session_id":"9a1e-77b0"}"#).as_deref(), Some("9a1e-77b0"));
        for bad in ["--mcp-config", "a b", "../x", ""] {
            assert_eq!(session_of(&json!({"session_id": bad}).to_string()), None, "{bad:?}");
        }
        assert_eq!(session_of("not json"), None);
    }

    #[test]
    fn context_names_tab_ids_only() {
        assert_eq!(with_context("hi", &[]), "hi");
        assert_eq!(with_context("hi", &[12, 40]), "hi\n\n(Context: tab 12, tab 40 in baa.)");
    }

    #[test]
    fn switcher_rows_read_like_the_mock() {
        let t = vec![Entry::Assistant("x".into()), Entry::User("Lofoten trip plan\nmore".into())];
        assert_eq!(title_of(&t), "Lofoten trip plan");
        assert_eq!(title_of(&[]), "New chat");
        let now = 100 * 86_400;
        assert_eq!(meta(now, now - 2 * 3600, "dailyread.news"), "2h ago · dailyread.news");
        assert_eq!(meta(now, now - 86_400, ""), "Yesterday");
        assert_eq!(meta(now, 0, "a.com"), "a.com");
    }

    #[test]
    fn user_line_is_one_json_line() {
        let l = user_line("a\nb");
        assert_eq!(l.matches('\n').count(), 1);
        assert_eq!(serde_json::from_str::<Value>(&l).unwrap()["message"]["content"][0]["text"], "a\nb");
    }
}
