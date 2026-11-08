//! GPUI views (PLAN.md §3, §8, §10). Codes only against `baa_web::WebHost`.

pub mod actions;
mod agent;
mod agent_panel;
mod browser;
mod command_bar;
mod fuzzy;
mod icons;
mod input;
mod library;
mod lil_baa;
mod motion;
mod pages;
mod prompts;
mod settings;
mod skin;
mod space_picker;
mod sidebar;
mod start;
mod theme;
mod tools;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_dialogs;
#[cfg(test)]
mod tests_m3_bar;
#[cfg(test)]
mod tests_settings;
#[cfg(test)]
mod tests_dev;
#[cfg(test)]
mod tests_agent;

pub use agent::AgentSwitch;
pub use browser::Browser;
pub use icons::Assets;
pub use settings::KeymapPath;

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

/// Binds the user's `keymap.json` over the defaults and remembers where it is, for the
/// settings window's "Edit keymap.json…". A missing file is fine.
pub fn load_user_keymap(path: std::path::PathBuf, cx: &mut gpui::App) {
    if let Ok(json) = std::fs::read_to_string(&path) {
        if let Err(e) = actions::bind_keymap(&json, cx) {
            log::error!("{e:#}");
        }
    }
    cx.set_global(KeymapPath(path));
}
