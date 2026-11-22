use super::*;
use gpui::{point, px, size, Bounds};

#[gpui::test]
fn a_code_field_reaches_its_page_and_goes(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    let seen = events(&page, cx);

    let (origin, top) = (c"https://login.a.test".as_ptr(), c"https://a.test".as_ptr());
    (env.callbacks.code_field)(env.ctx(), tab, 7, origin, top, 10., 20., 300., 40., true);
    (env.callbacks.code_field)(env.ctx(), tab, 7, origin, top, 10., 20., 300., 40., false);
    (env.callbacks.code_field)(env.ctx(), 9999, 8, origin, top, 0., 0., 1., 1., true);
    cx.run_until_parked();
    let field = |at| PageEvent::CodeField { id: 7, origin: "https://login.a.test".into(), top: "https://a.test".into(), at };
    assert_eq!(*seen.borrow(), [field(Some(Bounds::new(point(px(10.), px(20.)), size(px(300.), px(40.))))), field(None)]);
}

#[gpui::test]
fn a_fill_goes_to_the_engine_with_its_field(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = env.page(1, "https://a.test", cx);
    let tab = env.tab_of(&page, cx);
    page.read_with(cx, |page, _| page.fill_code(7, "123456"));
    assert_eq!(calls("tab_fill_code"), [format!("tab_fill_code {tab} 7 123456")]);
}

#[gpui::test]
fn the_engine_names_the_site(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    assert_eq!(env.host.site_of("mail.google.com"), "google.com");
    assert_eq!(env.host.site_of("example.com"), "example.com");
    assert_eq!(env.host.site_of("long.test"), "long.test", "a cut answer is not a site");
}
