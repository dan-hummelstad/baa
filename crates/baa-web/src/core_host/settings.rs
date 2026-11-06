//! baa's global settings as Chrome writes (PLAN-settings.md §7). `writes` is
//! pure; the host sends the list to every profile as it opens and the
//! difference on every change.

use std::ffi::c_int;

use baa_model::{Cookies, Https, PageScheme, Permission, PermissionState, Preload, SecureDns, Settings};

use super::{
    c_string,
    ffi::{self, Core, Profile},
    host::{permission_bit, state_code},
};

#[derive(Debug, Clone, PartialEq)]
pub enum Write {
    Bool(&'static str, bool),
    Int(&'static str, i32),
    Str(&'static str, String),
    LocalBool(&'static str, bool),
    LocalInt(&'static str, i32),
    LocalStr(&'static str, String),
    SiteDefault(Permission, PermissionState),
    Engine { name: String, url: String },
    Zoom(f64),
}

impl Write {
    /// Local State is browser-wide: written once, not per profile.
    pub fn is_local(&self) -> bool {
        matches!(self, Self::LocalBool(..) | Self::LocalInt(..) | Self::LocalStr(..))
    }
}

pub fn writes(settings: &Settings) -> Vec<Write> {
    let web = &settings.web;
    let mut out = vec![Write::Int(
        "profile.cookie_controls_mode",
        if web.cookies == Cookies::AllowAll { 0 } else { 1 },
    )];
    // Block wins over session-only; either wins over a default the user set in site settings.
    let cookies = match (web.cookies, web.clear_cookies_on_quit) {
        (Cookies::BlockAll, _) => Some(PermissionState::Block),
        (_, true) => Some(PermissionState::SessionOnly),
        _ => None,
    };
    out.extend(cookies.map(|state| Write::SiteDefault(Permission::Cookies, state)));
    let (strict, balanced) = (web.https == Https::Strict, web.https == Https::Balanced);
    out.push(Write::Bool("https_only_mode_enabled", strict));
    out.push(Write::Bool("https_first_balanced_mode_enabled", balanced));
    out.push(Write::Int(
        "net.network_prediction_options",
        match web.preload {
            Preload::Standard => 0,
            Preload::Off => 2,
            Preload::Extended => 3,
        },
    ));
    out.push(Write::Bool("enable_do_not_track", web.do_not_track));
    out.push(Write::Bool("helium.global_privacy_control", web.global_privacy_control));
    out.push(Write::Bool("download.prompt_for_download", web.ask_where_to_save));
    // ponytail: back to None leaves the pushed value in place; Chrome's default returns when the profile is next created.
    if let Some(dir) = &web.download_dir {
        out.push(Write::Str("download.default_directory", dir.to_string_lossy().into_owned()));
    }
    if let Some(languages) = &web.languages {
        out.push(Write::Str("intl.accept_languages", languages.clone()));
    }
    out.push(Write::Bool("browser.enable_spellchecking", web.spellcheck));
    out.push(Write::Int("webkit.webprefs.default_font_size", web.font_size as i32));
    out.push(Write::Int("webkit.webprefs.minimum_font_size", web.min_font_size as i32));
    out.push(Write::Bool("settings.a11y.focus_highlight", web.focus_highlight));
    out.push(Write::Bool("settings.a11y.caretbrowsing.enabled", web.caret_browsing));
    out.push(Write::Bool("extensions.ui.developer_mode", web.developer_mode));
    out.push(Write::Zoom(web.default_zoom));
    out.push(Write::Engine { name: settings.search_engine.name.clone(), url: settings.search_engine.url.clone() });
    for &(kind, state) in &web.site_defaults {
        let taken = out.iter().any(|w| matches!(w, Write::SiteDefault(k, _) if *k == kind));
        if !taken && kind.takes(state) {
            out.push(Write::SiteDefault(kind, state));
        }
    }

    let (mode, templates) = match &web.secure_dns {
        SecureDns::Off => ("off", String::new()),
        SecureDns::Automatic => ("automatic", String::new()),
        SecureDns::Custom(template) => ("secure", template.clone()),
    };
    out.push(Write::LocalStr("dns_over_https.mode", mode.into()));
    out.push(Write::LocalStr("dns_over_https.templates", templates));
    out.push(Write::LocalBool("hardware_acceleration_mode.enabled", web.hardware_acceleration));
    out.push(Write::LocalInt("performance_tuning.high_efficiency_mode.state", if web.memory_saver { 2 } else { 0 }));
    out
}

/// The `baa_tab_set_color_scheme` value websites get.
pub fn page_scheme_code(scheme: PageScheme) -> c_int {
    match scheme {
        PageScheme::System => 0,
        PageScheme::Light => 1,
        PageScheme::Dark => 2,
    }
}

/// Sends `write` to `profile` (ignored for Local State writes).
pub fn apply(core: &Core, profile: *mut Profile, write: &Write) {
    let local = std::ptr::null_mut();
    let accepted = match write {
        Write::Bool(name, v) => (core.pref_set_bool)(profile, c_string(name).as_ptr(), *v),
        Write::Int(name, v) => (core.pref_set_int)(profile, c_string(name).as_ptr(), *v),
        Write::Str(name, v) => (core.pref_set_string)(profile, c_string(name).as_ptr(), c_string(v).as_ptr()),
        Write::LocalBool(name, v) => (core.pref_set_bool)(local, c_string(name).as_ptr(), *v),
        Write::LocalInt(name, v) => (core.pref_set_int)(local, c_string(name).as_ptr(), *v),
        Write::LocalStr(name, v) => (core.pref_set_string)(local, c_string(name).as_ptr(), c_string(v).as_ptr()),
        Write::SiteDefault(kind, state) => {
            permission_bit(*kind).is_some_and(|bit| (core.profile_set_site_rule)(profile, bit, c"*".as_ptr(), state_code(*state)))
        }
        Write::Engine { name, url } => (core.profile_set_search_engine)(profile, c_string(name).as_ptr(), c"".as_ptr(), c_string(url).as_ptr()),
        Write::Zoom(factor) => {
            (core.profile_set_default_zoom)(profile, *factor);
            true
        }
    };
    if !accepted {
        log::warn!("Chrome refused {write:?}");
    }
}

/// A default the user no longer overrides goes back to Chrome's.
pub fn restore_default(core: &Core, profile: *mut Profile, kind: Permission) {
    if let Some(bit) = permission_bit(kind) {
        (core.profile_set_site_rule)(profile, bit, c"*".as_ptr(), ffi::SITE_UNSET);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baa_model::SecureDns;

    fn default_list() -> Vec<Write> {
        use Write::*;
        vec![
            Int("profile.cookie_controls_mode", 1),
            Bool("https_only_mode_enabled", false),
            Bool("https_first_balanced_mode_enabled", true),
            Int("net.network_prediction_options", 2),
            Bool("enable_do_not_track", false),
            Bool("helium.global_privacy_control", false),
            Bool("download.prompt_for_download", false),
            Bool("browser.enable_spellchecking", true),
            Int("webkit.webprefs.default_font_size", 16),
            Int("webkit.webprefs.minimum_font_size", 0),
            Bool("settings.a11y.focus_highlight", false),
            Bool("settings.a11y.caretbrowsing.enabled", false),
            Bool("extensions.ui.developer_mode", false),
            Zoom(1.0),
            Engine { name: "Google".into(), url: "https://www.google.com/search?q=%s".into() },
            LocalStr("dns_over_https.mode", "automatic".into()),
            LocalStr("dns_over_https.templates", String::new()),
            LocalBool("hardware_acceleration_mode.enabled", true),
            LocalInt("performance_tuning.high_efficiency_mode.state", 2),
        ]
    }

    #[test]
    fn defaults_are_the_exact_list() {
        assert_eq!(writes(&Settings::default()), default_list());
    }

    #[test]
    fn block_all_beats_clear_on_quit_beats_a_site_default() {
        let mut s = Settings::default();
        s.web.cookies = Cookies::BlockAll;
        s.web.clear_cookies_on_quit = true;
        s.web.site_defaults = vec![(Permission::Cookies, PermissionState::Allow)];
        let list = writes(&s);
        assert!(list.contains(&Write::Int("profile.cookie_controls_mode", 1)));
        let cookie_defaults: Vec<_> = list.iter().filter(|w| matches!(w, Write::SiteDefault(Permission::Cookies, _))).collect();
        assert_eq!(cookie_defaults, [&Write::SiteDefault(Permission::Cookies, PermissionState::Block)]);

        s.web.cookies = Cookies::AllowAll;
        let list = writes(&s);
        assert!(list.contains(&Write::Int("profile.cookie_controls_mode", 0)));
        assert!(list.contains(&Write::SiteDefault(Permission::Cookies, PermissionState::SessionOnly)));

        s.web.clear_cookies_on_quit = false;
        assert!(writes(&s).contains(&Write::SiteDefault(Permission::Cookies, PermissionState::Allow)));
    }

    #[test]
    fn https_modes_write_the_pair_chrome_writes() {
        let pair = |https| {
            let mut s = Settings::default();
            s.web.https = https;
            let list = writes(&s);
            (
                list.contains(&Write::Bool("https_only_mode_enabled", true)),
                list.contains(&Write::Bool("https_first_balanced_mode_enabled", true)),
            )
        };
        assert_eq!(pair(Https::Strict), (true, false));
        assert_eq!(pair(Https::Balanced), (false, true));
        assert_eq!(pair(Https::Off), (false, false));
    }

    #[test]
    fn optional_prefs_write_only_when_set() {
        let mut s = Settings::default();
        let has = |list: &[Write], name: &str| list.iter().any(|w| matches!(w, Write::Str(n, _) if *n == name));
        assert!(!has(&writes(&s), "intl.accept_languages") && !has(&writes(&s), "download.default_directory"));
        s.web.languages = Some("en-US, sv".into());
        s.web.download_dir = Some("/tmp/dl".into());
        let list = writes(&s);
        assert!(list.contains(&Write::Str("intl.accept_languages", "en-US, sv".into())));
        assert!(list.contains(&Write::Str("download.default_directory", "/tmp/dl".into())));
    }

    #[test]
    fn local_state_and_enums_map_to_chrome_values() {
        let mut s = Settings::default();
        s.web.secure_dns = SecureDns::Custom("https://dns.quad9.net/dns-query".into());
        s.web.preload = Preload::Extended;
        s.web.memory_saver = false;
        s.web.hardware_acceleration = false;
        let list = writes(&s);
        assert!(list.contains(&Write::LocalStr("dns_over_https.mode", "secure".into())));
        assert!(list.contains(&Write::LocalStr("dns_over_https.templates", "https://dns.quad9.net/dns-query".into())));
        assert!(list.contains(&Write::Int("net.network_prediction_options", 3)));
        assert!(list.contains(&Write::LocalInt("performance_tuning.high_efficiency_mode.state", 0)));
        assert!(list.contains(&Write::LocalBool("hardware_acceleration_mode.enabled", false)));
        s.web.secure_dns = SecureDns::Off;
        assert!(writes(&s).contains(&Write::LocalStr("dns_over_https.mode", "off".into())));
        assert_eq!(list.iter().filter(|w| w.is_local()).count(), 4);
    }

    #[test]
    fn site_defaults_skip_states_the_kind_does_not_take() {
        let mut s = Settings::default();
        s.web.site_defaults = vec![
            (Permission::Popups, PermissionState::Allow),
            (Permission::Sound, PermissionState::Ask),
            (Permission::Location, PermissionState::Block),
        ];
        let list = writes(&s);
        assert!(list.contains(&Write::SiteDefault(Permission::Popups, PermissionState::Allow)));
        assert!(list.contains(&Write::SiteDefault(Permission::Location, PermissionState::Block)));
        assert!(!list.iter().any(|w| matches!(w, Write::SiteDefault(Permission::Sound, _))));
    }
}
