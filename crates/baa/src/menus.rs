//! The menu bar, laid out as Arc's: baa, File, Edit, View, Spaces, Tabs,
//! Archive, Extensions, Window, Help. Shortcuts shown are the keymap's, so a
//! user keymap.json moves them too.

use baa_ui::actions::*;
use gpui::{actions, App, Menu, MenuItem, OsAction, SystemMenuType};

use crate::Quit;

actions!(baa_app, [About, Hide, HideOthers, ShowAll, Minimize, Zoom, EditOs]);

pub fn set(cx: &mut App) {
    cx.on_action(|_: &About, _| show_about_panel());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| on_window(cx, gpui::Window::minimize_window));
    cx.on_action(|_: &Zoom, cx| on_window(cx, gpui::Window::zoom_window));
    cx.bind_keys([
        gpui::KeyBinding::new("cmd-h", Hide, None),
        gpui::KeyBinding::new("cmd-alt-h", HideOthers, None),
        gpui::KeyBinding::new("cmd-m", Minimize, None),
    ]);
    cx.set_menus(menus());
}

fn show_about_panel() {
    // SAFETY: on the main thread (an action); the pointer is used within this call only.
    unsafe {
        use objc2::{class, msg_send, runtime::AnyObject};
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if !app.is_null() {
            let _: () = msg_send![app, orderFrontStandardAboutPanel: std::ptr::null_mut::<AnyObject>()];
        }
    }
}

fn on_window(cx: &mut App, f: fn(&gpui::Window)) {
    if let Some(w) = cx.active_window() {
        w.update(cx, |_, window, _| f(window)).ok();
    }
}

fn menus() -> Vec<Menu> {
    use MenuItem::Separator as Sep;
    fn a(name: &'static str, action: impl gpui::Action) -> MenuItem {
        MenuItem::action(name, action)
    }
    // ponytail: the Edit items carry no ⌘C/⌘V glyph. A key equivalent would let
    // the menu take ⌘V before baa's text fields see it; the page and the fields
    // already handle the keys, and a click still reaches the page via the
    // responder chain. Undo/Redo are gpui actions (gpui sends no `undo:`) that only
    // baa's focused text fields handle, so on a page they are disabled and the page's
    // own undo stays on ⌘Z.
    // `EditOs` is never bound, so no glyph; with no native view first, nothing handles it.
    // Full Screen is not listed: AppKit adds it to a menu titled View.
    let edit = |name: &str, os| MenuItem::os_action(name.to_string(), EditOs, os);
    vec![
        Menu::new("baa").items([
            a("About baa", About),
            Sep,
            a("Settings…", OpenSettings),
            Sep,
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            Sep,
            a("Hide baa", Hide),
            a("Hide Others", HideOthers),
            a("Show All", ShowAll),
            Sep,
            a("Quit baa", Quit),
        ]),
        Menu::new("File").items([
            a("New Tab", CommandBar),
            a("New Window", NewWindow),
            a("New Space", NewSpace),
            a("New Incognito Window", NewIncognitoWindow),
            Sep,
            a("Open Location…", FocusAddress),
            a("Reopen Closed Tab", ReopenClosedTab),
            a("Close Tab", CloseTab),
            a("Close Window", CloseWindow),
            Sep,
            a("Print…", Print),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            Sep,
            edit("Cut", OsAction::Cut),
            edit("Copy", OsAction::Copy),
            edit("Paste", OsAction::Paste),
            edit("Select All", OsAction::SelectAll),
            Sep,
            a("Find…", Find),
            a("Copy URL", CopyUrl),
        ]),
        Menu::new("View").items([
            a("Toggle Sidebar", ToggleSidebar),
            a("Library", ToggleLibrary),
            a("Agent", ToggleAgent),
            Sep,
            a("Reload Page", Reload),
            a("Force Reload Page", HardReload),
            a("Reader Mode", ReaderMode),
            Sep,
            a("Zoom In", ZoomIn),
            a("Zoom Out", ZoomOut),
            a("Actual Size", ZoomReset),
            Sep,
            MenuItem::submenu(Menu::new("Developer").items([
                a("Developer Tools", DevTools),
                a("View Source", ViewSource),
                a("Responsive View", ResponsiveView),
                a("Switch Environment", CycleEnvironment),
            ])),
        ]),
        Menu::new("Spaces").items([
            a("Next Space", NextSpace),
            a("Previous Space", PrevSpace),
            Sep,
            a("New Space", NewSpace),
            a("New Incognito Window", NewIncognitoWindow),
        ]),
        Menu::new("Tabs").items([
            a("Next Tab", NextTab),
            a("Previous Tab", PrevTab),
            a("Last Tab", LastTab),
            Sep,
            a("Pin / Unpin Tab", TogglePin),
            a("Mute Tab", MuteTab),
            a("Split View", SplitView),
            a("Picture in Picture", PictureInPicture),
            Sep,
            a("Clear Today", ClearToday),
        ]),
        Menu::new("The Fold").items([
            a("Show The Fold", ShowArchive),
            a("History", ShowHistory),
            a("Downloads", ShowDownloads),
            Sep,
            a("Reopen Closed Tab", ReopenClosedTab),
        ]),
        Menu::new("Extensions").items([a("Manage Extensions", ManageExtensions)]),
        Menu::new("Window").items([
            a("Minimize", Minimize),
            a("Zoom", Zoom),
        ]),
        Menu::new("Help").items([a("Keyboard Shortcuts", KeyboardShortcuts), a("Report a Problem…", ReportProblem)]),
    ]
}
