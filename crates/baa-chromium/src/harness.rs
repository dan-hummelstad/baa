//! Harness-only helpers: edit commands and synthetic input. The page view
//! machinery is baa-web's `native.rs`, shared through `#[path]`.

use objc2::{
    class, msg_send,
    runtime::{AnyObject, Bool},
    sel,
};

use crate::native::{CGPoint, CGRect};

pub const EDIT_COPY: std::ffi::c_int = 0;
pub const EDIT_CUT: std::ffi::c_int = 1;
pub const EDIT_PASTE: std::ffi::c_int = 2;
pub const EDIT_SELECT_ALL: std::ffi::c_int = 3;
pub const EDIT_UNDO: std::ffi::c_int = 4;
pub const EDIT_REDO: std::ffi::c_int = 5;

/// ⌘C/⌘X/⌘V/⌘A/⌘Z/⇧⌘Z as a BaaCore edit command; None for any other key.
pub fn edit_command(event: *mut AnyObject) -> Option<std::ffi::c_int> {
    // NSEventModifierFlags: shift, control, option, command.
    const SHIFT: usize = 1 << 17;
    const OTHERS: usize = (1 << 18) | (1 << 19);
    const COMMAND: usize = 1 << 20;
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
