//! Native context menus (NSMenu), for pages and for the app's own UI.
//!
//! A menu runs its own nested event loop until it closes, so it is never
//! opened from inside a gpui update or a CEF callback: gpui's state would stay
//! borrowed while the CEF pump and gpui's own callbacks keep running under it.
//! [`popup_soon`] opens it on a later run loop turn instead.

use std::cell::{Cell, RefCell};

use cef::{ImplMenuModel as _, ImplRunContextMenuCallback as _, MenuItemType, MenuModel, RunContextMenuCallback};
use objc2::{
    class, msg_send,
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel},
    sel,
};

use crate::MenuItem;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGPoint,
}

unsafe impl objc2::encode::Encode for CGPoint {
    const ENCODING: objc2::encode::Encoding =
        objc2::encode::Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}

unsafe impl objc2::encode::Encode for CGRect {
    const ENCODING: objc2::encode::Encoding = objc2::encode::Encoding::Struct(
        "CGRect",
        &[CGPoint::ENCODING, objc2::encode::Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING])],
    );
}

thread_local! {
    /// The tag of the item picked in the menu that is open, if any.
    static PICKED: Cell<Option<isize>> = const { Cell::new(None) };
    static PENDING: RefCell<Vec<Box<dyn FnOnce()>>> = const { RefCell::new(Vec::new()) };
}

/// Runs `run` on a later run loop turn, with nothing borrowed.
pub(crate) fn popup_soon(run: impl FnOnce() + 'static) {
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
            let class: &AnyClass = match ClassBuilder::new(c"GpuiCefMenuTarget", class!(NSObject)) {
                Some(mut builder) => {
                    unsafe {
                        builder.add_method(
                            sel!(pick:),
                            pick as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
                        )
                    };
                    builder.register()
                }
                None => AnyClass::get(c"GpuiCefMenuTarget").expect("registered"),
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

/// An NSMenu for `items`; picked items report `tag(path)`.
fn build(items: &[MenuItem], tags: &mut Vec<usize>, prefix: &[usize]) -> *mut AnyObject {
    unsafe {
        let menu: *mut AnyObject = msg_send![class!(NSMenu), alloc];
        let menu: *mut AnyObject = msg_send![menu, initWithTitle: ns_string("")];
        let _: () = msg_send![menu, setAutoenablesItems: Bool::NO];
        for (i, item) in items.iter().enumerate() {
            let mut path = prefix.to_vec();
            path.push(i);
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
                    let sub = build(items, tags, &path);
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
/// `view`, until the menu closes; the picked item's id. Main thread, outside
/// gpui updates and CEF callbacks (see [`popup_soon`]).
pub(crate) fn popup(view: *mut AnyObject, x: f32, y: f32, items: &[MenuItem]) -> Option<usize> {
    if view.is_null() || items.is_empty() {
        return None;
    }
    let mut tags = Vec::new();
    let menu = build(items, &mut tags, &[]);
    PICKED.with(|p| p.set(None));
    unsafe {
        let flipped: Bool = msg_send![view, isFlipped];
        let bounds: CGRect = msg_send![view, bounds];
        let at = CGPoint { x: x as f64, y: if flipped.as_bool() { y as f64 } else { bounds.size.y - y as f64 } };
        let _: Bool = msg_send![menu, popUpMenuPositioningItem: std::ptr::null_mut::<AnyObject>(), atLocation: at, inView: view];
    }
    PICKED.with(|p| p.take()).and_then(|tag| tags.get(tag as usize).copied())
}

/// Chrome's menu for a page (its items, extensions' included) as
/// [`MenuItem`]s; ids are Chrome's command ids.
fn from_model(model: &MenuModel) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = Vec::new();
    for i in 0..model.count() {
        if model.is_visible_at(i) == 0 {
            continue;
        }
        let label = mnemonic_free(&cef::CefString::from(&model.label_at(i)).to_string());
        let kind = model.type_at(i);
        let item = if kind == MenuItemType::SEPARATOR {
            // Chrome's model can leave separators doubled or trailing once hidden items go.
            if matches!(items.last(), None | Some(MenuItem::Separator)) {
                continue;
            }
            MenuItem::Separator
        } else if kind == MenuItemType::SUBMENU {
            let Some(sub) = model.sub_menu_at(i) else { continue };
            MenuItem::Submenu { label, items: from_model(&sub) }
        } else {
            MenuItem::Action {
                label,
                enabled: model.is_enabled_at(i) != 0,
                checked: model.is_checked_at(i) != 0,
                id: model.command_id_at(i).max(0) as usize,
            }
        };
        items.push(item);
    }
    if matches!(items.last(), Some(MenuItem::Separator)) {
        items.pop();
    }
    items
}

/// Chrome's labels mark a mnemonic with `&` and a literal one with `&&`;
/// macOS menus have no mnemonics.
fn mnemonic_free(label: &str) -> String {
    label.replace("&&", "\u{0}").replace('&', "").replace('\u{0}', "&")
}

/// Chrome's Inspect item, which the app handles: Chrome would open DevTools
/// against the tab's off-screen window.
fn inspect_id(items: &[MenuItem]) -> Option<usize> {
    items.iter().find_map(|item| match item {
        // ponytail: found by its English label. Ceiling: Chrome's command id,
        // which CEF's menu ids do not name.
        MenuItem::Action { label, id, .. } if label == "Inspect" => Some(*id),
        _ => None,
    })
}

/// Shows a page's Chrome menu natively over `view` and answers `callback`;
/// Inspect goes to `inspect` instead.
pub(crate) fn run_page_menu(
    view: *mut AnyObject,
    x: f32,
    y: f32,
    model: &MenuModel,
    callback: RunContextMenuCallback,
    inspect: impl FnOnce() + 'static,
) {
    let items = from_model(model);
    let inspect_id = inspect_id(&items);
    popup_soon(move || match popup(view, x, y, &items) {
        Some(id) if Some(id) == inspect_id => {
            callback.cancel();
            inspect();
        }
        Some(id) => callback.cont(id as i32, cef::EventFlags::default()),
        None => callback.cancel(),
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn mnemonics_go_and_literal_ampersands_stay() {
        assert_eq!(super::mnemonic_free("&Back"), "Back");
        assert_eq!(super::mnemonic_free("Save &As…"), "Save As…");
        assert_eq!(super::mnemonic_free("Tom && Jerry"), "Tom & Jerry");
    }
}
