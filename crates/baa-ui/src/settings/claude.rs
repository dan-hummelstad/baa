//! Claude: the switch for baa's tool server, what Claude may see, and how to reach it from a terminal.

use baa_model::Settings;
use baa_web::WebHost;
use gpui::{div, prelude::*, px, Context, SharedString};

use super::{Field, Item, SettingsView};
use crate::theme::surface;

impl<H: WebHost> SettingsView<H> {
    pub(super) fn claude(&self, s: &Settings, cx: &mut Context<Self>) -> Vec<Item> {
        let on = s.claude.enabled;
        let enabled = self
            .switch("pref-claude", on)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.motion.from(("pref-claude", 0), crate::motion::Feel::Glide, on as u8 as f32, !on as u8 as f32);
                // Through the browser: turning it off closes the socket and kills the child.
                this.browser.update(cx, |b, cx| b.set_claude_enabled(!on, cx));
            }))
            .into_any_element();
        let mut out = vec![
            self.section("Claude"),
            self.row("claude_enabled", "Claude Code, in the panel (⌘J) or in a terminal, can read your tabs in the Spaces below. Acting on a site asks you first.", enabled),
            self.section("Access"),
            self.row("claude_spaces", "Incognito windows are never visible.", div().into_any_element()),
        ];
        let spaces: Vec<_> = self.browser.read(cx).session.spaces().iter().filter(|s| !s.ephemeral).map(|s| (s.id, format!("{} {}", s.icon, s.name))).collect();
        for (id, name) in spaces {
            let allowed = s.claude.allowed_spaces.contains(&id);
            let el = div()
                .id(SharedString::from(format!("pref-claude-space:{}", id.0)))
                .debug_selector(move || format!("pref-claude-space:{}", id.0))
                .child(self.switch("pref-claude-space", allowed))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.change(cx, |s| {
                        let v = &mut s.claude.allowed_spaces;
                        match v.iter().position(|x| *x == id) {
                            Some(i) => drop(v.remove(i)),
                            None => v.push(id),
                        }
                    })
                }))
                .into_any_element();
            out.push(self.sub_row(format!("claude-space:{}", id.0), SharedString::from(name).into_any_element(), el));
        }
        out.push(self.row(
            "claude_evaluate",
            "Run any JavaScript in a page. Asks every time.",
            self.toggle("pref-claude-evaluate", s.claude.evaluate_allowed, |s, v| s.claude.evaluate_allowed = v, cx),
        ));
        let found = match (&s.claude.claude_path, &s.claude.found_path) {
            (None, Some(p)) => format!("Found at {p}. Leave empty to find it automatically."),
            _ => "Leave empty to find it automatically.".into(),
        };
        out.push(self.row("claude_path", &found, self.field(Field::ClaudePath, "claude-path", "/path/to/claude", 220., cx)));
        out.push(self.section("Terminal"));
        // No token while off, so no command to copy.
        if !on {
            out.push(self.row("claude_terminal", "Turn Claude on to get the command for a terminal.", div().into_any_element()));
            return out;
        }
        let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "baa".into());
        // Quoted for the shell it is pasted into.
        let exe = if exe.contains(char::is_whitespace) { format!("'{}'", exe.replace('\'', "'\\''")) } else { exe };
        // The token pairs this command with this enabling of Claude; turning Claude off and on makes a new one.
        let command = format!("claude mcp add baa -e BAA_AGENT_TOKEN={} -- {exe} mcp", s.claude.token);
        let copy = command.clone();
        out.push(self.row(
            "claude_terminal",
            "Run it in a terminal. After turning Claude off and on, run `claude mcp remove baa` and add it again.",
            self.button("claude-copy", "Copy", false, cx)
                .on_click(cx.listener(move |_, _, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy.clone()))))
                .into_any_element(),
        ));
        out.push((None, div().px(px(10.)).pb(px(8.)).text_size(px(11.5)).font_family("Menlo").text_color(surface::muted()).child(command).into_any_element()));
        out
    }
}
