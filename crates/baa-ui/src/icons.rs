//! Lucide icons (ISC, assets/icons/LICENSE), embedded so the bundle needs no
//! asset directory. `main` installs [`Assets`] on the application.

use std::borrow::Cow;

use gpui::{px, svg, AssetSource, Hsla, SharedString, Styled};

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        const ICONS: &[(&str, &[u8])] = &[$(
            (concat!("icons/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg"))),
        )*];
    };
}

icons!(
    "archive", "arrow-down-to-line", "arrow-left", "arrow-right", "bell", "book-open", "calendar",
    "camera", "check", "chevron-down", "circle-arrow-down", "clipboard", "code-xml", "disc", "file",
    "file-archive", "file-image", "file-music", "file-spreadsheet", "file-text",
    "file-video-camera", "folder", "folder-open", "globe", "history", "house", "import",
    "key-round", "languages", "link", "list-filter", "lock", "map-pin", "maximize-2", "mic",
    "panel-left", "picture-in-picture-2", "plus", "printer", "rotate-cw", "search", "settings",
    "sliders-horizontal", "trash-2", "triangle-alert", "volume-2", "volume-x", "x",
);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| (*p).into())
            .collect())
    }
}

/// `name` is the lucide name, e.g. "arrow-left".
pub fn icon(name: &'static str, size: f32, color: Hsla) -> gpui::Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_loads() {
        for (path, _) in ICONS {
            assert!(Assets.load(path).unwrap().is_some_and(|b| b.starts_with(b"<!--")), "{path}");
        }
        assert!(Assets.load("icons/nope.svg").unwrap().is_none());
    }
}
