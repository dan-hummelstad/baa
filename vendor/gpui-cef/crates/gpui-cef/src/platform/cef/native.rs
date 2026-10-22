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
//! as before. AppKit sends clicks and hover over a page straight to Chromium,
//! unless gpui's own hit test has something occluding above it there (see
//! `route_mouse_to_page`); scrolls still arrive at gpui's view, which hands
//! them on (`forward_scroll`). Keyboard and IME go to Chromium natively, as first
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
    placed: std::cell::Cell<Option<(CGRect, CGRect)>>,
    /// What the app asked for; layout never shows a page the app hid.
    visible: std::cell::Cell<bool>,
    /// Kept above the other pages' views (an overlay like Peek). Native views
    /// stack in insertion order, not in gpui's paint order.
    floating: std::cell::Cell<bool>,
    gpui_view: std::cell::Cell<*mut AnyObject>,
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
            if !gpui_view.is_null() {
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
            }
        }
    }

    pub(crate) fn set_corner_radius(&self, radius: f32) {
        unsafe {
            let layer: *mut AnyObject = msg_send![self.view, layer];
            let _: () = msg_send![layer, setCornerRadius: radius as f64];
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
    pub(crate) fn place(&self, page: *mut AnyObject, bounds: Bounds<Pixels>, visible: Bounds<Pixels>) {
        unsafe {
            let content: *mut AnyObject = msg_send![self.view, superview];
            if content.is_null() {
                return;
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
            if self.placed.get() != Some((clip, inner)) || page_frame(page) != Some(inner) {
                self.placed.set(Some((clip, inner)));
                let _: () = msg_send![self.view, setFrame: clip];
                if !page.is_null() {
                    let _: () = msg_send![page, setFrame: inner];
                }
            }
            if self.visible.get() {
                if hidden.as_bool() {
                    let _: () = msg_send![self.view, setHidden: Bool::NO];
                }
                set_view_hidden(page, false);
            }
        }
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

impl Drop for Container {
    fn drop(&mut self) {
        PAGES.with(|pages| pages.borrow_mut().remove(&(self.view as usize)));
        unsafe {
            let _: () = msg_send![self.view, removeFromSuperview];
            let _: () = msg_send![self.view, release];
        }
    }
}

/// A page container's gpui view, and whether gpui's own hit test puts the page
/// frontmost under the mouse.
type Page = (usize, Box<dyn Fn() -> bool>);

thread_local! {
    static PAGES: RefCell<HashMap<usize, Page>> = RefCell::new(HashMap::new());
}

/// Lets AppKit deliver mouse events over `container` straight to the page,
/// whenever `frontmost` says gpui draws nothing interactive above it there.
/// Chromium ignores mouse events that a window hit test does not land on its
/// own view, so they cannot be forwarded from gpui instead.
pub(crate) fn route_mouse_to_page(
    gpui_view: *mut AnyObject,
    container: &Container,
    frontmost: impl Fn() -> bool + 'static,
) {
    if gpui_view.is_null() {
        return;
    }
    install_hit_test(gpui_view);
    PAGES.with(|pages| {
        pages
            .borrow_mut()
            .insert(container.view as usize, (gpui_view as usize, Box::new(frontmost)))
    });
}

/// gpui's view answers hit tests with nil over a frontmost page, so the page
/// view under it gets the event.
// ponytail: adds hitTest: to gpui's view class. Ceiling: breaks if gpui defines
// its own hitTest:. Upgrade: a pass-through region API in gpui.
fn install_hit_test(gpui_view: *mut AnyObject) {
    thread_local!(static INSTALLED: Cell<bool> = const { Cell::new(false) });
    if INSTALLED.with(|i| i.replace(true)) {
        return;
    }
    unsafe extern "C-unwind" fn hit_test(this: *mut AnyObject, _: Sel, point: CGPoint) -> *mut AnyObject {
        if unsafe { page_frontmost_at(this, point) } {
            return std::ptr::null_mut();
        }
        unsafe {
            let this = &*this;
            msg_send![super(this, class!(NSView)), hitTest: point]
        }
    }
    unsafe {
        let class = objc2::ffi::object_getClass(gpui_view) as *mut AnyClass;
        type HitTest = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, CGPoint) -> *mut AnyObject;
        objc2::ffi::class_addMethod(
            class,
            sel!(hitTest:),
            std::mem::transmute::<HitTest, Imp>(hit_test),
            c"@@:{CGPoint=dd}".as_ptr(),
        );
    }
}

/// `point` is in the superview's coordinates, like the containers' frames.
unsafe fn page_frontmost_at(gpui_view: *mut AnyObject, point: CGPoint) -> bool {
    PAGES.with(|pages| {
        pages.borrow().iter().any(|(&container, (view, frontmost))| {
            if *view != gpui_view as usize {
                return false;
            }
            let container = container as *mut AnyObject;
            let hidden: Bool = unsafe { msg_send![container, isHiddenOrHasHiddenAncestor] };
            let frame: CGRect = unsafe { msg_send![container, frame] };
            !hidden.as_bool()
                && point.x >= frame.origin.x
                && point.x < frame.origin.x + frame.size.width
                && point.y >= frame.origin.y
                && point.y < frame.origin.y + frame.size.height
                && frontmost()
        })
    })
}

thread_local! {
    /// The scroll NSEvent gpui's view is handling right now.
    static SCROLL_EVENT: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) };
    static GPUI_SCROLL_WHEEL: Cell<Option<Imp>> = const { Cell::new(None) };
}

/// AppKit sends scroll events to gpui's view even where its hit test passes
/// the point through, so gpui's view records each one for the page element to
/// hand on (see [`forward_scroll`]). Idempotent.
// ponytail: replaces scrollWheel: on gpui's view class. Ceiling: breaks if gpui
// stops implementing it on its NSView. Upgrade: an event hook in gpui.
pub(crate) fn record_scroll_events(gpui_view: *mut AnyObject) {
    if gpui_view.is_null() || GPUI_SCROLL_WHEEL.with(|o| o.get().is_some()) {
        return;
    }
    unsafe extern "C-unwind" fn scroll_wheel(this: *mut AnyObject, cmd: Sel, event: *mut AnyObject) {
        let previous = SCROLL_EVENT.with(|e| e.replace(event));
        if let Some(original) = GPUI_SCROLL_WHEEL.with(Cell::get) {
            type Handler = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject);
            let original: Handler = unsafe { std::mem::transmute::<Imp, Handler>(original) };
            unsafe { original(this, cmd, event) };
        }
        SCROLL_EVENT.with(|e| e.set(previous));
    }
    unsafe {
        let class = objc2::ffi::object_getClass(gpui_view) as *mut AnyClass;
        let Some(method) = (*class).instance_method(sel!(scrollWheel:)) else {
            return;
        };
        GPUI_SCROLL_WHEEL.with(|o| o.set(Some(method.implementation())));
        type Handler = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject);
        objc2::ffi::class_replaceMethod(
            class,
            sel!(scrollWheel:),
            std::mem::transmute::<Handler, Imp>(scroll_wheel),
            c"v@:@".as_ptr(),
        );
    }
}

/// Hands the scroll event gpui is handling to the view of `page` under it, so
/// Chromium gets the original event with its phases and momentum.
pub(crate) fn forward_scroll(page: *mut AnyObject) {
    let event = SCROLL_EVENT.with(Cell::get);
    if page.is_null() || event.is_null() {
        return;
    }
    unsafe {
        let in_window: CGPoint = msg_send![event, locationInWindow];
        let parent: *mut AnyObject = msg_send![page, superview];
        let point: CGPoint =
            msg_send![parent, convertPoint: in_window, fromView: std::ptr::null_mut::<AnyObject>()];
        let hit: *mut AnyObject = msg_send![page, hitTest: point];
        let target = if hit.is_null() { page } else { hit };
        let _: () = msg_send![target, scrollWheel: event];
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
/// with `radius` (device pixels) rounded corners, instead of drawing an image.
pub(crate) fn punch_buffer(radius: usize) -> Option<CVPixelBuffer> {
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
