//! The command bar on Chrome's autocomplete.

use baa_model::Store;
use baa_web::Suggestion;
use gpui::TestAppContext;

use crate::{mock::MockHost, Browser};

fn m(url: &str, text: &str, description: &str, search: bool) -> Suggestion {
    Suggestion { url: url.into(), text: text.into(), description: description.into(), search }
}

#[gpui::test]
fn typing_asks_after_the_delay_and_updates_show_rows(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (b, cx) = cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx));
    cx.simulate_keystrokes("secondary-t");
    cx.simulate_input("2+2");
    b.read_with(cx, |b, _| assert!(b.host.autocompletes.borrow().is_empty(), "not before the delay"));
    cx.executor().advance_clock(std::time::Duration::from_millis(200));
    cx.run_until_parked();
    b.read_with(cx, |b, _| assert_eq!(*b.host.autocompletes.borrow(), ["2+2"]));
    b.update(cx, |b, cx| b.host.suggest(vec![m("", "2+2 = 4", "", true), m("https://calc.example/", "calc.example", "Calc", false)], cx));
    cx.run_until_parked();
    b.read_with(cx, |b, _| {
        let bar = b.bar.as_ref().unwrap();
        let rows = crate::command_bar::rows(&bar.field.text, b.session(), &b.settings, &bar.matches);
        assert!(rows.iter().any(|r| r.suggestion && r.label == "2+2 = 4"));
        assert!(rows.iter().any(|r| r.history && r.label == "Calc"));
    });
}
