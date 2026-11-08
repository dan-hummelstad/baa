use super::*;
use crate::Suggestion;

fn suggestion(url: &str) -> Suggestion {
    Suggestion { url: url.into(), text: url.into(), description: String::new(), search: false }
}

/// An `autocomplete` callback with one match per url.
fn send_matches(env: &Env, id: u64, urls: &[&str], done: bool) {
    let strings: Vec<CString> = urls.iter().map(|u| CString::new(*u).unwrap()).collect();
    let raw: Vec<ffi::Match> = strings
        .iter()
        .map(|s| ffi::Match { url: s.as_ptr(), contents: s.as_ptr(), description: c"".as_ptr(), search: false })
        .collect();
    (env.callbacks.autocomplete)(env.ctx(), id, raw.as_ptr(), raw.len(), done);
}

#[gpui::test]
fn page_callbacks_become_events(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    (env.callbacks.fullscreen)(env.ctx(), tab, true);
    (env.callbacks.exclusive_access)(env.ctx(), tab, c"example.com".as_ptr(), 3);
    (env.callbacks.exclusive_access)(env.ctx(), tab, c"".as_ptr(), 0);
    (env.callbacks.find_result)(env.ctx(), tab, 2, 5);
    (env.callbacks.tab_group)(env.ctx(), tab, 1 << 63 | 5, c"X".as_ptr());
    (env.callbacks.tab_group)(env.ctx(), tab, 0, c"".as_ptr());
    cx.run_until_parked();
    assert_eq!(
        *seen.borrow(),
        [
            PageEvent::Fullscreen(true),
            PageEvent::ExclusiveAccess { origin: "example.com".into(), kind: Some(ExclusiveKind::KeyboardLock) },
            PageEvent::ExclusiveAccess { origin: String::new(), kind: None },
            PageEvent::FindResult { active: 2, total: 5 },
            PageEvent::GroupChanged { group: Some(1 << 63 | 5), title: "X".into() },
            PageEvent::GroupChanged { group: None, title: String::new() },
        ]
    );
}

#[gpui::test]
fn crashed_flips_report_once_each(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);
    cx.run_until_parked();
    seen.borrow_mut().clear();

    let crashed = State { crashed: true, ..Default::default() };
    env.state(tab, &crashed);
    env.state(tab, &crashed);
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [PageEvent::Crashed(true)]);
    env.state(tab, &State::default());
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [PageEvent::Crashed(true), PageEvent::Crashed(false)]);
}

#[gpui::test]
fn set_group_goes_to_chrome_with_none_as_key_zero(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    page.read_with(cx, |page, _| {
        page.set_group(Some((3, "A")));
        page.set_group(None);
        page.exit_fullscreen();
    });
    assert_eq!(calls("tab_set_group"), [format!("tab_set_group {tab:?} 3 A"), format!("tab_set_group {tab:?} 0 ")]);
    assert_eq!(calls("tab_exit_fullscreen").len(), 1);
}

#[gpui::test]
fn autocomplete_answers_only_the_latest_query(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    type Got = Rc<RefCell<Vec<(&'static str, Vec<Suggestion>)>>>;
    let got: Got = Rc::default();
    let (first, second) = (got.clone(), got.clone());
    env.host.autocomplete(ProfileId(1), "ex", Box::new(move |list, _| first.borrow_mut().push(("first", list))));
    env.host.autocomplete(ProfileId(1), "exa", Box::new(move |list, _| second.borrow_mut().push(("second", list))));
    assert_eq!(calls("profile_autocomplete"), ["profile_autocomplete 1 ex", "profile_autocomplete 2 exa"]);

    send_matches(&env, 1, &["https://late.test/"], true);
    cx.run_until_parked();
    assert!(got.borrow().is_empty(), "a replaced query is dropped");

    send_matches(&env, 2, &["https://a.test/"], false);
    send_matches(&env, 2, &["https://a.test/", "https://b.test/"], true);
    send_matches(&env, 2, &["https://after-done.test/"], true);
    cx.run_until_parked();
    assert_eq!(
        *got.borrow(),
        [
            ("second", vec![suggestion("https://a.test/")]),
            ("second", vec![suggestion("https://a.test/"), suggestion("https://b.test/")]),
        ],
        "one update per callback until done"
    );
}

#[gpui::test]
fn queued_matches_are_dropped_when_a_newer_query_starts_first(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let got: Rc<RefCell<Vec<Vec<Suggestion>>>> = Rc::default();
    let first = got.clone();
    env.host.autocomplete(ProfileId(1), "ex", Box::new(move |list, _| first.borrow_mut().push(list)));
    send_matches(&env, 1, &["https://a.test/"], false);
    env.host.autocomplete(ProfileId(1), "exa", Box::new(|_, _| {}));
    cx.run_until_parked();
    assert!(got.borrow().is_empty());
}
