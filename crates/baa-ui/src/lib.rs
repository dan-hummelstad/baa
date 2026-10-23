//! GPUI views (PLAN.md §3, §8, §10). Codes only against `baa_web::WebHost`.

pub mod actions;
mod browser;
mod command_bar;
mod extensions;
mod fuzzy;
mod icons;
mod input;
mod library;
mod lil_baa;
mod motion;
mod prefs;
mod prompts;
mod sidebar;
mod start;
mod theme;
mod tools;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

pub use browser::Browser;
pub use icons::Assets;

/// Registers the platform default keymap (§8). Call once at startup, then
/// `actions::bind_keymap` with the user's `keymap.json` so it overrides.
pub fn init(cx: &mut gpui::App) {
    // ASCII Chrome's face; bundled so the look does not depend on what is installed (OFL, assets/fonts).
    cx.text_system()
        .add_fonts(vec![
            include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf").as_slice().into(),
            include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf").as_slice().into(),
        ])
        .expect("bundled fonts load");
    actions::bind_keymap(actions::DEFAULT_KEYMAP, cx).expect("bundled keymap is valid JSON");
}
