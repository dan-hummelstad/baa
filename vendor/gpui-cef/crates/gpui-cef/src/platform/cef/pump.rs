//! The timer that drives CEF's message loop.
//!
//! # Why not gpui's executor
//!
//! The first attempt ran the pump on `cx.spawn` plus
//! `background_executor().timer()`. When the app is idle, gpui's foreground
//! executor barely makes progress — measured at one tick every one to five
//! seconds. CEF only advances here, so at that rate a page never even finishes
//! loading.
//!
//! Worse, the dependency is circular: the pump does not run, so CEF produces no
//! frames, so gpui does not repaint, so the executor does not run, so the pump
//! does not run.
//!
//! So this bypasses gpui and installs a `CFRunLoopTimer` directly on the main
//! thread's run loop. It is registered in `kCFRunLoopCommonModes`, so it keeps
//! ticking while the run loop is nested — during a window resize, for instance.
//!
//! Because the callback comes straight from the run loop, nobody is borrowing
//! gpui's `App` at that moment. That matters: `do_message_loop_work()` runs
//! Chromium tasks, and gpui's paint callbacks can re-enter from inside them.
//!
//! # One timer for the whole process
//!
//! `do_message_loop_work()` drives all of CEF, not one browser, so there is a
//! single timer per process and webviews register with it. A timer per webview
//! would pump CEF once per webview per tick, which is pure waste as soon as an
//! app shows more than one.

use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    rc::Rc,
};

use core_foundation::{
    date::CFAbsoluteTimeGetCurrent,
    runloop::{kCFRunLoopCommonModes, CFRunLoop, CFRunLoopTimer, CFRunLoopTimerContext},
};
use core_foundation::base::TCFType as _;
use gpui::{AsyncApp, WeakEntity};

use super::{shared::Shared, Webview};

/// Fallback tick when no display link is available (macOS < 14), in seconds.
const INTERVAL: f64 = 1. / 120.;

thread_local! {
    /// The one timer, created on first use and left running for the life of the
    /// process. CEF still needs pumping for its own housekeeping after the last
    /// webview goes away.
    static PUMP: RefCell<Option<Pump>> = const { RefCell::new(None) };
}

#[derive(Clone)]
struct View {
    id: u64,
    shared: Rc<Shared>,
    webview: WeakEntity<Webview>,
    cx: AsyncApp,
}

#[derive(Default)]
struct PumpState {
    next_id: Cell<u64>,
    views: RefCell<Vec<View>>,
}

struct Pump {
    // The timer is never removed, but keep it so its lifetime is explicit.
    _timer: Option<CFRunLoopTimer>,
    // What the timer's context points at. Has to outlive the timer.
    state: Rc<PumpState>,
}

/// Keeps one webview registered with the pump; unregisters on drop.
pub(crate) struct Registration {
    id: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let id = self.id;
        PUMP.with(|slot| {
            // The pump never borrows this slot from inside the tick, but stay
            // defensive: failing to unregister is better than a panic on drop.
            if let Ok(slot) = slot.try_borrow() {
                if let Some(pump) = slot.as_ref() {
                    if let Ok(mut views) = pump.state.views.try_borrow_mut() {
                        views.retain(|view| view.id != id);
                    }
                }
            }
        });
    }
}

/// Registers a webview with the process-wide pump, starting it if needed.
pub(crate) fn register(
    shared: Rc<Shared>,
    webview: WeakEntity<Webview>,
    cx: AsyncApp,
) -> Registration {
    PUMP.with(|slot| {
        let mut slot = slot.borrow_mut();
        let pump = slot.get_or_insert_with(Pump::start);

        let id = pump.state.next_id.get();
        pump.state.next_id.set(id + 1);
        pump.state.views.borrow_mut().push(View {
            id,
            shared,
            webview,
            cx,
        });

        Registration { id }
    })
}

impl Pump {
    fn start() -> Self {
        let state = Rc::new(PumpState::default());
        if display_link::start() {
            return Self { _timer: None, state };
        }

        let mut context = CFRunLoopTimerContext {
            version: 0,
            info: Rc::as_ptr(&state) as *mut c_void,
            retain: None,
            release: None,
            copyDescription: None,
        };

        let now = unsafe { CFAbsoluteTimeGetCurrent() };
        let timer = CFRunLoopTimer::new(now + INTERVAL, INTERVAL, 0, 0, on_tick, &mut context);

        unsafe {
            CFRunLoop::get_main().add_timer(&timer, kCFRunLoopCommonModes);
        }

        Self {
            _timer: Some(timer),
            state,
        }
    }
}

/// Ticks the pump once per display refresh (120 Hz on ProMotion), in phase with
/// vsync, so wheel input and repaints land on display frames.
pub(crate) mod display_link {
    use objc2::{
        class, msg_send,
        runtime::{AnyObject, ClassBuilder, NSObject, Sel},
        sel, ClassType as _,
    };

    extern "C-unwind" fn tick(_this: *mut AnyObject, _sel: Sel, _link: *mut AnyObject) {
        // Clone the state out so ticking (which may register webviews) does not
        // hold the PUMP borrow.
        let state = super::PUMP.with(|slot| slot.try_borrow().ok()?.as_ref().map(|p| p.state.clone()));
        if let Some(state) = state {
            super::tick_all(&state);
        }
    }

    /// The main screen's highest refresh rate (120 on ProMotion), 60 if unknown.
    pub(crate) fn max_refresh_rate() -> i32 {
        unsafe {
            let screen: *mut AnyObject = msg_send![class!(NSScreen), mainScreen];
            if screen.is_null() {
                return 60;
            }
            let fps: isize = msg_send![screen, maximumFramesPerSecond];
            if fps > 0 { fps as i32 } else { 60 }
        }
    }

    /// `[NSScreen displayLinkWithTarget:selector:]` on the main run loop. False
    /// before macOS 14, where the caller falls back to a timer.
    // ponytail: tied to the main screen. Ceiling: a window on a 60 Hz external
    // display still ticks at the main screen's rate. Upgrade: NSView's display link.
    pub(super) fn start() -> bool {
        unsafe {
            let screen: *mut AnyObject = msg_send![class!(NSScreen), mainScreen];
            if screen.is_null() {
                return false;
            }
            let supported: bool =
                msg_send![screen, respondsToSelector: sel!(displayLinkWithTarget:selector:)];
            if !supported {
                return false;
            }
            let Some(mut builder) = ClassBuilder::new(c"GpuiCefPumpTarget", NSObject::class()) else {
                return false;
            };
            builder.add_method(
                sel!(tick:),
                tick as extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
            let target_class = builder.register();
            // Lives for the process: the run loop retains the link, the link its target.
            let target: *mut AnyObject = msg_send![target_class, new];
            let link: *mut AnyObject =
                msg_send![screen, displayLinkWithTarget: target, selector: sel!(tick:)];
            if link.is_null() {
                return false;
            }
            let run_loop: *mut AnyObject = msg_send![class!(NSRunLoop), mainRunLoop];
            // kCFRunLoopCommonModes is toll-free bridged to NSRunLoopCommonModes.
            let mode = core_foundation::runloop::kCFRunLoopCommonModes as *const AnyObject;
            let _: () = msg_send![link, addToRunLoop: run_loop, forMode: mode];
            true
        }
    }
}

extern "C" fn on_tick(_timer: core_foundation::runloop::CFRunLoopTimerRef, info: *mut c_void) {
    if info.is_null() {
        return;
    }
    tick_all(unsafe { &*(info as *const PumpState) });
}

thread_local! {
    /// Chromium can spin a nested run loop inside `do_message_loop_work`; a
    /// scheduled pass must not re-enter it.
    static IN_WORK: Cell<bool> = const { Cell::new(false) };
}

/// One pending out-of-band pass at most, however often CEF asks.
static WORK_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn do_work() {
    if IN_WORK.with(|flag| flag.replace(true)) {
        return;
    }
    cef::do_message_loop_work();
    IN_WORK.with(|flag| flag.set(false));
}

/// Runs `do_message_loop_work` on the main run loop after `delay_ms`, when that is
/// sooner than the next tick would be. Callable from any thread.
pub(crate) fn schedule_work(delay_ms: i64) {
    use std::sync::atomic::Ordering;
    let delay = (delay_ms.max(0) as f64) / 1000.;
    if delay >= INTERVAL || WORK_PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    extern "C" fn fire(_timer: core_foundation::runloop::CFRunLoopTimerRef, _info: *mut c_void) {
        WORK_PENDING.store(false, std::sync::atomic::Ordering::Release);
        do_work();
    }
    let mut context = CFRunLoopTimerContext {
        version: 0,
        info: std::ptr::null_mut(),
        retain: None,
        release: None,
        copyDescription: None,
    };
    let now = unsafe { CFAbsoluteTimeGetCurrent() };
    // One-shot; the run loop releases it after firing.
    let timer = CFRunLoopTimer::new(now + delay, 0., 0, 0, fire, &mut context);
    let main = CFRunLoop::get_main();
    unsafe {
        main.add_timer(&timer, kCFRunLoopCommonModes);
        core_foundation::runloop::CFRunLoopWakeUp(main.as_concrete_TypeRef());
    }
}

fn tick_all(state: &PumpState) {
    // Driving CEF here, with gpui's App unborrowed, is the whole point. Calling
    // this while the App is borrowed lets gpui's paint re-enter from the run
    // loop Chromium spins, which panics with "RefCell already borrowed".
    do_work();

    // Snapshot, because the updates below run application code that may add or
    // remove webviews.
    let views = match state.views.try_borrow() {
        Ok(views) => views.clone(),
        Err(_) => return,
    };

    for view in views {
        tick_view(&view);
    }
}

fn tick_view(view: &View) {
    if view.shared.take_resized() {
        if let Some(host) = view.shared.host() {
            use cef::ImplBrowserHost as _;
            host.was_resized();
        }
    }

    if let Some((at, dx, dy)) = view.shared.take_scroll() {
        if let Some(host) = view.shared.host() {
            use cef::ImplBrowserHost as _;
            host.send_mouse_wheel_event(Some(&at), dx, dy);
        }
    }


    let stale = view.shared.take_stale_cpu_frames();
    let events = view.shared.take_events();
    let dirty = view.shared.take_dirty();
    if stale.is_empty() && !dirty {
        return;
    }

    let mut cx = view.cx.clone();
    let _ = view.webview.update(&mut cx, |_, cx| {
        // Release the textures the CPU path is done with.
        for frame in stale {
            cx.drop_image(frame, None);
        }
        for event in events {
            cx.emit(event);
        }
        if dirty {
            cx.notify();
        }
    });
}
