//! Which engine a page gets. Pages that do go to WebKit are not made here: a test binary
//! has no app bundle, and WebKit's processes refuse it.

use super::*;

fn webkit_on(env: &Env, cx: &mut TestAppContext) {
    let mut settings = baa_model::Settings::default();
    settings.web.webkit_drm = true;
    cx.update(|_| env.host.apply_settings(&settings));
}

#[gpui::test]
fn a_page_supplied_url_of_a_listed_site_stays_in_chrome(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    webkit_on(&env, cx);
    env.opened(1, "https://www.netflix.com/title/1", cx);
    assert_eq!(calls("tab_create").len(), 1, "never re-issued to WebKit");
}

#[gpui::test]
fn with_the_switch_off_a_listed_site_is_a_chrome_tab(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://www.netflix.com/", cx);
    assert_eq!(calls("tab_create").len(), 1);
    page.read_with(cx, |page, _| page.load_url("https://www.disneyplus.com/"));
    assert_eq!(calls("tab_load_url").len(), 1);
    assert_eq!(calls("tab_close").len(), 0);
}

#[gpui::test]
fn an_unlisted_url_loads_in_the_same_chrome_tab(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    webkit_on(&env, cx);
    let page = env.page(1, "https://example.com/", cx);
    page.read_with(cx, |page, _| page.load_url("http://www.netflix.com/"));
    assert_eq!(calls("tab_create").len(), 1);
    assert_eq!(calls("tab_load_url").len(), 1, "http is never sent to WebKit");
    assert_eq!(calls("tab_close").len(), 0);
}
