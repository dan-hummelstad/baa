//! Shortcuts in the bar: their chips, and every keymap binding offered by name.

use gpui::{Action, App, FocusHandle, SharedString, Window};

use super::COMMANDS;
use crate::actions;

/// A keymap binding the bar offers by name.
pub(crate) struct Keyed {
    pub label: String,
    pub action: Box<dyn Action>,
    pub keys: Vec<SharedString>,
}

/// Every action the keymap binds that runs at `target` (where the bar gives focus back), but
/// those a command or Space row already offers. Later bindings win, as in the keymap: a
/// keystroke's chip goes only to the action it runs there (in mail ⌘R is Reply, not Reload).
pub(crate) fn keyed(window: &Window, target: &FocusHandle, cx: &App) -> Vec<Keyed> {
    let offered: Vec<Box<dyn Action>> = COMMANDS.iter().filter_map(|(c, _)| c.action()).collect();
    let mut out: Vec<Keyed> = Vec::new();
    let mut taken: Vec<Vec<SharedString>> = Vec::new();
    for binding in cx.key_bindings().borrow().bindings().rev() {
        let action = binding.action();
        let any = action.as_any();
        if any.is::<actions::CommandBar>() || any.is::<actions::GoToSpace>() || any.is::<gpui::NoAction>() {
            continue;
        }
        if !window.is_action_available_in(action, target) {
            continue;
        }
        let keys = chips(binding);
        let free = !taken.contains(&keys);
        taken.push(keys.clone());
        if offered.iter().any(|a| a.partial_eq(action)) {
            continue;
        }
        match out.iter_mut().find(|k| k.action.partial_eq(action)) {
            Some(k) if k.keys.is_empty() && free => k.keys = keys,
            Some(_) => {}
            None => out.push(Keyed { label: action_label(action), action: action.boxed_clone(), keys: if free { keys } else { Vec::new() } }),
        }
    }
    out.reverse();
    out
}

/// `baa::ToggleSidebar` → "Toggle Sidebar".
fn action_label(action: &dyn Action) -> String {
    if let Some(actions::GoToTab(n)) = action.as_any().downcast_ref() {
        return format!("Go to Tab {}", n + 1);
    }
    let name = action.name().rsplit("::").next().unwrap_or_default();
    match name {
        "ToggleLibrary" => return "Open or Close the Fold".into(),
        "ToggleAgent" => return "Chat".into(),
        "FocusAddress" => return "Edit Address".into(),
        _ => {}
    }
    let mut label = String::new();
    for c in name.chars() {
        if c.is_uppercase() && !label.is_empty() {
            label.push(' ');
        }
        label.push(c);
    }
    label
}

/// A shortcut as the mock draws it: one chip per modifier, then the key.
pub(crate) fn chips(binding: &gpui::KeyBinding) -> Vec<SharedString> {
    binding.keystrokes().iter().flat_map(|s| stroke_chips(s.inner())).collect()
}

pub(crate) fn stroke_chips(k: &gpui::Keystroke) -> Vec<SharedString> {
    let m = &k.modifiers;
    let mac = cfg!(target_os = "macos");
    let mut out: Vec<SharedString> = [
        (m.control, "⌃", "Ctrl"),
        (m.alt, "⌥", "Alt"),
        (m.shift, "⇧", "Shift"),
        (m.platform, "⌘", "Win"),
    ]
    .into_iter()
    .filter(|(on, ..)| *on)
    .map(|(_, mac_label, other)| if mac { mac_label } else { other }.into())
    .collect();
    let key = match k.key.as_str() {
        "enter" => "↩".to_string(),
        "left" => "←".into(),
        "right" => "→".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "tab" => "⇥".into(),
        "escape" => "esc".into(),
        "backspace" => "⌫".into(),
        other => other.to_uppercase(),
    };
    out.push(key.into());
    out
}
