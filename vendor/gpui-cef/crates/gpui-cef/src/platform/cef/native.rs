//! The page as a native view under gpui's.
//!
//! CEF renders each browser into its own NSView, the way Chrome does, so
//! scrolling, video and animation run on Chromium's own display link at the
//! screen's refresh rate. Off-screen rendering capped input-driven frames at
//! 60 Hz (viz pins the frame interval to an off-screen view's fallback rate).
//!
//! Layering: the window's content view holds one clipping container per page,
//! ordered *below* gpui's view. gpui's layer is transparent where a page element
//! punches a hole (see `punch_buffer`), so gpui draws over and around the page
//! as before. gpui's view keeps every mouse event and hit-tests with its own
//! occlusion; the page element hands the original NSEvent to Chromium's view
//! (see `install_mouse_forwarding`). Keyboard and IME go to Chromium natively, as first
//! responder; its shortcuts get offered to gpui first (`offer_key_equivalent`).

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
/// NSViewWidthSizable | NSViewHeightSizable: margins fixed, size follows.
const FILL: usize = 2 | 16;
/// NSViewMinYMargin: in an unflipped superview, keeps the top-left corner put.
const TOP_LEFT: usize = 8;
/// Shortest time between two sizes for a page during a resize. Each new size
/// costs the page a full style and layout pass (tens of ms on a big site);
/// sent faster than that they queue up, and the page crawls through every
/// stale size after the resize has ended.
const RESIZE_INTERVAL: f64 = 0.04;

/// gpui's NSView for `window`.
pub(crate) fn gpui_view(window: &Window) -> *mut AnyObject {
    match HasWindowHandle::window_handle(window).map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::AppKit(handle)) => handle.ns_view.as_ptr().cast(),
        _ => std::ptr::null_mut(),
    }
}

/// A layer-backed view that clips the page to the element's visible rect and
/// rounds its corners. Owns one reference; removed from the window on drop.
pub(crate) struct Container {
    view: *mut AnyObject,
    placed: std::cell::Cell<Option<CGRect>>,
    /// What the app asked for; layout never shows a page the app hid.
    visible: std::cell::Cell<bool>,
    /// Kept above the other pages' views (an overlay like Peek). Native views
    /// stack in insertion order, not in gpui's paint order.
    floating: std::cell::Cell<bool>,
    gpui_view: std::cell::Cell<*mut AnyObject>,
    /// When the page last got a new size.
    resized_at: std::cell::Cell<f64>,
}

impl Container {
    /// Hidden until the first [`Container::place`].
    pub(crate) fn new(gpui_view: *mut AnyObject, corner_radius: f32) -> Self {
        unsafe {
            let view: *mut AnyObject = msg_send![class!(NSView), alloc];
            let zero = CGRect {
                origin: CGPoint { x: 0., y: 0. },
                size: CGSize { width: 1., height: 1. },
            };
            let view: *mut AnyObject = msg_send![view, initWithFrame: zero];
            let _: () = msg_send![view, setWantsLayer: Bool::YES];
            let layer: *mut AnyObject = msg_send![view, layer];
            let _: () = msg_send![layer, setCornerRadius: corner_radius as f64];
            let _: () = msg_send![layer, setMasksToBounds: Bool::YES];
            let _: () = msg_send![view, setHidden: Bool::YES];
            // During a live resize AppKit resizes the window's views at once,
            // while gpui only re-lays out the page on its next frame: follow
            // the window here, gpui sets the exact frame when it draws.
            let _: () = msg_send![view, setAutoresizingMask: FILL];
            if !gpui_view.is_null() {
                // Behind the page holes: white, like a page, rather than the
                // desktop through gpui's transparent layer while a tab switches.
                let window: *mut AnyObject = msg_send![gpui_view, window];
                let white: *mut AnyObject = msg_send![class!(NSColor), whiteColor];
                let _: () = msg_send![window, setBackgroundColor: white];
                let content: *mut AnyObject = msg_send![gpui_view, superview];
                let _: () = msg_send![
                    content,
                    addSubview: view,
                    positioned: NS_WINDOW_BELOW,
                    relativeTo: gpui_view
                ];
            }
            Self {
                view,
                placed: Default::default(),
                visible: std::cell::Cell::new(true),
                floating: std::cell::Cell::new(false),
                gpui_view: std::cell::Cell::new(gpui_view),
                resized_at: std::cell::Cell::new(0.),
            }
        }
    }

    pub(crate) fn set_corner_radius(&self, radius: f32) {
        unsafe {
            let layer: *mut AnyObject = msg_send![self.view, layer];
            let _: () = msg_send![layer, setCornerRadius: radius as f64];
        }
    }

    /// Leaves the `square` corners (top left 1, top right 2, bottom left 4,
    /// bottom right 8) unrounded.
    pub(crate) fn set_square_corners(&self, square: u8) {
        // CACornerMask: the view is not flipped, so "min Y" is the bottom.
        const ALL: usize = 1 | 2 | 4 | 8;
        let mut rounded = ALL;
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
    pub(crate) fn move_below(&self, gpui_view: *mut AnyObject) {
        if gpui_view.is_null() {
            return;
        }
        unsafe {
            let content: *mut AnyObject = msg_send![gpui_view, superview];
            let _: () = msg_send![self.view, removeFromSuperview];
            let _: () = msg_send![
                content,
                addSubview: self.view,
                positioned: NS_WINDOW_BELOW,
                relativeTo: gpui_view
            ];
        }
        self.placed.set(None);
        self.gpui_view.set(gpui_view);
    }

    pub(crate) fn set_floating(&self, floating: bool) {
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
            let count: usize = msg_send![subviews, count];
            let top: *mut AnyObject = match count {
                0 | 1 => return,
                n => msg_send![subviews, objectAtIndex: n - 2],
            };
            if top != self.view {
                let _: () = msg_send![
                    content,
                    addSubview: self.view,
                    positioned: NS_WINDOW_BELOW,
                    relativeTo: gpui_view
                ];
            }
        }
    }

    pub(crate) fn as_ptr(&self) -> *mut AnyObject {
        self.view
    }

    /// Puts the container at `visible` (the element's rect clipped by its
    /// content mask) and the page at `bounds`, both in gpui window coordinates.
    /// True when the page's new size is held back (see [`RESIZE_INTERVAL`]):
    /// call again on a later frame.
    pub(crate) fn place(&self, page: *mut AnyObject, bounds: Bounds<Pixels>, visible: Bounds<Pixels>) -> bool {
        let mut pending = false;
        unsafe {
            let content: *mut AnyObject = msg_send![self.view, superview];
            if content.is_null() {
                return false;
            }
            // The content view is not flipped: y grows upwards from the bottom.
            let content_frame: CGRect = msg_send![content, bounds];
            let height = content_frame.size.height;
            let clip = CGRect {
                origin: CGPoint {
                    x: f32::from(visible.origin.x) as f64,
                    y: height - f32::from(visible.bottom()) as f64,
                },
                size: CGSize {
                    width: f32::from(visible.size.width) as f64,
                    height: f32::from(visible.size.height) as f64,
                },
            };
            let inner = CGRect {
                origin: CGPoint {
                    x: f32::from(bounds.origin.x - visible.origin.x) as f64,
                    y: f32::from(visible.bottom() - bounds.bottom()) as f64,
                },
                size: CGSize {
                    width: f32::from(bounds.size.width) as f64,
                    height: f32::from(bounds.size.height) as f64,
                },
            };
            if self.floating.get() {
                self.raise();
            }
            let hidden: Bool = msg_send![self.view, isHidden];
            if self.placed.get() != Some(clip) {
                self.placed.set(Some(clip));
                let _: () = msg_send![self.view, setFrame: clip];
            }
            let current = page_frame(page);
            if !page.is_null() && current != Some(inner) {
                let now = core_foundation::date::CFAbsoluteTimeGetCurrent();
                let resized = current.is_some_and(|c| c.size != inner.size);
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
            if self.visible.get() {
                if hidden.as_bool() {
                    let _: () = msg_send![self.view, setHidden: Bool::NO];
                }
                set_view_hidden(page, false);
            }
        }
        pending
    }

    /// Chromium only notices visibility changes, and a page created behind a
    /// hidden container starts out believing it is visible: show it a hide.
    /// AppKit reports only effective changes, so the container is unhidden for
    /// the toggle too; all in one run loop turn, so nothing reaches the screen.
    pub(crate) fn page_created(&self, page: *mut AnyObject) {
        if !self.visible.get() && !page.is_null() {
            self.set_hidden(false);
            set_view_hidden(page, false);
            set_view_hidden(page, true);
            self.set_hidden(true);
        }
    }

    /// Showing waits for the next [`Container::place`].
    pub(crate) fn set_visible(&self, visible: bool, page: *mut AnyObject) {
        self.visible.set(visible);
        if !visible {
            self.set_hidden(true);
            set_view_hidden(page, true);
        }
    }

    pub(crate) fn set_hidden(&self, hidden: bool) {
        unsafe {
            let _: () = msg_send![self.view, setHidden: Bool::new(hidden)];
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

fn page_frame(page: *mut AnyObject) -> Option<CGRect> {
    (!page.is_null()).then(|| unsafe { msg_send![page, frame] })
}

/// Takes a page's view out of the window.
pub(crate) fn detach(view: *mut AnyObject) {
    if !view.is_null() {
        unsafe {
            let _: () = msg_send![view, removeFromSuperview];
        }
    }
}

impl Drop for Container {
    fn drop(&mut self) {
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
    /// gpui's own implementations, by selector.
    static ORIGINALS: RefCell<HashMap<Sel, Imp>> = RefCell::new(HashMap::new());
    /// Set while an event is being handed to a page; gpui's view then steps out
    /// of hit tests (see `install_hit_test`).
    static FORWARDING: Cell<bool> = const { Cell::new(false) };
    static GPUI_VIEW_CLASS: Cell<*const AnyClass> = const { Cell::new(std::ptr::null()) };
    static WINDOW_SEND_EVENT: Cell<Option<Imp>> = const { Cell::new(None) };
}

/// gpui's view stays in charge of the mouse: it hit-tests with its own
/// occlusion, hovers and scrolls its own UI, and the page element hands the
/// original NSEvent on. This wraps gpui's view's mouse methods so they record
/// that event, and makes the view step aside while one is being handed on.
/// Idempotent.
// ponytail: method replacement on gpui's view class. Ceiling: breaks if gpui
// stops handling these selectors on its NSView. Upgrade: an event hook in gpui.
pub(crate) fn install_mouse_forwarding(gpui_view: *mut AnyObject) {
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
/// while a page has focus; gpui would never learn where the pointer is (no
/// hover, no edge reveal). gpui's windows hand every move to gpui's view.
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
        let Some(method) = (*window_class).instance_method(sel!(sendEvent:)) else {
            return;
        };
        WINDOW_SEND_EVENT.with(|o| o.set(Some(method.implementation())));
        objc2::ffi::class_replaceMethod(
            window_class,
            sel!(sendEvent:),
            std::mem::transmute::<Handler, Imp>(send_event),
            c"v@:@".as_ptr(),
        );
    }
}

/// The gpui view among `window`'s content view's subviews.
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

/// Chromium drops a mouse event unless the window's hit test at its location
/// lands on Chromium's own view; while one is being handed on, gpui's view
/// answers nil so the test falls through to the page.
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
/// it, or to `target` (the view a drag started on). Returns the view that got it.
pub(crate) fn forward_current_event(
    page: *mut AnyObject,
    target: Option<*mut AnyObject>,
) -> Option<*mut AnyObject> {
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

/// Lets gpui's view handle `event` as a key equivalent; whether it did.
pub(crate) fn offer_key_equivalent(gpui_view: *mut AnyObject, event: *mut AnyObject) -> bool {
    if gpui_view.is_null() || event.is_null() {
        return false;
    }
    let handled: Bool = unsafe { msg_send![gpui_view, performKeyEquivalent: event] };
    handled.as_bool()
}

/// Gives `view`'s window keyboard focus to `view`.
pub(crate) fn make_first_responder(view: *mut AnyObject) {
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
pub(crate) fn has_first_responder(view: *mut AnyObject) -> bool {
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

/// The marker surface the vendored gpui renderer turns into a transparent hole
/// with `radius` (device pixels) rounded corners, instead of drawing an image;
/// `square` corners stay square (bits as in `Webview::set_square_corners`).
pub(crate) fn punch_buffer(radius: usize, square: u8) -> Option<CVPixelBuffer> {
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


/// Chrome's own dialogs (the Web Store's "Add extension?") come up as titled
/// windows of their own with an opaque titlebar over their first line,
/// centred on the screen. This clears the titlebar and puts each one at the
/// top centre of the front app window.
// ponytail: polled from the pump; a window notification would do it without polling.
pub(crate) fn place_chrome_dialogs() {
    const TITLED: usize = 1;
    const CLOSABLE_OR_RESIZABLE: usize = 2 | 8;
    // Below the app's top bar, as Chrome puts them under its toolbar.
    const TOP: f64 = 44.;
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let windows: *mut AnyObject = msg_send![app, orderedWindows];
        let n: usize = msg_send![windows, count];
        let class_of = |w: *mut AnyObject| (*w).class().name().to_string_lossy().into_owned();
        let Some(main) = (0..n)
            .map(|i| -> *mut AnyObject { msg_send![windows, objectAtIndex: i] })
            .find(|&w| class_of(w).contains("GPUIWindow"))
        else {
            return;
        };
        for i in 0..n {
            let w: *mut AnyObject = msg_send![windows, objectAtIndex: i];
            let mask: usize = msg_send![w, styleMask];
            let done: Bool = msg_send![w, titlebarAppearsTransparent];
            if !class_of(w).contains("NativeWidgetMacNSWindow")
                || mask & TITLED == 0
                || mask & CLOSABLE_OR_RESIZABLE != 0
                || done.as_bool()
            {
                continue;
            }
            let _: () = msg_send![w, setTitlebarAppearsTransparent: Bool::YES];
            let _: () = msg_send![w, setTitleVisibility: 1isize];
            let parent: CGRect = msg_send![main, frame];
            let frame: CGRect = msg_send![w, frame];
            let origin = CGPoint {
                x: parent.origin.x + (parent.size.width - frame.size.width) / 2.,
                y: parent.origin.y + parent.size.height - TOP - frame.size.height,
            };
            let _: () = msg_send![w, setFrameOrigin: origin];
        }
    }
}

/// The NSWindow behind a CEF window handle (its content view on macOS).
pub(crate) fn window_of(handle: *mut AnyObject) -> *mut AnyObject {
    if handle.is_null() {
        return handle;
    }
    unsafe {
        let is_view: Bool = msg_send![handle, isKindOfClass: class!(NSView)];
        if is_view.as_bool() {
            msg_send![handle, window]
        } else {
            handle
        }
    }
}

/// Puts `child` (a borderless window) over `bounds` of `gpui_view`'s window,
/// rounded, as a child window so it follows. Returns its screen frame.
pub(crate) fn attach_over(
    child: *mut AnyObject,
    gpui_view: *mut AnyObject,
    bounds: Bounds<Pixels>,
    corner_radius: f32,
) -> Option<[f64; 4]> {
    if child.is_null() || gpui_view.is_null() {
        return None;
    }
    unsafe {
        let parent: *mut AnyObject = msg_send![gpui_view, window];
        if parent.is_null() {
            return None;
        }
        let outer: CGRect = msg_send![parent, frame];
        let content: CGRect = msg_send![parent, contentRectForFrameRect: outer];
        let (x, y, w, h) = (
            f64::from(f32::from(bounds.origin.x)),
            f64::from(f32::from(bounds.origin.y)),
            f64::from(f32::from(bounds.size.width)),
            f64::from(f32::from(bounds.size.height)),
        );
        let frame = CGRect {
            origin: CGPoint { x: content.origin.x + x, y: content.origin.y + content.size.height - y - h },
            size: CGSize { width: w, height: h },
        };
        let _: () = msg_send![child, setFrame: frame, display: Bool::YES];
        let attached: *mut AnyObject = msg_send![child, parentWindow];
        if attached != parent {
            let _: () = msg_send![child, setOpaque: Bool::NO];
            let _: () = msg_send![child, setHasShadow: Bool::NO];
            let clear: *mut AnyObject = msg_send![class!(NSColor), clearColor];
            let _: () = msg_send![child, setBackgroundColor: clear];
            let _: () = msg_send![parent, addChildWindow: child, ordered: 1isize];
        }
        let view: *mut AnyObject = msg_send![child, contentView];
        let _: () = msg_send![view, setWantsLayer: Bool::YES];
        let layer: *mut AnyObject = msg_send![view, layer];
        if !layer.is_null() {
            let _: () = msg_send![layer, setCornerRadius: f64::from(corner_radius)];
            let _: () = msg_send![layer, setMasksToBounds: Bool::YES];
        }
        Some([frame.origin.x, frame.origin.y, w, h])
    }
}

pub(crate) fn detach_window(child: *mut AnyObject) {
    if child.is_null() {
        return;
    }
    unsafe {
        let parent: *mut AnyObject = msg_send![child, parentWindow];
        if !parent.is_null() {
            let _: () = msg_send![parent, removeChildWindow: child];
        }
    }
}


/// Gives key status back to `window` (no-op for null) on the next run loop
/// turn: not from inside the activation that took it, which Chromium's
/// observers and gpui's window state are still handling.
pub(crate) fn make_key_soon(window: *mut AnyObject) {
    if !window.is_null() {
        unsafe {
            let _: () = msg_send![
                window,
                performSelector: sel!(makeKeyWindow),
                withObject: std::ptr::null_mut::<AnyObject>(),
                afterDelay: 0f64
            ];
        }
    }
}

/// A Chrome-style tab's own window: shown, so Chromium keeps the page live,
/// but never seen or clicked. Views keeps 40pt of any window on screen and
/// resets its alpha now and then, so the pump re-applies this every tick.
pub(crate) fn keep_unseen(window: *mut AnyObject) {
    const OFF_SCREEN: f64 = -30000.;
    /// NSWindowCollectionBehaviorTransient | IgnoresCycle: out of Mission
    /// Control and ⌘`.
    const UNSEEN: usize = (1 << 3) | (1 << 6);
    if window.is_null() {
        return;
    }
    unsafe {
        allow_off_screen(window);
        let frame: CGRect = msg_send![window, frame];
        if frame.origin.x > OFF_SCREEN + 1. {
            let _: () = msg_send![window, setFrameOrigin: CGPoint { x: OFF_SCREEN, y: OFF_SCREEN }];
        }
        let alpha: f64 = msg_send![window, alphaValue];
        if alpha != 0. {
            let _: () = msg_send![window, setAlphaValue: 0f64];
            let _: () = msg_send![window, setIgnoresMouseEvents: Bool::YES];
            let _: () = msg_send![window, setExcludedFromWindowsMenu: Bool::YES];
            let _: () = msg_send![window, setCollectionBehavior: UNSEEN];
        }
    }
}

/// Moves `window`'s content view (Chrome's `BridgedContentView`, which holds
/// the page) into `container`, and returns it, retained.
pub(crate) fn adopt_content_view(window: *mut AnyObject, container: *mut AnyObject) -> *mut AnyObject {
    if window.is_null() || container.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        let view: *mut AnyObject = msg_send![window, contentView];
        if view.is_null() {
            return view;
        }
        let _: *mut AnyObject = msg_send![view, retain];
        let _: () = msg_send![container, addSubview: view];
        view
    }
}

/// Puts an adopted content view back in its window, so Chrome closes it the
/// way it opened it, and lets go of it.
pub(crate) fn restore_content_view(window: *mut AnyObject, view: *mut AnyObject) {
    if view.is_null() {
        return;
    }
    unsafe {
        let _: () = msg_send![view, removeFromSuperview];
        if !window.is_null() {
            let _: () = msg_send![window, setContentView: view];
        }
        let _: () = msg_send![view, release];
    }
}

/// The visible `RenderWidgetHostViewCocoa` under `view`: what takes the
/// page's mouse and keys. Chrome's `BridgedContentView` answers hit tests
/// itself, so events have to be aimed past it. `view` when there is none.
pub(crate) fn render_view_in(view: *mut AnyObject) -> *mut AnyObject {
    unsafe fn find(view: *mut AnyObject) -> Option<*mut AnyObject> {
        unsafe {
            let hidden: Bool = msg_send![view, isHidden];
            if hidden.as_bool() {
                return None;
            }
            if (*view).class().name().to_bytes().starts_with(b"RenderWidgetHostViewCocoa") {
                return Some(view);
            }
            let subviews: *mut AnyObject = msg_send![view, subviews];
            let count: usize = msg_send![subviews, count];
            (0..count).rev().find_map(|i| find(msg_send![subviews, objectAtIndex: i]))
        }
    }
    if view.is_null() {
        return view;
    }
    unsafe { find(view) }.unwrap_or(view)
}

/// `view`'s size in points, rounded.
pub(crate) fn view_size(view: *mut AnyObject) -> Option<(i32, i32)> {
    page_frame(view).map(|f| (f.size.width.round() as i32, f.size.height.round() as i32))
}

pub(crate) fn retained(object: *mut AnyObject) -> *mut AnyObject {
    if !object.is_null() {
        unsafe {
            let _: *mut AnyObject = msg_send![object, retain];
        }
    }
    object
}

pub(crate) fn release(object: *mut AnyObject) {
    if !object.is_null() {
        unsafe {
            let _: () = msg_send![object, release];
        }
    }
}

thread_local! {
    static UNSEEN: RefCell<std::collections::HashSet<usize>> = RefCell::default();
    /// Each window class's own `constrainFrameRect:toScreen:`, before ours.
    static CONSTRAIN: RefCell<HashMap<usize, Imp>> = RefCell::default();
}

/// Chrome's windows clamp themselves to keep a corner on screen; the windows
/// passed here (tabs', DevTools') are let go wholly off it.
// ponytail: replaces `constrainFrameRect:toScreen:` on each such window's
// class, skipping it only for those windows. Ceiling: breaks if Chrome clamps
// somewhere else.
unsafe fn allow_off_screen(window: *mut AnyObject) {
    type Constrain = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, CGRect, *mut AnyObject) -> CGRect;
    unsafe extern "C-unwind" fn constrain(this: *mut AnyObject, cmd: Sel, rect: CGRect, screen: *mut AnyObject) -> CGRect {
        if UNSEEN.with(|u| u.borrow().contains(&(this as usize))) {
            return rect;
        }
        // The nearest class whose own method this replaced.
        let mut class = unsafe { objc2::ffi::object_getClass(this) };
        while !class.is_null() {
            if let Some(original) = CONSTRAIN.with(|c| c.borrow().get(&(class as usize)).copied()) {
                return unsafe { std::mem::transmute::<Imp, Constrain>(original)(this, cmd, rect, screen) };
            }
            class = unsafe { (*class).superclass() }.map_or(std::ptr::null(), |c| c as *const AnyClass);
        }
        rect
    }
    if !UNSEEN.with(|u| u.borrow_mut().insert(window as usize)) {
        return;
    }
    unsafe {
        let mut class = objc2::ffi::object_getClass(window) as *mut AnyClass;
        // A KVO subclass is AppKit's; the method lives on its parent.
        if (*class).name().to_bytes().starts_with(b"NSKVONotifying_") {
            class = (*class).superclass().map_or(class, |c| c as *const AnyClass as *mut AnyClass);
        }
        if CONSTRAIN.with(|c| c.borrow().contains_key(&(class as usize))) {
            return;
        }
        let Some(method) = (*class).instance_method(sel!(constrainFrameRect:toScreen:)) else { return };
        CONSTRAIN.with(|c| c.borrow_mut().insert(class as usize, method.implementation()));
        objc2::ffi::class_replaceMethod(
            class,
            sel!(constrainFrameRect:toScreen:),
            std::mem::transmute::<Constrain, Imp>(constrain),
            c"{CGRect={CGPoint=dd}{CGSize=dd}}@:{CGRect={CGPoint=dd}{CGSize=dd}}@".as_ptr(),
        );
    }
}

/// Forgets a tab window passed to [`keep_unseen`], before it is released.
pub(crate) fn forget_unseen(window: *mut AnyObject) {
    UNSEEN.with(|u| u.borrow_mut().remove(&(window as usize)));
}

/// Tells Chrome `window` became key, and then that it stopped, without
/// AppKit's key window changing: Chrome records it as the last active window
/// (what extensions read as "current"), and the keyboard and gpui's window
/// state are left alone.
pub(crate) fn announce_key(window: *mut AnyObject) {
    if window.is_null() {
        return;
    }
    unsafe {
        let center: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
        for name in [c"NSWindowDidBecomeKeyNotification", c"NSWindowDidResignKeyNotification"] {
            let name: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: name.as_ptr()];
            let _: () = msg_send![center, postNotificationName: name, object: window];
        }
    }
}

/// Resizes `window`'s content to `size` points.
pub(crate) fn set_window_size(window: *mut AnyObject, size: (i32, i32)) {
    if window.is_null() {
        return;
    }
    unsafe {
        // The content, not the frame: a titled window's frame is a title bar taller.
        let size = CGSize { width: size.0.max(1) as f64, height: size.1.max(1) as f64 };
        let _: () = msg_send![window, setContentSize: size];
    }
}
