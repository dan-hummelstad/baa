//! Native context menus (NSMenu) for the app's own UI.
//!
//! A menu runs its own nested event loop until it closes, so it is never
//! opened from inside a gpui update: gpui's state would stay borrowed while
//! its callbacks keep running under it. [`popup_soon`] opens it on a later run
//! loop turn instead.

use std::{
    cell::{Cell, RefCell},
    ffi::c_int,
    rc::Rc,
};

use gpui::{App, Pixels, Window};
use objc2::{
    class, msg_send,
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel},
    sel,
};

use super::{
    ffi, from_c,
    native::{self, CGPoint, CGRect},
    PageKind, Shared,
};
use crate::{MenuDone, MenuItem};

thread_local! {
    /// The tag of the item picked in the menu that is open, if any.
    static PICKED: Cell<Option<isize>> = const { Cell::new(None) };
    static PENDING: RefCell<Vec<Box<dyn FnOnce()>>> = const { RefCell::new(Vec::new()) };
}

/// Runs `run` on a later run loop turn, with nothing borrowed.
fn popup_soon(run: impl FnOnce() + 'static) {
    use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop, CFRunLoopTimer, CFRunLoopTimerContext};
    PENDING.with(|p| p.borrow_mut().push(Box::new(run)));
    extern "C" fn fire(_timer: core_foundation::runloop::CFRunLoopTimerRef, _info: *mut std::ffi::c_void) {
        for run in PENDING.with(|p| std::mem::take(&mut *p.borrow_mut())) {
            run();
        }
    }
    let mut context = CFRunLoopTimerContext {
        version: 0,
        info: std::ptr::null_mut(),
        retain: None,
        release: None,
        copyDescription: None,
    };
    let now = unsafe { core_foundation::date::CFAbsoluteTimeGetCurrent() };
    let timer = CFRunLoopTimer::new(now, 0., 0, 0, fire, &mut context);
    unsafe { CFRunLoop::get_main().add_timer(&timer, kCFRunLoopCommonModes) };
}

/// The object every item's action goes to; it records the item's tag.
fn target() -> *mut AnyObject {
    thread_local! {
        static TARGET: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) };
    }
    TARGET.with(|t| {
        if t.get().is_null() {
            unsafe extern "C-unwind" fn pick(_this: *mut AnyObject, _cmd: Sel, item: *mut AnyObject) {
                let tag: isize = unsafe { msg_send![item, tag] };
                PICKED.with(|p| p.set(Some(tag)));
            }
            let class: &AnyClass = match ClassBuilder::new(c"BaaMenuTarget", class!(NSObject)) {
                Some(mut builder) => {
                    unsafe {
                        builder.add_method(
                            sel!(pick:),
                            pick as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
                        )
                    };
                    builder.register()
                }
                None => AnyClass::get(c"BaaMenuTarget").expect("registered"),
            };
            let object: *mut AnyObject = unsafe { msg_send![class, new] };
            t.set(object);
        }
        t.get()
    })
}

fn ns_string(text: &str) -> *mut AnyObject {
    let text = std::ffi::CString::new(text.replace('\0', "")).unwrap_or_default();
    unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
}

/// An NSMenu for `items`; picked items report their index in `tags`.
fn build(items: &[MenuItem], tags: &mut Vec<usize>) -> *mut AnyObject {
    unsafe {
        let menu: *mut AnyObject = msg_send![class!(NSMenu), alloc];
        let menu: *mut AnyObject = msg_send![menu, initWithTitle: ns_string("")];
        let _: () = msg_send![menu, setAutoenablesItems: Bool::NO];
        for item in items {
            let entry: *mut AnyObject = match item {
                MenuItem::Separator => msg_send![class!(NSMenuItem), separatorItem],
                MenuItem::Action { label, enabled, checked, id } => {
                    let entry: *mut AnyObject = msg_send![class!(NSMenuItem), alloc];
                    let entry: *mut AnyObject = msg_send![entry, initWithTitle: ns_string(label), action: sel!(pick:), keyEquivalent: ns_string("")];
                    let _: () = msg_send![entry, setTarget: target()];
                    let _: () = msg_send![entry, setTag: tags.len() as isize];
                    tags.push(*id);
                    let _: () = msg_send![entry, setEnabled: Bool::new(*enabled)];
                    let _: () = msg_send![entry, setState: *checked as isize];
                    let _: *mut AnyObject = msg_send![entry, autorelease];
                    entry
                }
                MenuItem::Submenu { label, items } => {
                    let entry: *mut AnyObject = msg_send![class!(NSMenuItem), alloc];
                    let entry: *mut AnyObject = msg_send![entry, initWithTitle: ns_string(label), action: None::<Sel>, keyEquivalent: ns_string("")];
                    let sub = build(items, tags);
                    let _: () = msg_send![entry, setSubmenu: sub];
                    let _: *mut AnyObject = msg_send![entry, autorelease];
                    entry
                }
            };
            let _: () = msg_send![menu, addItem: entry];
        }
        let _: *mut AnyObject = msg_send![menu, autorelease];
        menu
    }
}

/// Shows `items` at `(x, y)` (window coordinates, from the top left) over
/// `view`, until the menu closes; the picked item's id.
fn popup(view: *mut AnyObject, x: f32, y: f32, items: &[MenuItem]) -> Option<usize> {
    if view.is_null() || items.is_empty() {
        return None;
    }
    let mut tags = Vec::new();
    let menu = build(items, &mut tags);
    PICKED.with(|p| p.set(None));
    unsafe {
        let flipped: Bool = msg_send![view, isFlipped];
        let bounds: CGRect = msg_send![view, bounds];
        let at = CGPoint { x: x as f64, y: if flipped.as_bool() { y as f64 } else { bounds.size.height - y as f64 } };
        let _: Bool = msg_send![menu, popUpMenuPositioningItem: std::ptr::null_mut::<AnyObject>(), atLocation: at, inView: view];
    }
    PICKED.with(|p| p.take()).and_then(|tag| tags.get(tag as usize).copied())
}

/// Opens `items` at `at` (window coordinates) on a later run loop turn;
/// `done` gets the picked item's id.
pub fn show_menu(window: &mut Window, cx: &mut App, at: gpui::Point<Pixels>, items: Vec<MenuItem>, done: MenuDone) {
    let view = native::gpui_view(window);
    let handle = window.window_handle();
    let mut cx = cx.to_async();
    popup_soon(move || {
        let picked = popup(view, f32::from(at.x), f32::from(at.y), &items);
        let _ = gpui::AppContext::update_window(&mut cx, handle, |_, window, cx| done(picked, window, cx));
    });
}

/// Opens `items` over the page view (`view` is asked for when the menu is
/// about to open: the page may be gone by then) at (`x`, `y`) in it; `done`
/// gets the picked item's id. Later run loop turn, never inside a callback.
pub fn show_at(
    view: impl FnOnce() -> *mut AnyObject + 'static,
    x: f32,
    y: f32,
    items: Vec<MenuItem>,
    done: impl FnOnce(Option<usize>) + 'static,
) {
    popup_soon(move || done(popup(view(), x, y, &items)));
}

/// One row of a context menu, as Chrome flattened it.
pub(super) struct Row {
    pub(super) label: String,
    pub(super) command: c_int,
    pub(super) depth: u8,
    pub(super) kind: u8,
    pub(super) enabled: bool,
    pub(super) checked: bool,
}

impl Row {
    pub(super) fn from_c(items: *const ffi::MenuItem, count: usize) -> Vec<Row> {
        if items.is_null() {
            return Vec::new();
        }
        unsafe { std::slice::from_raw_parts(items, count) }
            .iter()
            .map(|item| Row {
                label: unsafe { from_c(item.label) },
                command: item.command,
                depth: item.depth,
                kind: item.kind,
                enabled: item.enabled,
                checked: item.checked,
            })
            .collect()
    }
}

/// The rows as a menu tree; an action's id is its row index.
pub(super) fn menu_tree(rows: &[Row]) -> Vec<MenuItem> {
    fn level(rows: &[Row], at: &mut usize, depth: u8) -> Vec<MenuItem> {
        let mut items = Vec::new();
        while let Some(row) = rows.get(*at).filter(|row| row.depth == depth) {
            let index = *at;
            *at += 1;
            items.push(match row.kind {
                ffi::MENU_SEPARATOR => MenuItem::Separator,
                ffi::MENU_SUBMENU => MenuItem::Submenu { label: row.label.clone(), items: level(rows, at, depth + 1) },
                _ => MenuItem::Action { label: row.label.clone(), enabled: row.enabled, checked: row.checked, id: index },
            });
        }
        items
    }
    let mut at = 0;
    let mut items = level(rows, &mut at, 0);
    // Rows that skip a depth (a malformed list) end the menu rather than loop.
    while at < rows.len() && rows[at].depth == 0 {
        items.extend(level(rows, &mut at, 0));
    }
    items
}

/// What to tell Chrome once the menu closed, and whether baa's own DevTools
/// should open: Chrome's Inspect would open its window against our hidden one,
/// so on a tab it is not run.
pub(super) fn menu_answer(rows: &[Row], picked: Option<usize>, kind: PageKind) -> (c_int, bool) {
    match picked.and_then(|index| rows.get(index)) {
        Some(row) if row.kind == ffi::MENU_INSPECT && kind == PageKind::Tab => (-1, true),
        Some(row) => (row.command, false),
        None => (-1, false),
    }
}

/// Opens Chrome's menu over `shared`'s view and tells Chrome the outcome once;
/// `on_inspect` runs when baa's own DevTools were asked for.
pub(super) fn present_menu(shared: Rc<Shared>, id: u64, x: f32, y: f32, rows: Vec<Row>, kind: PageKind, on_inspect: impl FnOnce() + 'static) {
    let core = shared.core;
    let items = menu_tree(&rows);
    show_at(move || shared.view.get(), x, y, items, move |picked| {
        let (command, inspect) = menu_answer(&rows, picked, kind);
        (core.context_menu_done)(id, command);
        if inspect {
            on_inspect();
        }
    });
}
