//! Sparkle, which release engines carry (dev engines do not). It checks the appcast in
//! Info.plist (SUFeedURL, set by xtask; hourly per SUScheduledCheckInterval), downloads a signed
//! update in the background and installs it when baa quits, as Chrome does. Helium builds Sparkle
//! without its UI, so the driver is the headless one and the sidebar's card (baa-ui `update.rs`)
//! is what the user sees.

use std::cell::RefCell;

use block2::{Block, RcBlock};
use futures::channel::mpsc;
use objc2::{
    class, define_class, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool, NSObject},
    ClassType,
};

thread_local! {
    /// Sparkle's "install it now and start again" for the update it holds ready.
    static INSTALL: RefCell<Option<RcBlock<dyn Fn()>>> = const { RefCell::new(None) };
    static READY: RefCell<Option<mpsc::UnboundedSender<()>>> = const { RefCell::new(None) };
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `Delegate` has no ivars and no Drop.
    #[unsafe(super(NSObject))]
    #[name = "BaaUpdaterDelegate"]
    struct Delegate;

    impl Delegate {
        /// SPUUpdaterDelegate: an update is downloaded and will install on quit. Answering YES
        /// hands over the block that installs it sooner; quitting installs it either way.
        #[unsafe(method(updater:willInstallUpdateOnQuit:immediateInstallationBlock:))]
        fn will_install_on_quit(&self, _updater: *mut AnyObject, _item: *mut AnyObject, install: &Block<dyn Fn()>) -> Bool {
            INSTALL.with(|i| *i.borrow_mut() = Some(install.copy()));
            READY.with(|r| {
                if let Some(ready) = r.borrow().as_ref() {
                    ready.unbounded_send(()).ok();
                }
            });
            Bool::YES
        }
    }
);

pub struct Started {
    /// What the sidebar's card and Settings call.
    pub ui: baa_ui::Updater,
    /// One message for each update that is downloaded and ready.
    pub ready: mpsc::UnboundedReceiver<()>,
    /// One message for each "Restart and Update"; answer with [`install`] outside any gpui update.
    pub install: mpsc::UnboundedReceiver<()>,
}

/// Starts the updater. `None` where the engine has no Sparkle, or Sparkle would not start.
pub fn start() -> Option<Started> {
    let (updater, driver) = (AnyClass::get(c"SPUUpdater")?, AnyClass::get(c"SPUHeadlessUserDriver")?);
    let (ready_tx, ready) = mpsc::unbounded();
    let (install_tx, install) = mpsc::unbounded();
    READY.with(|r| *r.borrow_mut() = Some(ready_tx));
    let none: *mut AnyObject = std::ptr::null_mut();
    let updater = unsafe {
        let bundle: *mut AnyObject = msg_send![class!(NSBundle), mainBundle];
        let driver: *mut AnyObject = msg_send![driver, alloc];
        let driver: *mut AnyObject = msg_send![driver, init];
        let delegate: Retained<Delegate> = msg_send![Delegate::class(), new];
        let updater: *mut AnyObject = msg_send![updater, alloc];
        let updater: *mut AnyObject =
            msg_send![updater, initWithHostBundle: bundle, applicationBundle: bundle, userDriver: driver, delegate: &*delegate];
        // ponytail: never released; the updater, which holds its delegate weakly, lives as long as the app.
        std::mem::forget(delegate);
        let updater = Retained::from_raw(updater)?;
        // The headless driver cannot offer a found update, so nothing would ever install without this.
        let _: () = msg_send![&*updater, setAutomaticallyDownloadsUpdates: Bool::YES];
        let mut error: *mut AnyObject = std::ptr::null_mut();
        let started: Bool = msg_send![&*updater, startUpdater: &mut error];
        if !started.as_bool() {
            let text: *mut AnyObject = if error.is_null() { none } else { msg_send![error, localizedDescription] };
            let text: *const std::ffi::c_char = if text.is_null() { std::ptr::null() } else { msg_send![text, UTF8String] };
            let text = if text.is_null() { "unknown error".into() } else { std::ffi::CStr::from_ptr(text).to_string_lossy() };
            log::warn!("updates: Sparkle did not start: {text}");
            return None;
        }
        updater
    };
    let (get, set) = (updater.clone(), updater);
    Some(Started {
        ui: baa_ui::Updater {
            // Sparkle keeps this choice in the app's user defaults.
            automatic: Box::new(move || {
                let on: Bool = unsafe { msg_send![&*get, automaticallyChecksForUpdates] };
                on.as_bool()
            }),
            set_automatic: Box::new(move |on| unsafe {
                let _: () = msg_send![&*set, setAutomaticallyChecksForUpdates: Bool::new(on)];
            }),
            install: Box::new(move || {
                install_tx.unbounded_send(()).ok();
            }),
        },
        ready,
        install,
    })
}

/// Installs the update that is ready and starts baa again. It quits the app, so it must not
/// run inside a gpui update.
pub fn install() {
    let block = INSTALL.with(|i| i.borrow().clone());
    if let Some(block) = block {
        block.call(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    /// Sparkle's side of the delegate, played by hand: test builds have no Sparkle.
    #[test]
    fn a_downloaded_update_is_announced_and_its_install_block_kept_for_the_card() {
        assert!(start().is_none(), "no Sparkle here, so no updater and no card");
        let (tx, mut ready) = mpsc::unbounded();
        READY.with(|r| *r.borrow_mut() = Some(tx));
        let ran = Rc::new(Cell::new(0));
        let block = RcBlock::new({
            let ran = ran.clone();
            move || ran.set(ran.get() + 1)
        });
        install();
        assert_eq!(ran.get(), 0, "nothing to install before Sparkle says so");

        let none: *mut AnyObject = std::ptr::null_mut();
        let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::class(), new] };
        let taken: Bool =
            unsafe { msg_send![&*delegate, updater: none, willInstallUpdateOnQuit: none, immediateInstallationBlock: &*block] };
        assert!(taken.as_bool(), "baa takes the handler, so Sparkle waits for the card or the quit");
        assert!(ready.try_recv().is_ok(), "the window hears of it");
        assert_eq!(ran.get(), 0, "and nothing installs until asked");
        install();
        assert_eq!(ran.get(), 1);
    }
}
