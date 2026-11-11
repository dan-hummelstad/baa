//! A started download is thrown from the pointer into the Library button, or into the
//! bin while the sidebar is hidden.

use baa_model::Store;
use baa_web::PageEvent;
use gpui::{point, px, Entity, Modifiers, TestAppContext, VisualTestContext};

use crate::{actions::ToggleSidebar, mock::MockHost, motion::Feel, Browser};

const HOP: crate::motion::Key = ("download-hop", 1);

fn browser(cx: &mut TestAppContext) -> (Entity<Browser<MockHost>>, &mut VisualTestContext) {
    cx.update(crate::init);
    let (b, cx) = cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx));
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("a.example");
    cx.simulate_keystrokes("enter");
    cx.simulate_mouse_move(point(px(600.), px(300.)), None, Modifiers::none());
    (b, cx)
}

fn start_download(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) {
    let page = b.read_with(cx, |b, _| b.pages.get(&b.active_tab().unwrap()).unwrap().0.clone());
    page.update(cx, |_, cx| cx.emit(PageEvent::DownloadStarted));
    cx.run_until_parked();
}

/// Frames only draw when asked under test.
fn frame(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(std::time::Duration::from_millis(33));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn frames_until_landed(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) {
    for _ in 0..200 {
        if b.read_with(cx, |b, _| b.tosses.is_empty()) {
            return;
        }
        frame(cx);
    }
    panic!("the icon never landed");
}

#[gpui::test]
fn a_started_download_is_thrown_into_the_library_button(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    start_download(&b, cx);
    b.read_with(cx, |b, _| assert_eq!(b.tosses.len(), 1));
    frame(cx);
    b.read_with(cx, |b, _| {
        assert_eq!(b.tosses.len(), 1, "still in the air");
        assert_eq!(b.bin.target(), 0., "the sidebar is there: no bin");
        assert_eq!(b.motion.get(HOP, Feel::Glide), 0.);
    });
    frames_until_landed(&b, cx);
    b.read_with(cx, |b, _| assert!(b.motion.get(HOP, Feel::Glide) > 0., "the button hops as the icon sinks in"));
}

#[gpui::test]
fn with_the_sidebar_hidden_it_lands_in_a_bin_that_leaves_again(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    cx.dispatch_action(ToggleSidebar);
    cx.run_until_parked();
    start_download(&b, cx);
    frame(cx);
    b.read_with(cx, |b, _| {
        assert_eq!(b.tosses.len(), 1);
        assert_eq!(b.bin.target(), 1.);
    });
    frames_until_landed(&b, cx);
    b.read_with(cx, |b, _| {
        assert_eq!(b.bin.target(), 0.);
        assert!(b.bin.value() <= 0.01, "the icon left with the bin");
        assert_eq!(b.motion.get(HOP, Feel::Glide), 0., "nothing sank into the button");
    });
}

#[gpui::test]
fn reduce_motion_throws_nothing(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    cx.update(|_, cx| cx.set_reduce_motion(true));
    start_download(&b, cx);
    b.read_with(cx, |b, _| assert!(b.tosses.is_empty()));
}
