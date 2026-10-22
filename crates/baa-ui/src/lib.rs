//! GPUI views (PLAN.md §3, §8, §10). Codes only against `baa_web::WebHost`.

pub mod actions;
mod browser;
mod command_bar;
mod fuzzy;
mod input;
mod motion;
mod sidebar;
mod theme;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

pub use browser::Browser;

/// Registers the platform default keymap (§8). Call once at startup, then
/// `actions::bind_keymap` with the user's `keymap.json` so it overrides.
pub fn init(cx: &mut gpui::App) {
    actions::bind_keymap(actions::DEFAULT_KEYMAP, cx).expect("bundled keymap is valid JSON");
}
