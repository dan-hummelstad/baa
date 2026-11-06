//! The page's NSView under gpui's view.
//!
//! The window's content view holds a clipping container ordered below gpui's
//! view; gpui punches a transparent hole where the page goes. gpui keeps the
//! mouse and hands the original NSEvent on; keyboard and IME go to Chromium
//! natively as first responder.
// ponytail: a trimmed copy of baa-web's core_host/native.rs (the harness is its own crate
// so cargo never links it into the app).

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
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
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
/// class must carry it too (chromium/baa/baa_mac.mm).
pub fn nsapp_has_gpui_ivar() -> bool {
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        (*app).class().instance_variable(c"platform").is_some()
    }
}

pub struct Container {
    view: *mut AnyObject,
    placed: Cell<Option<CGRect>>,
    resized_at: Cell<f64>,
}

impl Container {
    /// A container below `gpui_view` holding `page`.
    pub fn new(gpui_view: *mut AnyObject, page: *mut AnyObject, corner_radius: f64) -> Self {
        unsafe {
            let view: *mut AnyObject = msg_send![class!(NSView), alloc];
            let one = CGRect { origin: CGPoint { x: 0., y: 0. }, size: CGSize { width: 1., height: 1. } };
            let view: *mut AnyObject = msg_send![view, initWithFrame: one];
            let _: () = msg_send![view, setWantsLayer: Bool::YES];
            let layer: *mut AnyObject = msg_send![view, layer];
            let _: () = msg_send![layer, setCornerRadius: corner_radius];
            let _: () = msg_send![layer, setMasksToBounds: Bool::YES];
            // gpui lays the page out on its next frame; during a live resize
            // AppKit resizes now, so follow the window until then.
            let _: () = msg_send![view, setAutoresizingMask: FILL];
            let window: *mut AnyObject = msg_send![gpui_view, window];
            let white: *mut AnyObject = msg_send![class!(NSColor), whiteColor];
            let _: () = msg_send![window, setBackgroundColor: white];
            let content: *mut AnyObject = msg_send![gpui_view, superview];
            let _: () = msg_send![content, addSubview: view, positioned: NS_WINDOW_BELOW, relativeTo: gpui_view];
            let _: () = msg_send![view, addSubview: page];
            Self { view, placed: Cell::new(None), resized_at: Cell::new(0.) }
        }
    }

    /// Puts the container at `visible` and the page at `bounds` (gpui window
    /// coordinates). True when the page's new size is held back: call again
    /// on a later frame.
    pub fn place(&self, page: *mut AnyObject, bounds: Bounds<Pixels>, visible: Bounds<Pixels>) -> bool {
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
                origin: CGPoint {
                    x: px(bounds.origin.x - visible.origin.x),
                    y: px(visible.bottom() - bounds.bottom()),
                },
                size: CGSize { width: px(bounds.size.width), height: px(bounds.size.height) },
            };
            if self.placed.get() != Some(clip) {
                self.placed.set(Some(clip));
                let _: () = msg_send![self.view, setFrame: clip];
            }
            let current: CGRect = msg_send![page, frame];
            if current == inner {
                return false;
            }
            let now = core_foundation::date::CFAbsoluteTimeGetCurrent();
            let resized = current.size != inner.size;
            if resized && now - self.resized_at.get() < RESIZE_INTERVAL {
                return true;
            }
            if resized {
                self.resized_at.set(now);
            }
            let _: () = msg_send![page, setAutoresizingMask: TOP_LEFT];
            let _: () = msg_send![page, setFrame: inner];
            false
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
            if kind == 5 {
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

pub fn offer_key_equivalent(gpui_view: *mut AnyObject, event: *mut AnyObject) -> bool {
    if gpui_view.is_null() || event.is_null() {
        return false;
    }
    let handled: Bool = unsafe { msg_send![gpui_view, performKeyEquivalent: event] };
    handled.as_bool()
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

/// ⌘C/⌘X/⌘V/⌘A/⌘Z/⇧⌘Z as a BaaCore edit command; None for any other key.
pub fn edit_command(event: *mut AnyObject) -> Option<std::ffi::c_int> {
    // NSEventModifierFlags: shift, control, option, command.
    const SHIFT: usize = 1 << 17;
    const OTHERS: usize = (1 << 18) | (1 << 19);
    const COMMAND: usize = 1 << 20;
    use crate::ffi::*;
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
            (b'c', false) => EDIT_COPY,
            (b'x', false) => EDIT_CUT,
            (b'v', false) => EDIT_PASTE,
            (b'a', false) => EDIT_SELECT_ALL,
            (b'z', false) => EDIT_UNDO,
            (b'z', true) => EDIT_REDO,
            _ => return None,
        })
    }
}

/// An edit command as an action down the responder chain, which reaches the
/// page's RenderWidgetHostViewCocoa (copy:, cut:, paste:, selectAll:, undo:, redo:).
pub fn send_edit_action(command: std::ffi::c_int) {
    use crate::ffi::*;
    let action = match command {
        EDIT_COPY => sel!(copy:),
        EDIT_CUT => sel!(cut:),
        EDIT_PASTE => sel!(paste:),
        EDIT_SELECT_ALL => sel!(selectAll:),
        EDIT_UNDO => sel!(undo:),
        _ => sel!(redo:),
    };
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let _: Bool = msg_send![app, sendAction: action, to: std::ptr::null_mut::<AnyObject>(), from: std::ptr::null_mut::<AnyObject>()];
    }
}

/// Queues a key press (down and up) for the window numbered `window_number`,
/// through the same `-[NSApp sendEvent:]` path as a real keystroke.
pub fn post_key(window_number: isize, key_code: u16, chars: &str, command: bool) {
    unsafe {
        let chars = std::ffi::CString::new(chars).unwrap();
        let chars: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: chars.as_ptr()];
        let flags: usize = if command { 1 << 20 } else { 0 };
        let info: *mut AnyObject = msg_send![class!(NSProcessInfo), processInfo];
        let now: f64 = msg_send![info, systemUptime];
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        for kind in [10usize, 11] {
            let event: *mut AnyObject = msg_send![
                class!(NSEvent),
                keyEventWithType: kind,
                location: CGPoint { x: 0., y: 0. },
                modifierFlags: flags,
                timestamp: now,
                windowNumber: window_number,
                context: std::ptr::null_mut::<AnyObject>(),
                characters: chars,
                charactersIgnoringModifiers: chars,
                isARepeat: Bool::NO,
                keyCode: key_code
            ];
            let _: () = msg_send![app, postEvent: event, atStart: Bool::NO];
        }
    }
}

/// Queues a left click (with ⌘ held when `command`) at (`x`, `y`) points from `view`'s top left.
pub fn post_click(view: *mut AnyObject, x: f64, y: f64, command: bool, right: bool) {
    unsafe {
        let bounds: CGRect = msg_send![view, bounds];
        let flipped: Bool = msg_send![view, isFlipped];
        let y = if flipped.as_bool() { y } else { bounds.size.height - y };
        let point: CGPoint = msg_send![view, convertPoint: CGPoint { x, y }, toView: std::ptr::null_mut::<AnyObject>()];
        let window: *mut AnyObject = msg_send![view, window];
        let number: isize = msg_send![window, windowNumber];
        let info: *mut AnyObject = msg_send![class!(NSProcessInfo), processInfo];
        let now: f64 = msg_send![info, systemUptime];
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        // NSEventTypeLeftMouseDown/Up, or the right ones (3, 4).
        for kind in if right { [3usize, 4] } else { [1, 2] } {
            let event: *mut AnyObject = msg_send![
                class!(NSEvent),
                mouseEventWithType: kind,
                location: point,
                modifierFlags: if command { 1usize << 20 } else { 0 },
                timestamp: now,
                windowNumber: number,
                context: std::ptr::null_mut::<AnyObject>(),
                eventNumber: 0isize,
                clickCount: 1isize,
                pressure: if kind == 1 || kind == 3 { 1.0f32 } else { 0.0f32 }
            ];
            let _: () = msg_send![app, postEvent: event, atStart: Bool::NO];
        }
    }
}

pub fn window_number(view: *mut AnyObject) -> isize {
    unsafe {
        let window: *mut AnyObject = msg_send![view, window];
        msg_send![window, windowNumber]
    }
}

/// The marker surface the vendored gpui renderer turns into a transparent hole
/// with `radius` (device pixels) rounded corners.
pub fn punch_buffer(radius: usize) -> Option<CVPixelBuffer> {
    thread_local! {
        static CACHE: RefCell<HashMap<usize, CVPixelBuffer>> = RefCell::new(HashMap::new());
    }
    let radius = radius.max(1);
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(buffer) = cache.get(&radius) {
            return Some(buffer.clone());
        }
        let buffer = CVPixelBuffer::new(kCVPixelFormatType_OneComponent8, radius, 1, None).ok()?;
        cache.insert(radius, buffer.clone());
        Some(buffer)
    })
}
