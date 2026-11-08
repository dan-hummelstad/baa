//! PLAN-m4-features C: engine-owned muted state, the mini-player, PiP for video leaving the screen.

use baa_model::{now, Store};
use baa_web::{Media, MediaAction, PageEvent};
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::{actions::MuteTab, mock::{MockHost, MockPage}, Browser};

fn browser(cx: &mut TestAppContext) -> (Entity<Browser<MockHost>>, &mut VisualTestContext) {
    cx.update(crate::init);
    cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx))
}

fn open_page(b: &Entity<Browser<MockHost>>, url: &str, cx: &mut VisualTestContext) -> (baa_model::TabId, Entity<MockPage>) {
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input(url);
    cx.simulate_keystrokes("enter");
    b.read_with(cx, |b, _| {
        let id = b.active_tab().unwrap();
        (id, b.pages.get(&id).unwrap().0.clone())
    })
}

fn playing(page: &Entity<MockPage>, cx: &mut VisualTestContext) {
    let media = Media {
        title: "Song".into(),
        playing: true,
        actions: vec![MediaAction::Pause, MediaAction::Next, MediaAction::PictureInPicture],
        ..Media::default()
    };
    page.update(cx, |_, cx| {
        cx.emit(PageEvent::Media(media));
        cx.emit(PageEvent::AudibleChanged(true));
    });
}

#[gpui::test]
fn mini_player_only_while_sound_plays(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let (a, page_a) = open_page(&b, "a.example", cx);
    let (bb, _) = open_page(&b, "b.example", cx);
    let (c, _) = open_page(&b, "c.example", cx);
    let silent = Media { title: "Muted autoplay".into(), playing: true, ..Media::default() };
    page_a.update(cx, |_, cx| cx.emit(PageEvent::Media(silent)));
    assert_eq!(mini(&b, cx), None, "playing without sound");
    playing(&page_a, cx);
    assert_eq!(mini(&b, cx), Some(a));
    page_a.update(cx, |_, cx| cx.emit(PageEvent::Muted(true)));
    page_a.update(cx, |_, cx| cx.emit(PageEvent::AudibleChanged(false)));
    assert_eq!(mini(&b, cx), Some(a), "muted by the user keeps its controls");
    page_a.update(cx, |_, cx| cx.emit(PageEvent::Muted(false)));
    page_a.update(cx, |_, cx| cx.emit(PageEvent::Media(Media { playing: false, ..Media::default() })));
    assert_eq!(mini(&b, cx), Some(a), "a paused track stays");
    b.update_in(cx, |b, window, cx| b.activate_tab(bb, window, cx));
    assert_eq!(mini(&b, cx), None, "until you switch away");
    playing(&page_a, cx);
    assert_eq!(mini(&b, cx), Some(a), "playing again brings it back");
    b.update_in(cx, |b, window, cx| b.activate_tab(c, window, cx));
    assert_eq!(mini(&b, cx), Some(a), "a switch keeps one that is still playing");
}

fn mini(b: &Entity<Browser<MockHost>>, cx: &mut VisualTestContext) -> Option<baa_model::TabId> {
    b.read_with(cx, |b, _| b.mini_player_tab())
}

#[gpui::test]
fn muted_is_what_chrome_says(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let (id, page) = open_page(&b, "a.example", cx);
    page.update(cx, |_, cx| cx.emit(PageEvent::Muted(true)));
    assert!(b.read_with(cx, |b, _| b.muted.contains(&id)));
    // The toggle asks Chrome to unmute; the tab stays muted until Chrome says otherwise.
    cx.dispatch_action(MuteTab);
    page.read_with(cx, |p, _| assert!(!p.muted.get()));
    assert!(b.read_with(cx, |b, _| b.muted.contains(&id)));
    page.update(cx, |_, cx| cx.emit(PageEvent::Muted(false)));
    assert!(!b.read_with(cx, |b, _| b.muted.contains(&id)));
}

#[gpui::test]
fn mini_player_shows_the_latest_playing_tab_off_screen(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let (a, page_a) = open_page(&b, "a.example", cx);
    let (bb, page_b) = open_page(&b, "b.example", cx);
    let (c, _) = open_page(&b, "c.example", cx);
    playing(&page_a, cx);
    playing(&page_b, cx);
    assert_eq!(mini(&b, cx), Some(bb));
    // b becomes a split pane beside c: on screen, so the mini-player falls back to a.
    b.update_in(cx, |b, window, cx| {
        let space = b.session.active_space().unwrap();
        assert!(b.session.add_to_split(space, bb, usize::MAX, now()).is_some());
        b.sync(window, cx);
        assert!(b.shown_panes().contains(&bb) && b.shown_panes().contains(&c));
    });
    assert_eq!(mini(&b, cx), Some(a));
    assert_eq!(b.read_with(cx, |b, _| b.mini_tab), Some(a));
    b.update_in(cx, |b, window, cx| b.activate_tab(a, window, cx));
    assert_eq!(mini(&b, cx), Some(bb), "a split not on screen is off screen");
}

#[gpui::test]
fn leaving_a_space_puts_its_playing_video_in_pip_when_the_setting_is_on(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let (a, page) = open_page(&b, "a.example", cx);
    playing(&page, cx);
    let first = b.read_with(cx, |b, _| b.session.active_space().unwrap());
    let second = b.update(cx, |b, _| b.session.add_space("Two", "x", None));
    let pips = |cx: &mut VisualTestContext| page.read_with(cx, |p, _| p.cdp.borrow().iter().filter(|c| *c == "media PictureInPicture").count());

    b.update_in(cx, |b, window, cx| b.activate_space(second, window, cx));
    assert_eq!(pips(cx), 1);
    assert_eq!(mini(&b, cx), Some(a), "the mini-player shows other Spaces' tabs");

    b.update_in(cx, |b, window, cx| {
        b.activate_space(first, window, cx);
        b.settings.auto_pip = false;
        b.activate_space(second, window, cx);
    });
    assert_eq!(pips(cx), 1, "off: no PiP");
}

#[gpui::test]
fn switching_tabs_puts_the_playing_video_in_pip(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let (a, page) = open_page(&b, "a.example", cx);
    playing(&page, cx);
    let pips = |cx: &mut VisualTestContext| page.read_with(cx, |p, _| p.cdp.borrow().iter().filter(|c| *c == "media PictureInPicture").count());

    let (other, _) = open_page(&b, "b.example", cx);
    assert_eq!(pips(cx), 1, "a left the screen while playing");
    let exits = |cx: &mut VisualTestContext| page.read_with(cx, |p, _| p.cdp.borrow().iter().filter(|c| *c == "media ExitPictureInPicture").count());
    b.update_in(cx, |b, window, cx| b.activate_tab(a, window, cx));
    assert_eq!(pips(cx), 1, "coming back is not leaving");
    assert_eq!(exits(cx), 1, "coming back closes the PiP baa opened");
    b.update_in(cx, |b, window, cx| b.activate_tab(other, window, cx));
    assert_eq!(pips(cx), 2);

    page.update(cx, |_, cx| cx.emit(PageEvent::Media(Media { playing: false, ..Media::default() })));
    b.update_in(cx, |b, window, cx| b.activate_tab(a, window, cx));
    b.update_in(cx, |b, window, cx| b.activate_tab(other, window, cx));
    assert_eq!(pips(cx), 2, "paused: no PiP");
    assert_eq!(exits(cx), 2, "only PiP baa opened is closed");
}

#[gpui::test]
fn back_to_tab_from_pip_shows_the_favorite(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let (fav, page) = open_page(&b, "youtube.com", cx);
    b.update(cx, |b, _| {
        let space = b.session.active_space().unwrap();
        b.session.move_tab(fav, space, baa_model::Zone::Favorite, 0);
    });
    let (other, _) = open_page(&b, "example.com", cx);
    assert_eq!(b.read_with(cx, |b, _| b.active_tab()), Some(other));
    page.update(cx, |_, cx| cx.emit(PageEvent::FocusRequested));
    cx.run_until_parked();
    assert_eq!(b.read_with(cx, |b, _| b.active_tab()), Some(fav));
}
