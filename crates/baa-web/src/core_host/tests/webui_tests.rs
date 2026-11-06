use super::*;

fn index_of(all: &[String], needle: &str) -> usize {
    all.iter().position(|c| c.contains(needle)).unwrap_or_else(|| panic!("no `{needle}` in {all:?}"))
}

#[gpui::test]
fn a_chrome_page_starts_blank_gets_the_script_then_loads_as_chrome(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "baa://flags", cx);
    let tab = env.tab_of(&page, cx);
    let all = calls("");
    let created = index_of(&all, "tab_create");
    assert!(all[created].ends_with("about:blank"));
    let script = index_of(&all, "tab_set_init_script");
    let load = index_of(&all, "tab_load_url");
    assert!(created < script && script < load);
    assert!(all[load].ends_with("chrome://flags"));
    assert!(all[script].contains(&format!("{tab:?}")));
    // Websites pay one comparison: it is the first statement.
    let body = all[script].split_once('\n').unwrap().1;
    assert!(body.trim_start().starts_with("if (location.protocol !== 'chrome:' && location.protocol !== 'baa:') return;"), "{body}");
    assert_eq!(calls("tab_eval"), Vec::<String>::new());
}

#[gpui::test]
fn a_website_loads_directly_with_the_guarded_script(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    env.tab_of(&page, cx);
    assert!(calls("tab_create")[0].ends_with("https://a.test"));
    assert_eq!(calls("tab_load_url"), Vec::<String>::new());
    assert_eq!(calls("tab_set_init_script").len(), 1);
}

#[gpui::test]
fn urls_reach_the_ui_as_baa_and_go_to_chrome_as_chrome(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);
    env.state(tab, &State { url: "chrome://flags/?u=chrome://x", ..Default::default() });
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [PageEvent::UrlChanged("baa://flags/?u=chrome://x".into())]);
    assert_eq!(page.read_with(cx, |p, _| p.url()), "baa://flags/?u=chrome://x");

    page.read_with(cx, |p, _| p.load_url("baa://settings/fonts"));
    assert_eq!(calls("tab_load_url").last().unwrap(), &format!("tab_load_url {tab:?} chrome://settings/fonts"));
    page.read_with(cx, |p, _| p.load_url("https://c.test/?r=baa://x"));
    assert_eq!(calls("tab_load_url").last().unwrap(), &format!("tab_load_url {tab:?} https://c.test/?r=baa://x"));
}

#[gpui::test]
fn tabs_chrome_made_for_chrome_pages_are_reported_and_adopted_as_baa(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let opener = env.page(1, "https://a.test", cx);
    let opener_tab = env.tab_of(&opener, cx);
    let seen = events(&opener, cx);
    env.created(0x9000 as *mut Tab, opener_tab, ProfileId(1), "chrome://extensions");
    cx.run_until_parked();
    assert_eq!(
        *seen.borrow(),
        [PageEvent::OpenRequested { url: "baa://extensions".into(), from_user_gesture: true, background: false }]
    );

    // Nobody asked for this one, so it waits for a page instead of being closed this turn.
    env.created(0x9010 as *mut Tab, std::ptr::null_mut(), ProfileId(1), "chrome://version");
    let creates = calls("tab_create").len();
    let adopted = env.page(1, "baa://version", cx);
    assert_eq!(env.tab_of(&adopted, cx), 0x9010 as *mut Tab);
    assert_eq!(calls("tab_create").len(), creates);
    assert_eq!(calls("tab_load_url"), Vec::<String>::new(), "already on its way");
    let evals = calls("tab_eval");
    assert_eq!(evals.len(), 1, "the skin once, for a page that is up already");
    assert!(evals[0].contains("window.__baaSkin = ") && evals[0].contains("location.protocol"));
}

#[gpui::test]
fn the_skin_reaches_only_that_profiles_tabs_and_guards_in_the_page(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let own = env.page(1, "baa://flags", cx);
    let site = env.page(1, "https://a.test", cx);
    let other = env.page(2, "baa://flags", cx);
    let (own_tab, site_tab, other_tab) = (env.tab_of(&own, cx), env.tab_of(&site, cx), env.tab_of(&other, cx));
    CALLS.with(|c| c.borrow_mut().clear());

    env.host.set_webui_skin(ProfileId(1), "a{color:red}".into(), true);
    let evals = calls("tab_eval");
    assert_eq!(evals.len(), 2, "both tabs of the profile, none of the other: {evals:?}");
    assert!(evals.iter().all(|c| c.contains("location.protocol === 'chrome:'") && c.contains("__baaSkin(\"a{color:red}\")")));
    assert!(evals.iter().any(|c| c.contains(&format!("{own_tab:?}"))) && evals.iter().any(|c| c.contains(&format!("{site_tab:?}"))));
    let scripts = calls("tab_set_init_script");
    assert_eq!(scripts.len(), 2);
    assert!(scripts.iter().all(|c| !c.contains(&format!("{other_tab:?}")) && c.contains("a{color:red}")));
    assert_eq!(calls("profile_set_color_schemes").len(), 1);
    assert!(calls("profile_set_color_schemes")[0].ends_with(" 0 2"), "Chrome's pages are dark");

    CALLS.with(|c| c.borrow_mut().clear());
    env.host.set_webui_skin(ProfileId(1), "a{color:red}".into(), true);
    assert_eq!(calls("tab_"), Vec::<String>::new(), "unchanged skin, nothing sent");

    // A Chrome page opened later starts in it.
    let later = env.page(1, "baa://version", cx);
    let later_tab = env.tab_of(&later, cx);
    let scripts = calls("tab_set_init_script");
    assert!(scripts[0].contains(&format!("{later_tab:?}")) && scripts[0].contains("a{color:red}"));
    drop((own, site, other, later));
}

#[gpui::test]
fn a_skin_never_opens_a_profile_and_a_later_one_inherits_it(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.host.set_webui_skin(ProfileId(7), "b{}".into(), true);
    assert_eq!(calls("profile_open"), Vec::<String>::new());

    env.page(7, "https://a.test", cx);
    assert!(calls("profile_set_color_schemes")[0].ends_with(" 0 2"));
    let page = env.page(7, "baa://flags", cx);
    let tab = env.tab_of(&page, cx);
    assert!(calls("tab_set_init_script").iter().any(|c| c.contains(&format!("{tab:?}")) && c.contains("b{}")));
}

#[test]
fn the_loader_keeps_nothing_in_storage() {
    let script = super::super::skin::loader("a{}");
    assert!(!script.contains("localStorage") && !script.contains("token"));
}
