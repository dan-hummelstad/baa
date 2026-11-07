//! Extensions in All Spaces, the install prompt's Space picker, and the delete warning.

use std::time::Duration;

use baa_model::{ProfileId, SpaceId, Store};
use baa_web::{Extension, ExtensionChange};
use gpui::{Entity, TestAppContext, VisualTestContext};

use super::InstallTarget;
use crate::{mock::MockHost, Browser};

fn browser(cx: &mut TestAppContext) -> (Entity<Browser<MockHost>>, &mut VisualTestContext) {
    cx.update(crate::init);
    cx.add_window_view(|window, cx| Browser::new(MockHost::new(cx), Store::open(":memory:").unwrap(), window, cx))
}

fn ext(id: &str) -> Extension {
    Extension { id: id.into(), name: id.to_uppercase(), icon: None, popup: None, options: None }
}

/// Two Spaces with their own profiles; `first` has extensions `ids`.
fn two_spaces(b: &Entity<Browser<MockHost>>, ids: &[&str], cx: &mut VisualTestContext) -> [(SpaceId, ProfileId); 2] {
    b.update_in(cx, |b, window, cx| {
        let first = b.session.active_space().unwrap();
        let p1 = b.session.space(first).unwrap().profile;
        b.host.profile_extensions.borrow_mut().insert(p1, ids.iter().map(|i| ext(i)).collect());
        b.new_space(window, cx);
        b.close_overlays(window, cx);
        let second = b.session.active_space().unwrap();
        let p2 = b.session.space(second).unwrap().profile;
        b.host.profile_extensions.borrow_mut().entry(p2).or_default();
        b.activate_space(first, window, cx);
        [(first, p1), (second, p2)]
    })
}

fn has(b: &Entity<Browser<MockHost>>, profile: ProfileId, id: &str, cx: &mut VisualTestContext) -> bool {
    b.read_with(cx, |b, _| b.host.profile_extensions.borrow().get(&profile).is_some_and(|l| l.iter().any(|e| e.id == id)))
}

#[gpui::test]
fn a_shared_extension_reaches_every_space_and_leaves_all_of_them(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let [(first, p1), (_, p2)] = two_spaces(&b, &["ubo"], cx);
    b.update(cx, |b, cx| b.set_extension_shared("ubo", true, cx));
    assert!(has(&b, p2, "ubo", cx), "copied into the other Space");
    assert_eq!(b.read_with(cx, |b, _| b.host.copies.borrow().clone()), [(p2, p1, "ubo".to_string())]);

    // A Space made later gets it too.
    let p3 = b.update_in(cx, |b, window, cx| {
        b.new_space(window, cx);
        b.close_overlays(window, cx);
        b.session.space(b.session.active_space().unwrap()).unwrap().profile
    });
    assert!(has(&b, p3, "ubo", cx), "a new Space gets the shared extension");

    b.update(cx, |b, cx| b.change_extension(first, "ubo", ExtensionChange::Remove, cx));
    let removed = b.read_with(cx, |b, _| b.host.extension_changes.borrow().iter().filter(|(_, c)| *c == ExtensionChange::Remove).count());
    assert_eq!(removed, 3, "removed from every Space");
    assert!(!b.read_with(cx, |b, _| b.is_extension_shared("ubo")));
}

#[gpui::test]
fn the_install_prompt_sends_the_new_extension_where_asked(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let [(first, p1), (second, p2)] = two_spaces(&b, &[], cx);

    // "All Spaces": shared once it shows up.
    b.update(cx, |b, cx| b.install_accepted(first, InstallTarget::All, cx));
    b.update(cx, |b, _| b.host.profile_extensions.borrow_mut().get_mut(&p1).unwrap().push(ext("pw")));
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert!(b.read_with(cx, |b, _| b.is_extension_shared("pw")));
    assert!(has(&b, p2, "pw", cx));

    // Another Space: copied there, then removed here.
    b.update(cx, |b, cx| b.install_accepted(first, InstallTarget::Space(second), cx));
    b.update(cx, |b, _| b.host.profile_extensions.borrow_mut().get_mut(&p1).unwrap().push(ext("dark")));
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert!(has(&b, p2, "dark", cx));
    assert!(!b.read_with(cx, |b, _| b.is_extension_shared("dark")));
    let changes = b.read_with(cx, |b, _| b.host.extension_changes.borrow().clone());
    assert_eq!(changes, [("dark".to_string(), ExtensionChange::Remove)], "the copy here goes");
}

#[gpui::test]
fn deleting_a_space_names_the_extensions_that_go_with_it(cx: &mut TestAppContext) {
    let (b, cx) = browser(cx);
    let [(first, _), (second, _)] = two_spaces(&b, &["ubo", "pw"], cx);
    b.update(cx, |b, cx| b.set_extension_shared("pw", true, cx));
    let lost = b.read_with(cx, |b, _| b.extensions_lost_with(first));
    assert_eq!(lost, ["UBO"], "shared extensions stay");
    assert_eq!(
        crate::settings::tabs::delete_warning(&lost),
        "Its tabs, logins and site data are deleted. Its extension UBO is removed too; extensions in All Spaces stay."
    );
    assert_eq!(crate::settings::tabs::delete_warning(&[]), "Its tabs, logins and site data are deleted.");
    assert!(b.read_with(cx, |b, _| b.extensions_lost_with(second)).is_empty(), "it only has the shared copy");
}
