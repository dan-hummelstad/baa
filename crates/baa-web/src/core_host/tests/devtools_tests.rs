use super::*;

#[gpui::test]
fn devtools_dock_in_two_halves_and_report_the_picker(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let inspected = env.page(1, "https://a.test", cx);
    let inspected_tab = env.tab_of(&inspected, cx);
    let devtools = env
        .window
        .update(cx, |_, window, cx| env.host.create_devtools(&inspected, Some((3, 4)), "left", window, cx))
        .unwrap();
    assert_eq!(calls("tab_open_devtools"), [format!("tab_open_devtools {inspected_tab:?} true 3 4")]);
    let front = env.tab_of(&devtools, cx);
    let seen = events(&devtools, cx);

    let first = "devtools://devtools/bundled/devtools_app.html?remoteBase=x&dockSide=left";
    let loading = State { url: first, loading: true, ..Default::default() };
    env.state(front, &loading);
    cx.run_until_parked();
    assert_eq!(calls("tab_eval"), Vec::<String>::new(), "not before it has loaded");

    env.state(front, &State { loading: false, ..loading.clone() });
    env.state(front, &State { loading: false, ..loading.clone() });
    cx.run_until_parked();
    let evals = calls("tab_eval");
    assert_eq!(evals.len(), 1);
    assert!(evals[0].contains("currentDockState") && evals[0].contains("\"left\""), "{}", evals[0]);

    // The script reloads the front-end with can_dock; the second half follows.
    let second = "devtools://devtools/bundled/devtools_app.html?remoteBase=x&can_dock=true";
    env.state(front, &State { url: second, loading: false, ..Default::default() });
    cx.run_until_parked();
    let evals = calls("tab_eval");
    assert_eq!(evals.len(), 2);
    assert!(evals[1].contains("__appDock") && evals[1].contains("setIsDocked(false, done)"));
    assert!(evals[1].contains("baa:"));

    (env.callbacks.page_message)(env.ctx(), front, c"devtools-dock:bottom".as_ptr());
    (env.callbacks.page_message)(env.ctx(), front, c"something-else".as_ptr());
    (env.callbacks.page_message)(env.ctx(), inspected_tab, c"devtools-dock:right".as_ptr());
    cx.run_until_parked();
    let docks: Vec<_> = seen.borrow().iter().filter(|e| matches!(e, PageEvent::DevToolsDock(_))).cloned().collect();
    assert_eq!(docks, [PageEvent::DevToolsDock("bottom".into())], "only DevTools' picker is heard");

    inspected.read_with(cx, |page, _| page.open_devtools(None));
    assert_eq!(calls("tab_show_devtools_window"), [format!("tab_show_devtools_window {inspected_tab:?} false 0 0")]);
}

#[gpui::test]
fn devtools_for_a_gone_page_close_themselves(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let inspected = env.page(1, "https://a.test", cx);
    (env.callbacks.tab_closed)(env.ctx(), env.tab_of(&inspected, cx));
    let devtools = env
        .window
        .update(cx, |_, window, cx| env.host.create_devtools(&inspected, None, "right", window, cx))
        .unwrap();
    let seen = events(&devtools, cx);
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [PageEvent::Closed]);
    assert_eq!(calls("tab_open_devtools"), Vec::<String>::new());
}
