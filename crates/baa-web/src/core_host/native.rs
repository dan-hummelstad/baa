//! The page's NSView under gpui's view.
//!
//! The window's content view holds a clipping container ordered below gpui's
//! view; gpui punches a transparent hole where the page goes. gpui keeps the
//! mouse and hands the original NSEvent on; keyboard and IME go to Chromium
//! natively as first responder.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

use core_video::pixel_buffer::{kCVPixelFormatType_OneComponent8, CVPixelBuffer};
use gpui::{Bounds, Pixels, Window};
use objc2::{
    class,
    encode::{Encode, Encoding},
    msg_send,
    runtime::{AnyClass, AnyObject, Bool, Imp, Sel},
    sel,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

unsafe impl Encode for CGPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for CGSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for CGRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[CGPoint::ENCODING, CGSize::ENCODING]);
}

const NS_WINDOW_BELOW: isize = -1;
#[cfg(test)]
const NS_WINDOW_ABOVE: isize = 1;
/// NSViewWidthSizable | NSViewHeightSizable.
const FILL: usize = 2 | 16;
/// NSViewMinYMargin: in an unflipped superview, keeps the top-left corner put.
const TOP_LEFT: usize = 8;
/// Each new size costs the page a full layout; sent faster they queue up and
/// the page crawls through stale sizes after a resize ends.
const RESIZE_INTERVAL: f64 = 0.04;

pub fn gpui_view(window: &Window) -> *mut AnyObject {
    match HasWindowHandle::window_handle(window).map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::AppKit(handle)) => handle.ns_view.as_ptr().cast(),
        _ => std::ptr::null_mut(),
    }
}

/// gpui's `GPUIApplication` is NSApplication plus this ivar; BaaCore's NSApp
/// class must carry it too (chromium/baa/app_mac.mm).
pub fn nsapp_has_gpui_ivar() -> bool {
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        (*app).class().instance_variable(c"platform").is_some()
    }
}

/// A layer-backed view that clips the page to the element's visible rect and
/// rounds its corners. Owns one reference; removed from the window on drop.
pub struct Container {
    view: *mut AnyObject,
    /// The tab's own NSView inside `view`; null until the tab has one.
    page: Cell<*mut AnyObject>,
    placed: Cell<Option<CGRect>>,
    /// What the app asked for; layout never shows a page the app hid.
    visible: Cell<bool>,
    /// Kept above the other pages' views (an overlay like Peek). Native views
    /// stack in insertion order, not in gpui's paint order.
    floating: Cell<bool>,
    gpui_view: Cell<*mut AnyObject>,
    resized_at: Cell<f64>,
}

impl Container {
    /// Below `gpui_view` (not attached when null, as in tests), holding
    /// `page` (may be null until the tab has a view). Hidden until the first
    /// [`Container::place`].
    pub fn new(gpui_view: *mut AnyObject, page: *mut AnyObject, corner_radius: f32) -> Self {
        unsafe {
            let view: *mut AnyObject = msg_send![class!(NSView), alloc];
            let one = CGRect { origin: CGPoint { x: 0., y: 0. }, size: CGSize { width: 1., height: 1. } };
            let view: *mut AnyObject = msg_send![view, initWithFrame: one];
            let _: () = msg_send![view, setWantsLayer: Bool::YES];
            let layer: *mut AnyObject = msg_send![view, layer];
            let _: () = msg_send![layer, setCornerRadius: corner_radius as f64];
            let _: () = msg_send![layer, setMasksToBounds: Bool::YES];
            let _: () = msg_send![view, setHidden: Bool::YES];
            // gpui lays the page out on its next frame; during a live resize
            // AppKit resizes now, so follow the window until then.
            let _: () = msg_send![view, setAutoresizingMask: FILL];
            let this = Self {
                view,
                page: Cell::new(std::ptr::null_mut()),
                placed: Cell::new(None),
                visible: Cell::new(true),
                floating: Cell::new(false),
                gpui_view: Cell::new(std::ptr::null_mut()),
                resized_at: Cell::new(0.),
            };
            this.move_below(gpui_view);
            this.swap_page(page);
            this
        }
    }

    #[cfg(test)]
    pub fn as_ptr(&self) -> *mut AnyObject {
        self.view
    }

    pub fn set_corner_radius(&self, radius: f32) {
        unsafe {
            let layer: *mut AnyObject = msg_send![self.view, layer];
            let _: () = msg_send![layer, setCornerRadius: radius as f64];
        }
    }

    /// Leaves the `square` corners (top left 1, top right 2, bottom left 4,
    /// bottom right 8) unrounded.
    pub fn set_square_corners(&self, square: u8) {
        // CACornerMask: the view is not flipped, so "min Y" is the bottom.
        let mut rounded = 1 | 2 | 4 | 8;
        for (bit, layer_corner) in [(1u8, 4usize), (2, 8), (4, 1), (8, 2)] {
            if square & bit != 0 {
                rounded &= !layer_corner;
            }
        }
        unsafe {
            let layer: *mut AnyObject = msg_send![self.view, layer];
            let _: () = msg_send![layer, setMaskedCorners: rounded];
        }
    }

    /// Re-parents the container (and the page in it) under another window's
    /// gpui view.
    pub fn move_below(&self, gpui_view: *mut AnyObject) {
        if gpui_view.is_null() {
            return;
        }
        unsafe {
            // Behind the page holes: white, like a page, rather than the
            // desktop through gpui's transparent layer while a tab switches.
            let window: *mut AnyObject = msg_send![gpui_view, window];
            if !window.is_null() {
                let white: *mut AnyObject = msg_send![class!(NSColor), whiteColor];
                let _: () = msg_send![window, setBackgroundColor: white];
            }
            let content: *mut AnyObject = msg_send![gpui_view, superview];
            let _: () = msg_send![self.view, removeFromSuperview];
            let _: () = msg_send![content, addSubview: self.view, positioned: NS_WINDOW_BELOW, relativeTo: gpui_view];
        }
        self.placed.set(None);
        self.gpui_view.set(gpui_view);
    }

    /// Over gpui's view instead of under it: a page that is an overlay on the
    /// UI rather than a hole in it. Mouse and keys go to the page natively.
    // ponytail: no overlay page is left in the app (the settings sheet was the one); the tests keep the mechanism.
    #[cfg(test)]
    pub fn move_above(&self, gpui_view: *mut AnyObject) {
        if gpui_view.is_null() {
            return;
        }
        unsafe {
            let content: *mut AnyObject = msg_send![gpui_view, superview];
            let _: () = msg_send![self.view, removeFromSuperview];
            let _: () = msg_send![content, addSubview: self.view, positioned: NS_WINDOW_ABOVE, relativeTo: gpui_view];
        }
        self.placed.set(None);
        OVERLAYS.with(|o| {
            let mut o = o.borrow_mut();
            if !o.contains(&self.view) {
                o.push(self.view);
            }
        });
    }

    /// Whether the container is in the same view as `gpui_view`.
    #[cfg(test)]
    pub fn is_beside(&self, gpui_view: *mut AnyObject) -> bool {
        if gpui_view.is_null() {
            return true;
        }
        unsafe {
            let mine: *mut AnyObject = msg_send![self.view, superview];
            let theirs: *mut AnyObject = msg_send![gpui_view, superview];
            !mine.is_null() && mine == theirs
        }
    }

    pub fn set_floating(&self, floating: bool) {
        self.floating.set(floating);
    }

    /// Directly below gpui's view, i.e. above every other page.
    fn raise(&self) {
        let gpui_view = self.gpui_view.get();
        if gpui_view.is_null() {
            return;
        }
        unsafe {
            let content: *mut AnyObject = msg_send![gpui_view, superview];
            let subviews: *mut AnyObject = msg_send![content, subviews];
            // Directly below gpui's view, wherever that is: an overlay (the
            // settings sheet) may sit above it.
            let at: usize = msg_send![subviews, indexOfObject: gpui_view];
            let count: usize = msg_send![subviews, count];
            if at == 0 || at >= count {
                return;
            }
            let top: *mut AnyObject = msg_send![subviews, objectAtIndex: at - 1];
            if top != self.view {
                let _: () = msg_send![content, addSubview: self.view, positioned: NS_WINDOW_BELOW, relativeTo: gpui_view];
            }
        }
    }

    pub fn page(&self) -> *mut AnyObject {
        self.page.get()
    }

    /// Chrome swapped the tab's view (discard, reload): the new one takes the
    /// old one's place. May be null (the tab is gone). True when it changed.
    pub fn swap_page(&self, new: *mut AnyObject) -> bool {
        let old = self.page.replace(new);
        if old == new {
            return false;
        }
        detach(old);
        if !new.is_null() {
            unsafe {
                let _: () = msg_send![self.view, addSubview: new];
            }
            if !self.visible.get() {
                set_view_hidden(new, true);
            }
            self.page_created();
        }
        true
    }

    /// Puts the container at `visible` (the element's rect clipped by its
    /// content mask) and the page at `bounds`, both in gpui window
    /// coordinates. True when the page's new size is held back: call again on
    /// a later frame.
    pub fn place(&self, bounds: Bounds<Pixels>, visible: Bounds<Pixels>) -> bool {
        let page = self.page.get();
        let mut pending = false;
        unsafe {
            let content: *mut AnyObject = msg_send![self.view, superview];
            if content.is_null() {
                return false;
            }
            // The content view is not flipped: y grows upwards.
            let content_frame: CGRect = msg_send![content, bounds];
            let height = content_frame.size.height;
            let px = |p: Pixels| f32::from(p) as f64;
            let clip = CGRect {
                origin: CGPoint { x: px(visible.origin.x), y: height - px(visible.bottom()) },
                size: CGSize { width: px(visible.size.width), height: px(visible.size.height) },
            };
            let inner = CGRect {
                origin: CGPoint { x: px(bounds.origin.x - visible.origin.x), y: px(visible.bottom() - bounds.bottom()) },
                size: CGSize { width: px(bounds.size.width), height: px(bounds.size.height) },
            };
            if self.floating.get() {
                self.raise();
            }
            if self.placed.get() != Some(clip) {
                self.placed.set(Some(clip));
                let _: () = msg_send![self.view, setFrame: clip];
            }
            if !page.is_null() {
                let current: CGRect = msg_send![page, frame];
                if current != inner {
                    let now = core_foundation::date::CFAbsoluteTimeGetCurrent();
                    let resized = current.size != inner.size;
                    if !resized || now - self.resized_at.get() >= RESIZE_INTERVAL {
                        if resized {
                            self.resized_at.set(now);
                        }
                        // Held sizes keep the page pinned to the card's top left.
                        let _: () = msg_send![page, setAutoresizingMask: TOP_LEFT];
                        let _: () = msg_send![page, setFrame: inner];
                    } else {
                        pending = true;
                    }
                }
            }
            if self.visible.get() {
                self.set_hidden(false);
                set_view_hidden(page, false);
            }
        }
        pending
    }

    /// Chromium only notices visibility changes, and a page created behind a
    /// hidden container starts out believing it is visible: show it a hide.
    /// AppKit reports only effective changes, so the container is unhidden for
    /// the toggle too; all in one run loop turn, so nothing reaches the screen.
    fn page_created(&self) {
        let page = self.page.get();
        if !self.visible.get() && !page.is_null() {
            self.set_hidden(false);
            set_view_hidden(page, false);
            set_view_hidden(page, true);
            self.set_hidden(true);
        }
    }

    /// Showing waits for the next [`Container::place`].
    pub fn set_visible(&self, visible: bool) {
        self.visible.set(visible);
        if !visible {
            self.set_hidden(true);
            set_view_hidden(self.page.get(), true);
        }
    }

    pub fn set_hidden(&self, hidden: bool) {
        unsafe {
            let is_hidden: Bool = msg_send![self.view, isHidden];
            if is_hidden.as_bool() != hidden {
                let _: () = msg_send![self.view, setHidden: Bool::new(hidden)];
            }
        }
    }
}

fn set_view_hidden(view: *mut AnyObject, hidden: bool) {
    if view.is_null() {
        return;
    }
    unsafe {
        let is_hidden: Bool = msg_send![view, isHidden];
        if is_hidden.as_bool() != hidden {
            let _: () = msg_send![view, setHidden: Bool::new(hidden)];
        }
    }
}

/// Takes a page's view out of the window.
pub fn detach(view: *mut AnyObject) {
    if !view.is_null() {
        unsafe {
            let _: () = msg_send![view, removeFromSuperview];
        }
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        // try_with: a container may outlive the thread-locals at thread exit.
        let _ = OVERLAYS.try_with(|o| o.borrow_mut().retain(|v| *v != self.view));
        unsafe {
            let _: () = msg_send![self.view, removeFromSuperview];
            let _: () = msg_send![self.view, release];
        }
    }
}

const MOUSE_SELECTORS: [&str; 11] = [
    "mouseDown:",
    "mouseUp:",
    "mouseMoved:",
    "mouseDragged:",
    "rightMouseDown:",
    "rightMouseUp:",
    "rightMouseDragged:",
    "otherMouseDown:",
    "otherMouseUp:",
    "otherMouseDragged:",
    "scrollWheel:",
];

thread_local! {
    /// The mouse NSEvent gpui's view is handling right now.
    static EVENT: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) };
    static ORIGINALS: RefCell<HashMap<Sel, Imp>> = RefCell::new(HashMap::new());
    /// While an event is handed to the page, gpui's view steps out of hit tests.
    static FORWARDING: Cell<bool> = const { Cell::new(false) };
    static GPUI_VIEW_CLASS: Cell<*const AnyClass> = const { Cell::new(std::ptr::null()) };
    /// Containers above gpui's view (the settings sheet): the page there takes the mouse itself.
    static OVERLAYS: RefCell<Vec<*mut AnyObject>> = const { RefCell::new(Vec::new()) };
    static WINDOW_SEND_EVENT: Cell<Option<Imp>> = const { Cell::new(None) };
}

/// gpui's view keeps the mouse: it hit-tests with its own occlusion and the
/// page element hands the original NSEvent on. This wraps gpui's view's mouse
/// methods so they record that event. Idempotent.
// ponytail: method replacement on gpui's view class. Ceiling: breaks if gpui
// stops handling these selectors on its NSView. Upgrade: an event hook in gpui.
pub fn install_mouse_forwarding(gpui_view: *mut AnyObject) {
    if gpui_view.is_null() || ORIGINALS.with(|o| !o.borrow().is_empty()) {
        return;
    }
    type Handler = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject);
    unsafe extern "C-unwind" fn record(this: *mut AnyObject, cmd: Sel, event: *mut AnyObject) {
        let previous = EVENT.with(|e| e.replace(event));
        if let Some(original) = ORIGINALS.with(|o| o.borrow().get(&cmd).copied()) {
            let original: Handler = unsafe { std::mem::transmute::<Imp, Handler>(original) };
            unsafe { original(this, cmd, event) };
        }
        EVENT.with(|e| e.set(previous));
    }
    unsafe {
        let class = objc2::ffi::object_getClass(gpui_view) as *mut AnyClass;
        for name in MOUSE_SELECTORS {
            let selector = Sel::register(&std::ffi::CString::new(name).unwrap());
            let Some(method) = (*class).instance_method(selector) else { continue };
            ORIGINALS.with(|o| o.borrow_mut().insert(selector, method.implementation()));
            objc2::ffi::class_replaceMethod(
                class,
                selector,
                std::mem::transmute::<Handler, Imp>(record),
                c"v@:@".as_ptr(),
            );
        }
        install_hit_test(class);
        GPUI_VIEW_CLASS.with(|c| c.set(class));
        let window: *mut AnyObject = msg_send![gpui_view, window];
        if !window.is_null() {
            route_mouse_moves(objc2::ffi::object_getClass(window) as *mut AnyClass);
        }
    }
}

/// AppKit sends mouse moves to the first responder, which is Chromium's view
/// while the page has focus; gpui would never see the pointer move.
unsafe fn route_mouse_moves(window_class: *mut AnyClass) {
    type Handler = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject);
    unsafe extern "C-unwind" fn send_event(this: *mut AnyObject, cmd: Sel, event: *mut AnyObject) {
        unsafe {
            let kind: usize = msg_send![event, type];
            if kind == 5 && !over_overlay(this, event) {
                if let Some(view) = find_gpui_view(this) {
                    let _: () = msg_send![view, mouseMoved: event];
                    return;
                }
            }
            if let Some(original) = WINDOW_SEND_EVENT.with(Cell::get) {
                let original: Handler = std::mem::transmute::<Imp, Handler>(original);
                original(this, cmd, event);
            }
        }
    }
    unsafe {
        let Some(method) = (*window_class).instance_method(sel!(sendEvent:)) else { return };
        WINDOW_SEND_EVENT.with(|o| o.set(Some(method.implementation())));
        objc2::ffi::class_replaceMethod(
            window_class,
            sel!(sendEvent:),
            std::mem::transmute::<Handler, Imp>(send_event),
            c"v@:@".as_ptr(),
        );
    }
}

/// Whether the pointer is over a visible overlay container (or something in it).
unsafe fn over_overlay(window: *mut AnyObject, event: *mut AnyObject) -> bool {
    if OVERLAYS.with(|o| o.borrow().is_empty()) {
        return false;
    }
    unsafe {
        let content: *mut AnyObject = msg_send![window, contentView];
        if content.is_null() {
            return false;
        }
        let parent: *mut AnyObject = msg_send![content, superview];
        let at: CGPoint = msg_send![event, locationInWindow];
        let point: CGPoint = msg_send![parent, convertPoint: at, fromView: std::ptr::null_mut::<AnyObject>()];
        let mut view: *mut AnyObject = msg_send![content, hitTest: point];
        while !view.is_null() {
            if OVERLAYS.with(|o| o.borrow().contains(&view)) {
                return true;
            }
            view = msg_send![view, superview];
        }
        false
    }
}

unsafe fn find_gpui_view(window: *mut AnyObject) -> Option<*mut AnyObject> {
    let class = GPUI_VIEW_CLASS.with(Cell::get);
    unsafe {
        let content: *mut AnyObject = msg_send![window, contentView];
        if content.is_null() {
            return None;
        }
        let subviews: *mut AnyObject = msg_send![content, subviews];
        let count: usize = msg_send![subviews, count];
        (0..count).rev().find_map(|i| {
            let view: *mut AnyObject = msg_send![subviews, objectAtIndex: i];
            (objc2::ffi::object_getClass(view) == class).then_some(view)
        })
    }
}

/// Chromium drops a mouse event unless the window's hit test lands on its own
/// view; while one is handed on, gpui's view answers nil.
unsafe fn install_hit_test(class: *mut AnyClass) {
    type HitTest = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, CGPoint) -> *mut AnyObject;
    unsafe extern "C-unwind" fn hit_test(this: *mut AnyObject, _: Sel, point: CGPoint) -> *mut AnyObject {
        if FORWARDING.with(Cell::get) {
            return std::ptr::null_mut();
        }
        unsafe {
            let this = &*this;
            msg_send![super(this, class!(NSView)), hitTest: point]
        }
    }
    unsafe {
        objc2::ffi::class_addMethod(
            class,
            sel!(hitTest:),
            std::mem::transmute::<HitTest, Imp>(hit_test),
            c"@@:{CGPoint=dd}".as_ptr(),
        );
    }
}

/// Hands the mouse event gpui is handling to the deepest view of `page` under
/// it, or to `target` (where a drag started). Returns the view that got it.
pub fn forward_current_event(page: *mut AnyObject, target: Option<*mut AnyObject>) -> Option<*mut AnyObject> {
    let event = EVENT.with(Cell::get);
    if page.is_null() || event.is_null() {
        return None;
    }
    unsafe {
        let kind: usize = msg_send![event, type];
        let selector = match kind {
            1 => sel!(mouseDown:),
            2 => sel!(mouseUp:),
            3 => sel!(rightMouseDown:),
            4 => sel!(rightMouseUp:),
            5 => sel!(mouseMoved:),
            6 => sel!(mouseDragged:),
            7 => sel!(rightMouseDragged:),
            22 => sel!(scrollWheel:),
            25 => sel!(otherMouseDown:),
            26 => sel!(otherMouseUp:),
            27 => sel!(otherMouseDragged:),
            _ => return None,
        };
        let target = match target {
            Some(target) => target,
            None => {
                let in_window: CGPoint = msg_send![event, locationInWindow];
                let parent: *mut AnyObject = msg_send![page, superview];
                let point: CGPoint =
                    msg_send![parent, convertPoint: in_window, fromView: std::ptr::null_mut::<AnyObject>()];
                let hit: *mut AnyObject = msg_send![page, hitTest: point];
                if hit.is_null() { page } else { hit }
            }
        };
        FORWARDING.with(|f| f.set(true));
        let _: *mut AnyObject = msg_send![target, performSelector: selector, withObject: event];
        FORWARDING.with(|f| f.set(false));
        Some(target)
    }
}

/// Offers a ⌘/⌃/Escape key-down to gpui's view in the event's window.
/// NSEventTypeKeyDown. Chromium also reports a modifier pressed on its own (⌘,
/// ⌃) as a key event whose NSEvent is flagsChanged; AppKit throws on asking
/// that for its characters, and an ObjC exception through these callbacks
/// aborts the app.
fn is_key_down(event: *mut AnyObject) -> bool {
    if event.is_null() {
        return false;
    }
    let kind: usize = unsafe { msg_send![event, type] };
    kind == 10
}

pub fn offer_key_equivalent(event: *mut AnyObject) -> bool {
    if !is_key_down(event) {
        return false;
    }
    unsafe {
        let window: *mut AnyObject = msg_send![event, window];
        let Some(gpui_view) = (!window.is_null()).then(|| find_gpui_view(window)).flatten() else {
            return false;
        };
        let handled: Bool = msg_send![gpui_view, performKeyEquivalent: event];
        handled.as_bool()
    }
}

pub fn make_first_responder(view: *mut AnyObject) {
    if view.is_null() {
        return;
    }
    unsafe {
        let window: *mut AnyObject = msg_send![view, window];
        if !window.is_null() {
            let _: Bool = msg_send![window, makeFirstResponder: view];
        }
    }
}

/// Whether the window's first responder is `view` or inside it.
pub fn has_first_responder(view: *mut AnyObject) -> bool {
    if view.is_null() {
        return false;
    }
    unsafe {
        let window: *mut AnyObject = msg_send![view, window];
        if window.is_null() {
            return false;
        }
        let responder: *mut AnyObject = msg_send![window, firstResponder];
        let is_view: Bool = msg_send![responder, isKindOfClass: class!(NSView)];
        if responder.is_null() || !is_view.as_bool() {
            return false;
        }
        let inside: Bool = msg_send![responder, isDescendantOf: view];
        inside.as_bool()
    }
}

/// ⌘C/⌘X/⌘V/⌘A/⌘Z/⇧⌘Z as the responder-chain action that runs it; None for
/// any other key.
pub fn edit_action(event: *mut AnyObject) -> Option<Sel> {
    // NSEventModifierFlags: shift, control, option, command.
    const SHIFT: usize = 1 << 17;
    const OTHERS: usize = (1 << 18) | (1 << 19);
    const COMMAND: usize = 1 << 20;
    if !is_key_down(event) {
        return None;
    }
    unsafe {
        let flags: usize = msg_send![event, modifierFlags];
        if flags & COMMAND == 0 || flags & OTHERS != 0 {
            return None;
        }
        let chars: *mut AnyObject = msg_send![event, charactersIgnoringModifiers];
        let utf8: *const std::ffi::c_char = msg_send![chars, UTF8String];
        if utf8.is_null() {
            return None;
        }
        let key = std::ffi::CStr::from_ptr(utf8).to_bytes().first()?.to_ascii_lowercase();
        Some(match (key, flags & SHIFT != 0) {
            (b'c', false) => sel!(copy:),
            (b'x', false) => sel!(cut:),
            (b'v', false) => sel!(paste:),
            (b'a', false) => sel!(selectAll:),
            (b'z', false) => sel!(undo:),
            (b'z', true) => sel!(redo:),
            _ => return None,
        })
    }
}

/// Runs `action` on the first responder (the page's view), the way a Chrome
/// window's Edit menu would; there is none behind the page.
pub fn send_edit_action(action: Sel) -> bool {
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let handled: Bool = msg_send![
            app,
            sendAction: action,
            to: std::ptr::null_mut::<AnyObject>(),
            from: std::ptr::null_mut::<AnyObject>()
        ];
        handled.as_bool()
    }
}

/// The marker surface the vendored gpui renderer turns into a transparent hole
/// with `radius` (device pixels) rounded corners; `square` corners stay square
/// (bits as in `Container::set_square_corners`).
pub fn punch_buffer(radius: usize, square: u8) -> Option<CVPixelBuffer> {
    thread_local! {
        static CACHE: RefCell<HashMap<(usize, u8), CVPixelBuffer>> = RefCell::new(HashMap::new());
    }
    let radius = radius.max(1);
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(buffer) = cache.get(&(radius, square)) {
            return Some(buffer.clone());
        }
        let buffer = CVPixelBuffer::new(kCVPixelFormatType_OneComponent8, radius, 1 + square as usize, None).ok()?;
        cache.insert((radius, square), buffer.clone());
        Some(buffer)
    })
}

#[cfg(test)]
mod key_tests {
    use super::*;

    /// ⌘ pressed on its own: a flagsChanged event, which throws if asked for
    /// its characters (that abort once took the app down).
    #[test]
    fn modifier_only_key_events_are_left_alone() {
        unsafe {
            let empty: *mut AnyObject = msg_send![class!(NSString), string];
            let event: *mut AnyObject = msg_send![
                class!(NSEvent),
                keyEventWithType: 12usize,
                location: CGPoint { x: 0., y: 0. },
                modifierFlags: 1usize << 20,
                timestamp: 0f64,
                windowNumber: 0isize,
                context: std::ptr::null_mut::<AnyObject>(),
                characters: empty,
                charactersIgnoringModifiers: empty,
                isARepeat: Bool::NO,
                keyCode: 55u16
            ];
            assert!(!event.is_null());
            assert_eq!(edit_action(event), None);
            assert!(!offer_key_equivalent(event));
        }
    }
}
