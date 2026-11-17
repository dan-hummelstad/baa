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

/// The badge's text colour: the extension's, else black or white, whichever reads on `badge`.
pub(crate) fn badge_ink(badge: gpui::Hsla, text_rgba: Option<u32>) -> gpui::Hsla {
    let c = badge.to_rgb();
    let light = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b > 0.5;
    text_rgba.map_or(if light { gpui::black() } else { gpui::white() }, |c| gpui::rgba(c).into())
}

/// Chrome's key token and modifiers for a gpui keystroke, and back. The tokens the engine
/// takes and reports: "A".."Z", "0".."9", and the named keys below.
pub(crate) fn chrome_key(keystroke: &gpui::Keystroke) -> Option<(String, gpui::Modifiers)> {
    if let Some((chrome, _)) = SHIFTED.into_iter().find(|(_, g)| *g == keystroke.key) {
        return Some((chrome.to_string(), gpui::Modifiers { shift: true, ..keystroke.modifiers }));
    }
    let key = key_pair(|(_, g)| *g == keystroke.key)
        .map(|(c, _)| c.to_string())
        .or_else(|| one_alnum(&keystroke.key).map(|c| c.to_ascii_uppercase().to_string()))?;
    Some((key, keystroke.modifiers))
}

pub(crate) fn gpui_keystroke(chrome: &str, modifiers: gpui::Modifiers) -> Option<gpui::Keystroke> {
    if let Some((_, shifted)) = SHIFTED.into_iter().find(|(c, _)| modifiers.shift && *c == chrome) {
        let modifiers = gpui::Modifiers { shift: false, ..modifiers };
        return Some(gpui::Keystroke { modifiers, key: shifted.to_string(), key_char: None });
    }
    let key = key_pair(|(c, _)| *c == chrome)
        .map(|(_, g)| g.to_string())
        .or_else(|| one_alnum(chrome).map(|c| c.to_ascii_lowercase().to_string()))?;
    Some(gpui::Keystroke { modifiers, key, key_char: None })
}

// ponytail: gpui folds Shift into the key for anything but a..z, so ⌘⇧1 arrives as ⌘"!".
// US layout only: on others these shortcuts never fire and cannot be recorded.
const SHIFTED: [(&str, &str); 12] = [
    ("1", "!"),
    ("2", "@"),
    ("3", "#"),
    ("4", "$"),
    ("5", "%"),
    ("6", "^"),
    ("7", "&"),
    ("8", "*"),
    ("9", "("),
    ("0", ")"),
    ("Comma", "<"),
    ("Period", ">"),
];

fn one_alnum(key: &str) -> Option<char> {
    let mut chars = key.chars();
    chars.next().filter(|c| c.is_ascii_alphanumeric() && chars.next().is_none())
}

fn key_pair(pick: impl Fn(&(&str, &str)) -> bool) -> Option<(&'static str, &'static str)> {
    const KEYS: [(&str, &str); 14] = [
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
    ];
    KEYS.into_iter().find(|k| pick(k))
}

/// The keystroke a command's shortcut is, `None` when it has none or gpui has no name for it.
pub(crate) fn command_keystroke(command: &baa_web::ExtensionCommand) -> Option<gpui::Keystroke> {
    gpui_keystroke(&command.key, command.modifiers)
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
    /// Re-reads the active Space's extensions and their actions.
    pub(crate) fn refresh_extensions(&mut self, cx: &mut Context<Self>) {
        self.extensions = self.active_space().map(|s| self.host.extensions(s.profile)).unwrap_or_default();
        self.refresh_actions(cx);
    }

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
        self.click_action(&opened.extension_id, window, cx);
    }

    /// As a click on `id`'s action: the engine shows the popup it has for the active tab
    /// (which `chrome.action.setPopup` may have set) or runs the extension's `onClicked`.
    fn click_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self.session.active_space() else { return };
        let popup = self.extensions.iter().find(|e| e.id == id).and_then(|e| e.popup.clone());
        // The host reads only the id from the URL.
        let url = popup.unwrap_or_else(|| format!("chrome-extension://{id}/"));
        self.open_preview(space, &url, true, window, cx);
    }

    /// A key baa's own keymap left alone: an enabled extension's "activate" shortcut is a
    /// click on its action. One that is also baa's never fires.
    pub(super) fn on_extension_key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        if event.is_held || !(ks.modifiers.platform || ks.modifiers.control) || self.prompt_shown() {
            return;
        }
        let Some(profile) = self.active_space().map(|s| s.profile) else { return };
        let id = self.host.extension_commands(profile).into_iter().find_map(|c| {
            let stroke = command_keystroke(&c).filter(|s| c.action && s.modifiers == ks.modifiers && s.key == ks.key)?;
            // The engine lists only enabled extensions' actions.
            (self.actions.contains_key(&c.extension_id) && baa_binding(&stroke, cx).is_none()).then_some(c.extension_id)
        });
        if let Some(id) = id {
            cx.stop_propagation();
            self.click_action(&id, window, cx);
        }
    }
}

#[cfg(test)]
#[test]
fn keys_map_both_ways_between_chrome_and_gpui() {
    let ks = |s: &str| gpui::Keystroke::parse(s).unwrap();
    for (chrome, shift, gpui) in [
        ("K", false, "cmd-k"),
        ("K", true, "cmd-shift-k"),
        ("7", false, "cmd-7"),
        ("1", true, "cmd-!"),
        ("0", true, "ctrl-)"),
        ("Comma", false, "cmd-,"),
        ("Comma", true, "cmd-<"),
        ("Period", true, "cmd->"),
        ("Up", true, "cmd-shift-up"),
        ("Space", false, "cmd-space"),
        ("PageDown", false, "cmd-pagedown"),
    ] {
        let stroke = ks(gpui);
        let modifiers = gpui::Modifiers { shift, ..stroke.modifiers };
        assert_eq!(gpui_keystroke(chrome, modifiers), Some(stroke.clone()), "{chrome}");
        assert_eq!(chrome_key(&stroke), Some((chrome.to_string(), modifiers)), "{gpui}");
    }
    for unknown in ["", "MediaPlayPause", "KK", "f13", "Backspace"] {
        assert_eq!(gpui_keystroke(unknown, gpui::Modifiers::command()), None, "{unknown}");
    }
    assert_eq!(chrome_key(&ks("cmd-escape")), None);
    assert_eq!(chrome_key(&ks("cmd-backspace")), None);
}

#[cfg(test)]
#[test]
fn badges_are_short_single_lines() {
    assert_eq!(badge_text("12345"), "1234");
    assert_eq!(badge_text("a\nb\u{202E}c"), "abc");
    assert_eq!(badge_text(""), "");
}
