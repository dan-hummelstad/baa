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
    "alarm-clock", "archive", "arrow-down-to-line", "arrow-left", "arrow-right", "arrow-up", "at-sign", "ban", "bell", "book-open", "calendar",
    "camera", "check", "chevron-down", "chevron-left", "chevron-right", "chevron-up", "chevrons-left-right", "chevrons-right", "chevrons-up-down", "circle-arrow-down", "circle-dot", "clipboard", "code-xml", "columns-2", "disc", "download", "ellipsis", "file",
    "file-archive", "file-image", "file-music", "file-pen", "file-spreadsheet", "file-text",
    "file-video-camera", "flag", "folder", "folder-open", "git-pull-request", "globe", "history", "hourglass", "house", "import", "inbox",
    "key-round", "languages", "link", "list-filter", "lock", "mail", "mail-open", "map-pin", "maximize-2", "megaphone", "message-circle", "mic", "minus", "moon-star",
    "panel-left", "paperclip", "pause", "pencil-line", "picture-in-picture-2", "pin", "pin-off", "play", "plus", "printer", "puzzle", "rotate-cw", "rss", "search", "send", "settings",
    "skip-forward", "sliders-horizontal", "sparkles", "square", "sun", "terminal", "trash-2", "triangle-alert", "user", "users", "volume-2", "volume-x", "x",
);

/// baa's own mark ("The leap, solid" in baa.pen, also the app icon: assets/baa.icns), for `img(MARK)`.
pub const MARK: &str = "baa.png";
/// uBlock Origin's own icon (GPLv3, from chromium/vendor): as a component extension it names
/// one that lives only in Chromium's resource pak, so the toolbar had no file to show.
const UBLOCK: &str = "ublock.png";
const IMAGES: &[(&str, &[u8])] = &[(MARK, include_bytes!("../assets/baa.png")), (UBLOCK, include_bytes!("../assets/ublock.png"))];

/// An extension's icon at `size`: its file, uBlock's bundled one, or a puzzle piece.
pub fn extension_icon(ext: &baa_web::Extension, size: f32, fallback: Hsla) -> gpui::AnyElement {
    use gpui::{img, IntoElement};
    match &ext.icon {
        Some(path) if path.exists() => img(path.clone()).size(px(size)).into_any_element(),
        _ if ext.name.starts_with("uBlock") => img(UBLOCK).size(px(size)).into_any_element(),
        _ => icon("puzzle", size, fallback).into_any_element(),
    }
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .chain(IMAGES)
            .find(|(p, _)| *p == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .chain(IMAGES)
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
