//! What the settings window has: its tabs and one entry per row. The window,
//! its search and the command bar all read this table, so they cannot drift
//! apart; the renderers take their labels from it too.

use baa_model::Permission;

use crate::fuzzy;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Tab {
    General,
    Appearance,
    Search,
    Privacy,
    Sites,
    Passwords,
    Extensions,
    Spaces,
    Keyboard,
    Developer,
    Claude,
    System,
}

impl Tab {
    pub(crate) const ALL: [Tab; 12] = [
        Tab::General,
        Tab::Appearance,
        Tab::Search,
        Tab::Privacy,
        Tab::Sites,
        Tab::Passwords,
        Tab::Extensions,
        Tab::Spaces,
        Tab::Keyboard,
        Tab::Developer,
        Tab::Claude,
        Tab::System,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Appearance => "Appearance",
            Tab::Search => "Search",
            Tab::Privacy => "Privacy",
            Tab::Sites => "Sites",
            Tab::Passwords => "Passwords",
            Tab::Extensions => "Extensions",
            Tab::Spaces => "Spaces",
            Tab::Keyboard => "Keyboard",
            Tab::Developer => "Developer",
            Tab::Claude => "Claude",
            Tab::System => "System",
        }
    }

    /// The longer name the window title and search results use.
    pub(crate) fn title(self) -> &'static str {
        match self {
            Tab::Privacy => "Privacy & Security",
            Tab::Sites => "Site Settings",
            other => other.label(),
        }
    }

    pub(crate) fn icon(self) -> &'static str {
        match self {
            Tab::General => "settings",
            Tab::Appearance => "panel-left",
            Tab::Search => "search",
            Tab::Privacy => "lock",
            Tab::Sites => "globe",
            Tab::Passwords => "key-round",
            Tab::Extensions => "puzzle",
            Tab::Spaces => "house",
            Tab::Keyboard => "code-xml",
            Tab::Developer => "terminal",
            Tab::Claude => "link",
            Tab::System => "sliders-horizontal",
        }
    }

    pub(crate) fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct Row {
    pub tab: Tab,
    pub id: &'static str,
    pub label: &'static str,
    pub keywords: &'static str,
}

const fn row(tab: Tab, id: &'static str, label: &'static str, keywords: &'static str) -> Row {
    Row { tab, id, label, keywords }
}

use Tab::*;

pub(crate) static ROWS: &[Row] = &[
    row(General, "default_browser", "Default browser", "make default http https"),
    row(General, "on_launch", "On launch", "startup restore tabs pinned"),
    row(General, "external_links", "Links from other apps", "little baa lil-baa external open"),
    row(General, "peek_links", "Links that open a new tab", "peek preview"),
    row(General, "auto_pip", "Picture in Picture when switching away", "pip video float automatic"),
    row(General, "archive", "Archive Today tabs after", "auto archive hours"),
    row(General, "download_dir", "Save downloads to", "downloads folder location"),
    row(General, "ask_save", "Ask where to save each file", "download prompt"),
    row(General, "languages", "Preferred languages", "language accept"),
    row(General, "spellcheck", "Spell check", "spelling"),
    row(General, "more_languages", "More language settings…", "translate fonts"),
    row(General, "import", "Import from Arc and other browsers", "arc history spaces"),
    row(Appearance, "appearance", "Appearance", "dark light mode theme baa system"),
    row(Appearance, "chrome", "Window style", "arc ascii terminal chrome"),
    row(Appearance, "side", "Sidebar side", "left right"),
    row(Appearance, "page_scheme", "Website appearance", "dark light mode theme prefers-color-scheme"),
    row(Appearance, "zoom", "Default page zoom", "magnify scale"),
    row(Appearance, "font_size", "Font size", "text small large"),
    row(Appearance, "min_font_size", "Minimum font size", "text tiny"),
    row(Appearance, "reduce_motion", "Reduce motion", "animation accessibility"),
    row(Appearance, "focus_highlight", "Highlight the focused element", "accessibility outline"),
    row(Appearance, "caret_browsing", "Caret browsing", "accessibility cursor"),
    row(Search, "engine", "Search engine", "google duckduckgo bing kagi brave default"),
    row(Search, "add_engine", "Add search engine", "custom url"),
    row(Search, "suggest", "Suggestions while typing", "autocomplete"),
    row(Privacy, "cookies", "Cookies", "third-party block tracking"),
    row(Privacy, "clear_on_quit", "Clear cookies and site data when you quit", "cookies session"),
    row(Privacy, "https", "HTTPS", "secure upgrade tls"),
    row(Privacy, "preload", "Preload pages", "prefetch prediction network"),
    row(Privacy, "dns", "Secure DNS", "doh cloudflare quad9 resolver"),
    row(Privacy, "dnt", "Send Do Not Track", "tracking"),
    row(Privacy, "gpc", "Global Privacy Control", "tracking"),
    row(Privacy, "clear_data", "Clear browsing data…", "history cache cookies delete"),
    row(Sites, "site.location", "Location", "geolocation gps"),
    row(Sites, "site.camera", "Camera", "video"),
    row(Sites, "site.microphone", "Microphone", "audio mic"),
    row(Sites, "site.notifications", "Notifications", "push alerts"),
    row(Sites, "site.clipboard", "Clipboard", "copy paste"),
    row(Sites, "site.midi", "MIDI devices", "sysex"),
    row(Sites, "site.popups", "Pop-ups and redirects", "popup window"),
    row(Sites, "site.sound", "Sound", "audio mute"),
    row(Sites, "site.javascript", "JavaScript", "scripts js"),
    row(Sites, "site.cookies", "Cookie exceptions", "site data allow block"),
    row(Sites, "site.downloads", "Automatic downloads", "multiple files"),
    row(Sites, "site.insecure", "Insecure content", "mixed http"),
    row(Sites, "site.sensors", "Motion sensors", "gyroscope accelerometer"),
    row(Sites, "site.windows", "Window management", "screens multi-screen"),
    row(Sites, "all_sites", "All sites and storage…", "site data usage"),
    row(Passwords, "pw_managers", "Password managers", "1password bitwarden extension passkeys"),
    row(Passwords, "pw_apple", "Apple Passwords", "icloud keychain"),
    row(Extensions, "dev_mode", "Developer mode", "unpacked extensions developer"),
    row(Extensions, "load_unpacked", "Load unpacked…", "extension folder developer"),
    row(Extensions, "ext_list", "Installed extensions", "add-ons plugins"),
    row(Spaces, "spaces", "Spaces", "rename colour color delete incognito"),
    row(Spaces, "shared_accounts", "Accounts in every Space", "share sign in login google microsoft github"),
    row(Keyboard, "shortcuts", "Keyboard shortcuts", "keys bindings"),
    row(Keyboard, "keymap", "Edit keymap.json…", "rebind customise"),
    row(Developer, "devtools", "Developer tools", "inspect dock devtools"),
    row(Developer, "chrome_pages", "Chrome pages", "flags about gpu net-internals inspect"),
    row(Developer, "environments", "Environments", "local staging production prod origin project env"),
    row(Developer, "dev_servers", "Dev servers folder", "localhost ports node vite live"),
    row(Developer, "console_badge", "Console errors on localhost tabs", "badge console error devtools localhost"),
    row(Claude, "claude_enabled", "Let Claude use baa", "mcp agent ai assistant"),
    row(Claude, "claude_spaces", "Spaces Claude can see", "allowed incognito"),
    row(Claude, "claude_evaluate", "Allow evaluate", "javascript run script"),
    row(Claude, "claude_path", "claude path", "binary cli install"),
    row(Claude, "claude_terminal", "Use from Claude Code in Terminal", "mcp add command copy"),
    row(System, "auto_update", "Automatic updates", "update version new release about"),
    row(System, "hw_accel", "Graphics acceleration", "gpu hardware"),
    row(System, "memory_saver", "Memory saver", "discard tabs performance"),
    row(System, "reset", "Reset settings…", "defaults restore"),
];

/// Looks up a row; every id the renderers use is in [`ROWS`], and a test checks it.
pub(crate) fn find(id: &str) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.id == id)
}

pub(crate) fn label(id: &str) -> &'static str {
    find(id).map_or("", |r| r.label)
}

/// How well `query` matches the row's label (counted double) or one of its keywords.
pub(crate) fn score(query: &str, row: &Row) -> Option<u32> {
    let by_label = fuzzy::score(query, row.label).map(|s| s * 2);
    let by_word = row.keywords.split(' ').filter_map(|w| fuzzy::score(query, w)).max();
    by_label.max(by_word)
}

/// Best matches for `query` in label and keywords, label hits first.
pub(crate) fn search(query: &str) -> Vec<&'static Row> {
    let mut hits: Vec<(u32, &Row)> = ROWS.iter().filter_map(|r| score(query, r).map(|s| (s, r))).collect();
    hits.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    hits.into_iter().map(|(_, r)| r).collect()
}

/// One site setting: the row it has here and the name Chrome's settings give it in a URL.
pub(crate) struct SiteKind {
    pub kind: Permission,
    pub row: &'static str,
    /// `baa://settings/content/<path>`
    pub path: &'static str,
}

const fn site(kind: Permission, row: &'static str, path: &'static str) -> SiteKind {
    SiteKind { kind, row, path }
}

/// The only place that pairs a site setting with its row and address.
pub(crate) static SITE_KINDS: [SiteKind; 14] = [
    site(Permission::Location, "site.location", "location"),
    site(Permission::Camera, "site.camera", "camera"),
    site(Permission::Microphone, "site.microphone", "microphone"),
    site(Permission::Notifications, "site.notifications", "notifications"),
    site(Permission::Clipboard, "site.clipboard", "clipboard"),
    site(Permission::Midi, "site.midi", "midiDevices"),
    site(Permission::Popups, "site.popups", "popups"),
    site(Permission::Sound, "site.sound", "sound"),
    site(Permission::JavaScript, "site.javascript", "javascript"),
    site(Permission::Cookies, "site.cookies", "cookies"),
    site(Permission::AutomaticDownloads, "site.downloads", "automaticDownloads"),
    site(Permission::InsecureContent, "site.insecure", "insecureContent"),
    site(Permission::Sensors, "site.sensors", "sensors"),
    site(Permission::WindowManagement, "site.windows", "windowManagement"),
];

/// The site-settings row of `kind`.
pub(crate) fn site_row(kind: Permission) -> &'static str {
    SITE_KINDS.iter().find(|k| k.kind == kind).map_or("", |k| k.row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_every_site_kind_has_a_row() {
        for (i, r) in ROWS.iter().enumerate() {
            assert!(ROWS[i + 1..].iter().all(|o| o.id != r.id), "{}", r.id);
        }
        for kind in Permission::SITE_KINDS {
            assert_eq!(find(site_row(kind)).map(|r| r.tab), Some(Sites), "{kind:?}");
        }
    }

    #[test]
    fn search_ranks_the_label_first() {
        assert_eq!(search("zoom")[0].id, "zoom");
        assert_eq!(search("cookies")[0].id, "cookies");
        assert!(search("zzzz").is_empty());
    }
}
