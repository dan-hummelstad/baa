//! Extensions' toolbar actions: badges, the "activate the extension" shortcut, and the
//! popup an extension opens itself.

use super::*;
use baa_web::{ExtensionActionChanged, ExtensionPopupOpened};

/// Chrome shows no more of a badge than this.
const BADGE_CHARS: usize = 4;

/// What the badge says on screen: one line, no control characters, at most four characters.
pub(crate) fn badge_text(text: &str) -> String {
    plain_text(text).chars().take(BADGE_CHARS).collect()
}

/// Chrome's key token for gpui's key name, and back. The tokens the engine takes and
/// reports: "A".."Z", "0".."9", and the named keys below.
pub(crate) fn chrome_key(gpui_key: &str) -> Option<String> {
    key_pair(|(_, g)| *g == gpui_key)
        .map(|(c, _)| c.to_string())
        .or_else(|| one_alnum(gpui_key).map(|c| c.to_ascii_uppercase().to_string()))
}

pub(crate) fn gpui_key(chrome: &str) -> Option<String> {
    key_pair(|(c, _)| *c == chrome)
        .map(|(_, g)| g.to_string())
        .or_else(|| one_alnum(chrome).map(|c| c.to_ascii_lowercase().to_string()))
}

fn one_alnum(key: &str) -> Option<char> {
    let mut chars = key.chars();
    chars.next().filter(|c| c.is_ascii_alphanumeric() && chars.next().is_none())
}

fn key_pair(pick: impl Fn(&(&str, &str)) -> bool) -> Option<(&'static str, &'static str)> {
    const KEYS: [(&str, &str); 15] = [
        ("Comma", ","),
        ("Period", "."),
        ("Space", "space"),
        ("Home", "home"),
        ("End", "end"),
        ("PageUp", "pageup"),
        ("PageDown", "pagedown"),
        ("Insert", "insert"),
        ("Delete", "delete"),
        ("Up", "up"),
        ("Down", "down"),
        ("Left", "left"),
        ("Right", "right"),
        ("Tab", "tab"),
        ("Backspace", "backspace"),
    ];
    KEYS.into_iter().find(|k| pick(k))
}

/// The keystroke a command's shortcut is, `None` when it has none or gpui has no name for it.
pub(crate) fn command_keystroke(command: &baa_web::ExtensionCommand) -> Option<gpui::Keystroke> {
    let key = gpui_key(&command.key).filter(|_| !command.key.is_empty())?;
    Some(gpui::Keystroke { modifiers: command.modifiers, key, key_char: None })
}

/// The name of baa's own action bound to `keystroke` in the browser window (the default
/// keymap with the user's `keymap.json` over it), if any.
pub(crate) fn baa_binding(keystroke: &gpui::Keystroke, cx: &gpui::App) -> Option<String> {
    let context = gpui::KeyContext::parse("Browser").ok()?;
    let map = cx.key_bindings();
    let map = map.borrow();
    let (bindings, _) = map.bindings_for_input(std::slice::from_ref(keystroke), &[context]);
    let action = bindings.first()?.action().name();
    Some(crate::settings::shortcut_label(action).map_or_else(|| action.to_string(), str::to_string))
}

impl<H: WebHost> Browser<H> {
    /// Re-reads the active Space's actions as they are on its active tab.
    pub(crate) fn refresh_actions(&mut self, cx: &mut Context<Self>) {
        let Some(profile) = self.active_space().map(|s| s.profile) else {
            self.actions.clear();
            return;
        };
        let list = self.host.extension_actions(profile, self.active_page().map(|p| p.read(cx)));
        self.actions = list.into_iter().map(|a| (a.extension_id.clone(), a)).collect();
        cx.notify();
    }

    pub(super) fn on_action_changed(&mut self, change: &ExtensionActionChanged, cx: &mut Context<Self>) {
        let ours = self.active_space().is_some_and(|s| s.profile == change.profile);
        let page = self.active_page().map(|p| p.entity_id());
        if ours && change.page.is_none_or(|p| Some(p) == page) {
            self.refresh_actions(cx);
        }
    }

    /// An extension asked for its popup (`chrome.action.openPopup`). Taken only by the front
    /// window showing that profile's Space, and never over a prompt: one not taken is closed.
    pub(super) fn on_popup_opened(&mut self, opened: &ExtensionPopupOpened, window: &mut Window, cx: &mut Context<Self>) {
        // A second window on the same Space would open a popup of its own with `create_popup`.
        let ours = window.is_window_active() && self.active_space().is_some_and(|s| s.profile == opened.profile);
        if !ours || self.prompt_shown() {
            return;
        }
        let Some(ext) = self.extensions.iter().find(|e| e.id == opened.extension_id && e.popup.is_some()).cloned() else {
            return;
        };
        self.open_extension(&ext, window, cx);
    }

    /// A key baa's own keymap left alone: an enabled extension's "activate" shortcut opens
    /// its popup as a click would. One that is also baa's never fires.
    pub(super) fn on_extension_key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        if !(ks.modifiers.platform || ks.modifiers.control) || self.prompt_shown() {
            return;
        }
        let Some(profile) = self.active_space().map(|s| s.profile) else { return };
        let ext = self.host.extension_commands(profile).into_iter().find_map(|c| {
            let stroke = command_keystroke(&c).filter(|s| c.action && s.modifiers == ks.modifiers && s.key == ks.key)?;
            let ext = self.extensions.iter().find(|e| e.id == c.extension_id && self.extension_enabled(&e.id))?;
            baa_binding(&stroke, cx).is_none().then(|| ext.clone())
        });
        if let Some(ext) = ext {
            cx.stop_propagation();
            self.open_extension(&ext, window, cx);
        }
    }
}

#[cfg(test)]
#[test]
fn keys_map_both_ways_between_chrome_and_gpui() {
    for (chrome, gpui) in [("K", "k"), ("7", "7"), ("Comma", ","), ("Up", "up"), ("Space", "space"), ("PageDown", "pagedown")] {
        assert_eq!(gpui_key(chrome).as_deref(), Some(gpui), "{chrome}");
        assert_eq!(chrome_key(gpui).as_deref(), Some(chrome), "{gpui}");
    }
    for unknown in ["", "MediaPlayPause", "KK", "f13"] {
        assert_eq!(gpui_key(unknown), None, "{unknown}");
    }
    assert_eq!(chrome_key("escape"), None);
}

#[cfg(test)]
#[test]
fn badges_are_short_single_lines() {
    assert_eq!(badge_text("12345"), "1234");
    assert_eq!(badge_text("a\nb\u{202E}c"), "abc");
    assert_eq!(badge_text(""), "");
}
