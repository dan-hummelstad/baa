use super::*;

#[gpui::test]
fn devtools_open_with_their_side_and_report_the_picker(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let inspected = env.page(1, "https://a.test", cx);
    let inspected_tab = env.tab_of(&inspected, cx);
    let devtools = env
        .window
        .update(cx, |_, window, cx| env.host.create_devtools(&inspected, Some((3, 4)), "left", window, cx))
        .unwrap();
    assert_eq!(calls("tab_open_devtools"), [format!("tab_open_devtools {inspected_tab:?} true 3 4 {}", ffi::DOCK_LEFT)]);
    let front = env.tab_of(&devtools, cx);
    let seen = events(&devtools, cx);

    // Chrome docks the front-end itself now: baa no longer evals a dock script.
    env.state(front, &State { url: "devtools://devtools/bundled/devtools_app.html", ..Default::default() });
    cx.run_until_parked();
    assert_eq!(calls("tab_eval"), Vec::<String>::new());

    (env.callbacks.devtools_dock)(env.ctx(), front, ffi::DOCK_LEFT);
    (env.callbacks.devtools_dock)(env.ctx(), front, ffi::DOCK_UNDOCKED);
    (env.callbacks.devtools_dock)(env.ctx(), front, ffi::DOCK_RIGHT);
    cx.run_until_parked();
    let docks: Vec<_> = seen.borrow().iter().filter(|e| matches!(e, PageEvent::DevToolsDock(_))).cloned().collect();
    assert_eq!(
        docks,
        [PageEvent::DevToolsDock("left".into()), PageEvent::DevToolsDock("undocked".into()), PageEvent::DevToolsDock("right".into())]
    );

    inspected.read_with(cx, |page, _| page.open_devtools(None));
    assert_eq!(calls("tab_show_devtools_window"), [format!("tab_show_devtools_window {inspected_tab:?} false 0 0")]);
}

#[gpui::test]
fn devtools_for_a_gone_page_close_themselves(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let inspected = env.page(1, "https://a.test", cx);
    (env.callbacks.tab_closed)(env.ctx(), env.tab_of(&inspected, cx), ffi::CLOSED_BY_CHROME);
    let devtools = env
        .window
        .update(cx, |_, window, cx| env.host.create_devtools(&inspected, None, "right", window, cx))
        .unwrap();
    let seen = events(&devtools, cx);
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [PageEvent::Closed]);
    assert_eq!(calls("tab_open_devtools"), Vec::<String>::new());
}
