use baa_model::{Cookies, PageScheme, Settings};

use super::*;
use crate::{Callback, ClearData};

/// Every write call to Chrome since the log was last cleared.
fn writes_logged() -> Vec<String> {
    CALLS.with(|c| {
        c.borrow()
            .iter()
            .filter(|c| {
                ["pref_set_", "pref_clear", "profile_set_default_zoom", "profile_set_search_engine", "profile_set_site_rule"]
                    .iter()
                    .any(|p| c.starts_with(p))
            })
            .cloned()
            .collect()
    })
}

fn clear_log() {
    CALLS.with(|c| c.borrow_mut().clear());
}

fn with(f: impl FnOnce(&mut Settings)) -> Settings {
    let mut settings = Settings::default();
    f(&mut settings);
    settings
}

#[gpui::test]
fn settings_go_to_every_open_profile_and_local_state_once(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.page(1, "https://a.test", cx);
    env.page(2, "https://b.test", cx);
    clear_log();

    env.host.apply_settings(&Settings::default());
    let logged = writes_logged();
    let local: Vec<_> = logged.iter().filter(|c| c.contains(" 0x0 ")).collect();
    assert_eq!(local.len(), 4, "{local:?}");
    assert!(local.iter().any(|c| c.ends_with("dns_over_https.mode automatic")));
    let at = |name: &str| logged.iter().position(|c| c.contains(name)).unwrap();
    assert!(at("dns_over_https.templates") < at("dns_over_https.mode"), "templates before mode");
    let per_profile = super::super::settings::writes(&Settings::default()).profile.len();
    assert_eq!(logged.len(), 4 + 2 * per_profile, "every other write goes to both profiles");
    assert!(logged.iter().any(|c| c.starts_with("profile_set_search_engine Google  https://www.google.com")));
    assert_eq!(logged.iter().filter(|c| c.starts_with("profile_set_default_zoom 1")).count(), 2);

    // Nothing changed: nothing sent. One change: one call per profile.
    clear_log();
    env.host.apply_settings(&Settings::default());
    assert_eq!(writes_logged(), Vec::<String>::new());
    env.host.apply_settings(&with(|s| s.web.font_size = 20));
    let logged = writes_logged();
    assert_eq!(logged.len(), 2, "{logged:?}");
    assert!(logged.iter().all(|c| c.ends_with("webkit.webprefs.default_font_size 20")));

    // A local-state change goes out once.
    clear_log();
    env.host.apply_settings(&with(|s| {
        s.web.font_size = 20;
        s.web.memory_saver = true;
    }));
    assert_eq!(writes_logged(), ["pref_set_int 0x0 performance_tuning.high_efficiency_mode.state 2"]);
}

#[gpui::test]
fn a_profile_opened_later_gets_everything_before_the_privacy_pins(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.page(1, "https://a.test", cx);
    env.host.apply_settings(&with(|s| s.web.do_not_track = true));
    clear_log();

    env.page(3, "https://c.test", cx);
    let all = calls("");
    let at = |needle: &str| all.iter().position(|c| c.contains(needle)).unwrap_or_else(|| panic!("no {needle}: {all:?}"));
    assert!(at("profile_open space-3") < at("enable_do_not_track true"));
    assert!(at("enable_do_not_track true") < at("alternate_error_pages.enabled false"), "pins come last");
    assert!(!all.iter().any(|c| c.contains("hardware_acceleration_mode")), "Local State is not per profile");
}

#[gpui::test]
fn settings_before_the_first_apply_send_nothing(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.page(1, "https://a.test", cx);
    assert!(writes_logged().iter().all(|c| c.starts_with("pref_set_bool")), "only the privacy pins");
}

#[gpui::test]
fn a_site_default_that_is_no_longer_set_goes_back_to_chromes(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.page(1, "https://a.test", cx);
    env.host.apply_settings(&with(|s| s.web.cookies = Cookies::BlockAll));
    assert!(writes_logged().contains(&"profile_set_site_rule 1024 * 2".to_string()));

    clear_log();
    env.host.apply_settings(&Settings::default());
    assert!(writes_logged().contains(&"profile_set_site_rule 1024 * -1".to_string()));

    // A profile that opens after the release is told too: the list is total.
    clear_log();
    env.page(2, "https://b.test", cx);
    assert!(writes_logged().contains(&"profile_set_site_rule 1024 * -1".to_string()));
    assert!(writes_logged().iter().any(|c| c.starts_with("pref_clear ") && c.ends_with("intl.accept_languages")));
}

#[gpui::test]
fn color_schemes_go_to_each_profile_when_it_opens_and_when_they_change(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    env.page(1, "https://a.test", cx);
    assert_eq!(calls("profile_set_color_schemes").len(), 1);
    assert!(calls("profile_set_color_schemes")[0].ends_with(" 0 0"), "system, system");
    env.page(1, "https://b.test", cx);
    assert_eq!(calls("profile_set_color_schemes").len(), 1, "once per profile, not per page");

    clear_log();
    env.host.apply_settings(&with(|s| s.page_scheme = PageScheme::Dark));
    assert_eq!(calls("profile_set_color_schemes").len(), 1);
    assert!(calls("profile_set_color_schemes")[0].ends_with(" 2 0"));
    env.host.apply_settings(&with(|s| s.page_scheme = PageScheme::Dark));
    assert_eq!(calls("profile_set_color_schemes").len(), 1, "unchanged");

    env.page(2, "https://c.test", cx);
    assert!(calls("profile_set_color_schemes")[1].ends_with(" 2 0"), "a later profile starts with the setting");
}

#[gpui::test]
fn site_rules_decode_the_visitor_and_unset_is_sent(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    SITE_RULES.with(|r| {
        r.borrow_mut().extend([
            ("*".to_string(), 2),
            ("[*.]a.com".to_string(), 3),
            ("b.com".to_string(), 1),
            ("c.com".to_string(), 0),
        ])
    });
    assert_eq!(
        env.host.site_rules(ProfileId(1), Permission::Cookies),
        [
            ("*".to_string(), PermissionState::Block),
            ("[*.]a.com".to_string(), PermissionState::SessionOnly),
            ("b.com".to_string(), PermissionState::Allow),
            ("c.com".to_string(), PermissionState::Ask),
        ]
    );
    assert_eq!(calls("profile_site_rules"), ["profile_site_rules 1024"]);
    assert_eq!(env.host.site_rules(ProfileId(1), Permission::Other), []);
    assert_eq!(calls("profile_site_rules").len(), 1, "Other has no rules to ask for");

    env.host.set_site_rule(ProfileId(1), Permission::Popups, "[*.]a.com", None);
    env.host.set_site_rule(ProfileId(1), Permission::Popups, "*", Some(PermissionState::Allow));
    assert_eq!(calls("profile_set_site_rule"), ["profile_set_site_rule 128 [*.]a.com -1", "profile_set_site_rule 128 * 1"]);
}

fn ran(count: &Rc<Cell<u32>>) -> Callback<()> {
    let count = count.clone();
    Box::new(move |(), _| count.set(count.get() + 1))
}
#[gpui::test]
fn clear_data_sends_bits_and_since_and_answers_once_on_a_later_turn(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let count = Rc::new(Cell::new(0));
    let since = std::time::UNIX_EPOCH + Duration::from_secs(5);
    let what = ClearData { history: true, cache: true, ..Default::default() };
    cx.update(|cx| env.host.clear_data(ProfileId(1), Some(since), what, cx, ran(&count)));
    cx.update(|cx| env.host.clear_data(ProfileId(1), None, ClearData { site_data: true, downloads: true, ..Default::default() }, cx, ran(&count)));
    assert_eq!(calls("profile_clear_data"), ["profile_clear_data 1 5000 9", "profile_clear_data 2 0 6"]);
    cx.run_until_parked();
    assert_eq!(count.get(), 0, "Chrome has not answered");

    (env.callbacks.clear_done)(env.ctx(), 1);
    assert_eq!(count.get(), 0, "never inside the callback");
    cx.run_until_parked();
    assert_eq!(count.get(), 1);
    (env.callbacks.clear_done)(env.ctx(), 1);
    (env.callbacks.clear_done)(env.ctx(), 99);
    cx.run_until_parked();
    assert_eq!(count.get(), 1, "an answered or unknown id is ignored");
    (env.callbacks.clear_done)(env.ctx(), 2);
    cx.run_until_parked();
    assert_eq!(count.get(), 2);
}

#[gpui::test]
fn load_unpacked_resolves_with_the_id_or_chromes_error(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    let results = Rc::new(RefCell::new(Vec::new()));
    for _ in 0..2 {
        let sink = results.clone();
        cx.update(|cx| {
            env.host.load_unpacked(ProfileId(1), std::path::Path::new("/tmp/ext"), cx, Box::new(move |r, _| sink.borrow_mut().push(r)))
        });
    }
    assert_eq!(calls("profile_load_unpacked"), ["profile_load_unpacked 1 /tmp/ext", "profile_load_unpacked 2 /tmp/ext"]);
    (env.callbacks.load_unpacked_done)(env.ctx(), 2, false, c"Manifest file is missing".as_ptr());
    (env.callbacks.load_unpacked_done)(env.ctx(), 1, true, c"abcdef".as_ptr());
    assert!(results.borrow().is_empty());
    cx.run_until_parked();
    assert_eq!(*results.borrow(), [Err("Manifest file is missing".to_string()), Ok("abcdef".to_string())]);

    // An Incognito Space has no extensions: refused without asking Chrome, still on a later turn.
    let ephemeral = env.host.ephemeral_profile();
    let sink = results.clone();
    cx.update(|cx| env.host.load_unpacked(ephemeral, std::path::Path::new("/x"), cx, Box::new(move |r, _| sink.borrow_mut().push(r))));
    assert_eq!(results.borrow().len(), 2);
    cx.run_until_parked();
    assert!(results.borrow()[2].is_err());
    assert_eq!(calls("profile_load_unpacked").len(), 2);
}

#[gpui::test]
fn default_browser_asks_and_sets_through_the_core(cx: &mut TestAppContext) {
    let env = Env::new(cx);
    assert!(!env.host.is_default_browser());
    DEFAULT_BROWSER.with(|d| d.set(true));
    assert!(env.host.is_default_browser());
    env.host.make_default_browser();
    assert_eq!(calls("set_default_browser"), ["set_default_browser"]);
}
