//! Mail in the shell (PLAN-mail.md §5), on `MockHost`.

use baa_model::Store;
use baa_web::{DocumentKind, PageHandle, WebHost};
use gpui::TestAppContext;

use crate::{mock::MockHost, Browser};

#[gpui::test]
fn the_mock_host_records_a_document_and_what_was_set_on_it(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (b, cx) = cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx));
    b.update_in(cx, |b, window, cx| {
        assert!(b.mail.is_none(), "nothing turns mail on yet");
        let host = b.host.clone();
        let profile = b.active_space().unwrap().profile;
        let tabs = host.created.borrow().len();

        let message = host.create_document(profile, DocumentKind::Message { remote_images: false }, window, cx);
        let editor = host.create_document(profile, DocumentKind::Editor, window, cx);
        assert_eq!(*host.documents.borrow(), [(profile, DocumentKind::Message { remote_images: false }), (profile, DocumentKind::Editor)]);
        assert_eq!(host.created.borrow().len(), tabs, "a document is not a tab");

        message.read(cx).set_document("<p>Hei</p>");
        message.read(cx).set_document("<p>Hei igjen</p>");
        assert_eq!(*message.read(cx).set_documents.borrow(), ["<p>Hei</p>", "<p>Hei igjen</p>"]);
        assert!(editor.read(cx).set_documents.borrow().is_empty());

        let tab = host.create_page(profile, "https://example.com/", true, window, cx);
        tab.read(cx).set_document("<p>not here</p>");
        assert!(tab.read(cx).set_documents.borrow().is_empty(), "a page that has a URL takes no document");
        tab.read(cx).fill_code(7, "123456");
        assert_eq!(*tab.read(cx).filled.borrow(), [(7, "123456".to_string())]);

        host.sites.borrow_mut().insert("mail.google.com".into(), "google.com".into());
        assert_eq!((host.site_of("mail.google.com"), host.site_of("example.com")), ("google.com".into(), "example.com".into()));
    });
}
