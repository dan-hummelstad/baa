use super::*;
use crate::DocumentKind;

const MESSAGE: DocumentKind = DocumentKind::Message { remote_images: false };

fn document(env: &Env, kind: DocumentKind, cx: &mut TestAppContext) -> Entity<CorePage> {
    env.window.update(cx, |_, window, cx| env.host.create_document(ProfileId(1), kind, window, cx)).unwrap()
}

#[gpui::test]
fn a_document_is_a_blank_tab_that_set_document_writes(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = document(&env, MESSAGE, cx);
    let tab = env.tab_of(&page, cx);
    assert_eq!(calls("tab_create").len(), 1);
    assert!(calls("tab_create")[0].ends_with(" about:blank"));
    assert!(calls("tab_set_init_script").is_empty(), "no skin loader in a document");

    env.state(tab, &State { url: "about:blank", origin: "null", ..Default::default() });
    cx.run_until_parked();
    page.read_with(cx, |page, _| page.set_document("<p>Hei</p>"));
    let script = super::document::write_script(MESSAGE, "<p>Hei</p>");
    assert_eq!(calls("tab_set_init_script"), [format!("tab_set_init_script {tab} {script}")]);
    assert_eq!(calls("tab_eval"), [format!("tab_eval {tab} {script} false")]);
}

#[gpui::test]
fn a_page_with_a_url_takes_no_document(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let tab_page = env.page(1, "https://a.test", cx);
    CALLS.with(|c| c.borrow_mut().clear());
    tab_page.read_with(cx, |page, _| page.set_document("<p>no</p>"));

    let left = document(&env, DocumentKind::Editor, cx);
    let tab = env.tab_of(&left, cx);
    env.state(tab, &State { url: "https://evil.test/", origin: "https://evil.test", ..Default::default() });
    cx.run_until_parked();
    left.read_with(cx, |page, _| page.set_document("<p>no</p>"));

    assert!(calls("tab_eval").is_empty() && calls("tab_set_init_script").is_empty(), "{:?}", calls("tab_"));
}

#[gpui::test]
fn a_skin_change_leaves_a_documents_script_alone(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let page = document(&env, MESSAGE, cx);
    page.read_with(cx, |page, _| page.set_document("<p>Hei</p>"));
    let before = calls("tab_set_init_script").len();
    env.host.set_webui_skin(ProfileId(1), "a{color:red}".into(), true);
    assert_eq!(calls("tab_set_init_script").len(), before);
}
