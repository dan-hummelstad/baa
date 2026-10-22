//! §8 keymap: gpui actions plus a loader for GPUI/Zed-format `keymap.json`:
//! `[{"context": "Browser", "bindings": {"cmd-t": "baa::CommandBar", "cmd-1": ["baa::GoToTab", 0]}}]`.
//! Later bindings win, so a user keymap bound after the default overrides it.

use std::{collections::BTreeMap, rc::Rc};

use anyhow::Context as _;
use gpui::{App, KeyBinding, KeyBindingContextPredicate, NoAction};
use serde::Deserialize;

gpui::actions!(
    baa,
    [
        CommandBar,
        FocusAddress,
        ToggleSidebar,
        CloseTab,
        ReopenClosedTab,
        NextTab,
        PrevTab,
        LastTab,
        NextSpace,
        PrevSpace,
        TogglePin,
        Back,
        Forward,
        Reload,
        Find,
        DevTools,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        NewSpace,
        NewIncognitoSpace,
    ]
);
// ponytail: Peek, Little baa, split view and archive view (§8) are milestone 4/5; their
// actions are added with their views so the keymap never binds a key to nothing.

/// 0-based index into the active Space's tabs (pinned first, then Today).
#[derive(
    Clone, PartialEq, Debug, Deserialize, gpui::private::schemars::JsonSchema, gpui::Action,
)]
#[schemars(crate = "gpui::private::schemars")]
#[action(namespace = baa)]
pub struct GoToTab(pub usize);

/// 0-based index into the Space list.
#[derive(
    Clone, PartialEq, Debug, Deserialize, gpui::private::schemars::JsonSchema, gpui::Action,
)]
#[schemars(crate = "gpui::private::schemars")]
#[action(namespace = baa)]
pub struct GoToSpace(pub usize);

#[cfg(target_os = "macos")]
pub const DEFAULT_KEYMAP: &str = include_str!("../keymaps/macos.json");
#[cfg(not(target_os = "macos"))]
pub const DEFAULT_KEYMAP: &str = include_str!("../keymaps/windows.json");

#[derive(Deserialize)]
struct Section {
    context: Option<String>,
    bindings: BTreeMap<String, serde_json::Value>,
}

/// Parse and bind a keymap. A bad binding is logged and skipped so one typo in a
/// user keymap does not cost every other shortcut; a malformed file is an error.
pub fn bind_keymap(json: &str, cx: &mut App) -> anyhow::Result<()> {
    // ponytail: plain JSON; Zed's keymap files allow comments, ours do not.
    let sections: Vec<Section> = serde_json::from_str(json).context("keymap.json")?;
    let mut bindings = Vec::new();
    for section in sections {
        let predicate = match section
            .context
            .as_deref()
            .map(KeyBindingContextPredicate::parse)
        {
            Some(Ok(p)) => Some(Rc::new(p)),
            Some(Err(e)) => {
                log::warn!("keymap: bad context {:?}: {e}", section.context);
                continue;
            }
            None => None,
        };
        for (keys, value) in section.bindings {
            let action = match value {
                serde_json::Value::Null => Ok(Box::new(NoAction) as Box<dyn gpui::Action>),
                serde_json::Value::String(name) => {
                    cx.build_action(&name, None).map_err(anyhow::Error::from)
                }
                serde_json::Value::Array(mut v) if v.len() == 2 && v[0].is_string() => {
                    let args = v.pop();
                    cx.build_action(v[0].as_str().unwrap(), args)
                        .map_err(anyhow::Error::from)
                }
                other => Err(anyhow::anyhow!(
                    "expected \"name\" or [\"name\", args], got {other}"
                )),
            };
            let binding = action.and_then(|action| {
                KeyBinding::load(
                    &keys,
                    action,
                    predicate.clone(),
                    false,
                    None,
                    cx.keyboard_mapper().as_ref(),
                )
                .map_err(anyhow::Error::from)
            });
            match binding {
                Ok(b) => bindings.push(b),
                Err(e) => log::warn!("keymap: skipping {keys:?}: {e}"),
            }
        }
    }
    cx.bind_keys(bindings);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn default_keymaps_bind_every_entry(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            for json in [
                include_str!("../keymaps/macos.json"),
                include_str!("../keymaps/windows.json"),
            ] {
                let sections: Vec<Section> = serde_json::from_str(json).unwrap();
                let expected: usize = sections.iter().map(|s| s.bindings.len()).sum();
                let before = cx.key_bindings().borrow().bindings().count();
                bind_keymap(json, cx).unwrap();
                let after = cx.key_bindings().borrow().bindings().count();
                assert_eq!(after - before, expected, "a default binding failed to load");
            }
        });
    }

    #[gpui::test]
    fn bad_entries_are_skipped(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let before = cx.key_bindings().borrow().bindings().count();
            bind_keymap(
                r#"[{"bindings": {"cmd-t": "baa::CommandBar", "cmd-q": "baa::Nope"}}]"#,
                cx,
            )
            .unwrap();
            assert_eq!(cx.key_bindings().borrow().bindings().count() - before, 1);
            assert!(bind_keymap("not json", cx).is_err());
        });
    }
}
