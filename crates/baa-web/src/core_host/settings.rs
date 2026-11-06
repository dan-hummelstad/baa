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
    /// Back to Chrome's value.
    Clear(&'static str),
    /// `None` restores Chrome's default for the kind.
    SiteDefault(Permission, Option<PermissionState>),
    Engine { name: String, url: String },
    Zoom(f64),
}

/// Everything baa pushes: per profile, and once into Local State.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Writes {
    pub profile: Vec<Write>,
    pub local: Vec<Write>,
}

pub fn writes(settings: &Settings) -> Writes {
    let web = &settings.web;
    let mut profile = vec![Write::Int(
        "profile.cookie_controls_mode",
        if web.cookies == Cookies::AllowAll { 0 } else { 1 },
    )];
    let (strict, balanced) = (web.https == Https::Strict, web.https == Https::Balanced);
    profile.push(Write::Bool("https_only_mode_enabled", strict));
    profile.push(Write::Bool("https_first_balanced_mode_enabled", balanced));
    profile.push(Write::Int(
        "net.network_prediction_options",
        match web.preload {
            Preload::Standard => 0,
            Preload::Off => 2,
            Preload::Extended => 3,
        },
    ));
    profile.push(Write::Bool("enable_do_not_track", web.do_not_track));
    profile.push(Write::Bool("helium.global_privacy_control", web.global_privacy_control));
    profile.push(Write::Bool("download.prompt_for_download", web.ask_where_to_save));
    profile.push(match &web.download_dir {
        Some(dir) => Write::Str("download.default_directory", dir.to_string_lossy().into_owned()),
        None => Write::Clear("download.default_directory"),
    });
    profile.push(match &web.languages {
        Some(languages) => Write::Str("intl.accept_languages", languages.clone()),
        None => Write::Clear("intl.accept_languages"),
    });
    profile.push(Write::Bool("browser.enable_spellchecking", web.spellcheck));
    profile.push(Write::Int("webkit.webprefs.default_font_size", web.font_size as i32));
    profile.push(Write::Int("webkit.webprefs.minimum_font_size", web.min_font_size as i32));
    profile.push(Write::Bool("settings.a11y.focus_highlight", web.focus_highlight));
    profile.push(Write::Bool("settings.a11y.caretbrowsing.enabled", web.caret_browsing));
    profile.push(Write::Bool("extensions.ui.developer_mode", web.developer_mode));
    profile.push(Write::Zoom(web.default_zoom));
    profile.push(Write::Engine { name: settings.search_engine.name.clone(), url: settings.search_engine.url.clone() });
    // Every kind, so one the user stopped overriding is restored. Block wins over
    // session-only, and either over a default set in site settings.
    for kind in Permission::SITE_KINDS {
        let chosen = web.site_defaults.iter().find(|(k, _)| *k == kind).map(|(_, s)| *s).filter(|s| kind.takes(*s));
        let state = match (kind, web.cookies, web.clear_cookies_on_quit) {
            (Permission::Cookies, Cookies::BlockAll, _) => Some(PermissionState::Block),
            (Permission::Cookies, _, true) => Some(PermissionState::SessionOnly),
            _ => chosen,
        };
        profile.push(Write::SiteDefault(kind, state));
    }

    let (mode, templates) = match &web.secure_dns {
        SecureDns::Off => ("off", String::new()),
        SecureDns::Automatic => ("automatic", String::new()),
        SecureDns::Custom(template) => ("secure", template.clone()),
    };
    // Templates first: a secure mode without them is briefly broken.
    let local = vec![
        Write::Str("dns_over_https.templates", templates),
        Write::Str("dns_over_https.mode", mode.into()),
        Write::Bool("hardware_acceleration_mode.enabled", web.hardware_acceleration),
        Write::Int("performance_tuning.high_efficiency_mode.state", if web.memory_saver { 2 } else { 0 }),
    ];
    Writes { profile, local }
}

/// `baa_profile_set_color_schemes` code for dark or light; 0 follows the system.
pub fn scheme_code(dark: bool) -> c_int {
    if dark { 2 } else { 1 }
}

/// prefers-color-scheme that websites see.
pub fn website_scheme(scheme: PageScheme) -> c_int {
    match scheme {
        PageScheme::System => 0,
        PageScheme::Light => scheme_code(false),
        PageScheme::Dark => scheme_code(true),
    }
}

/// Sends `write` to `profile`; null is Local State.
pub fn apply(core: &Core, profile: *mut Profile, write: &Write) {
    let accepted = match write {
        Write::Bool(name, v) => (core.pref_set_bool)(profile, c_string(name).as_ptr(), *v),
        Write::Int(name, v) => (core.pref_set_int)(profile, c_string(name).as_ptr(), *v),
        Write::Str(name, v) => (core.pref_set_string)(profile, c_string(name).as_ptr(), c_string(v).as_ptr()),
        Write::Clear(name) => (core.pref_clear)(profile, c_string(name).as_ptr()),
        Write::SiteDefault(kind, state) => permission_bit(*kind).is_some_and(|bit| {
            (core.profile_set_site_rule)(profile, bit, c"*".as_ptr(), state.map_or(ffi::SITE_UNSET, state_code))
        }),
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

#[cfg(test)]
mod tests {
    use super::*;
    use baa_model::SecureDns;

    fn default_profile_list() -> Vec<Write> {
        use Write::*;
        let mut list = vec![
            Int("profile.cookie_controls_mode", 1),
            Bool("https_only_mode_enabled", false),
            Bool("https_first_balanced_mode_enabled", true),
            Int("net.network_prediction_options", 2),
            Bool("enable_do_not_track", false),
            Bool("helium.global_privacy_control", false),
            Bool("download.prompt_for_download", false),
            Clear("download.default_directory"),
            Clear("intl.accept_languages"),
            Bool("browser.enable_spellchecking", true),
            Int("webkit.webprefs.default_font_size", 16),
            Int("webkit.webprefs.minimum_font_size", 0),
            Bool("settings.a11y.focus_highlight", false),
            Bool("settings.a11y.caretbrowsing.enabled", false),
            Bool("extensions.ui.developer_mode", false),
            Zoom(1.0),
            Engine { name: "Google".into(), url: "https://www.google.com/search?q=%s".into() },
        ];
        list.extend(Permission::SITE_KINDS.map(|kind| SiteDefault(kind, None)));
        list
    }

    fn cookie_default(w: &Writes) -> Option<PermissionState> {
        w.profile.iter().find_map(|w| match w {
            Write::SiteDefault(Permission::Cookies, s) => Some(*s),
            _ => None,
        })?
    }

    #[test]
    fn defaults_are_the_exact_lists() {
        let w = writes(&Settings::default());
        assert_eq!(w.profile, default_profile_list());
        assert_eq!(
            w.local,
            [
                Write::Str("dns_over_https.templates", String::new()),
                Write::Str("dns_over_https.mode", "automatic".into()),
                Write::Bool("hardware_acceleration_mode.enabled", true),
                Write::Int("performance_tuning.high_efficiency_mode.state", 0),
            ]
        );
    }

    #[test]
    fn block_all_beats_clear_on_quit_beats_a_site_default() {
        let mut s = Settings::default();
        s.web.cookies = Cookies::BlockAll;
        s.web.clear_cookies_on_quit = true;
        s.web.site_defaults = vec![(Permission::Cookies, PermissionState::Allow)];
        let w = writes(&s);
        assert!(w.profile.contains(&Write::Int("profile.cookie_controls_mode", 1)));
        assert_eq!(cookie_default(&w), Some(PermissionState::Block));

        s.web.cookies = Cookies::AllowAll;
        let w = writes(&s);
        assert!(w.profile.contains(&Write::Int("profile.cookie_controls_mode", 0)));
        assert_eq!(cookie_default(&w), Some(PermissionState::SessionOnly));

        s.web.clear_cookies_on_quit = false;
        assert_eq!(cookie_default(&writes(&s)), Some(PermissionState::Allow));
        s.web.site_defaults.clear();
        assert_eq!(cookie_default(&writes(&s)), None);
    }

    #[test]
    fn https_modes_write_the_pair_chrome_writes() {
        let pair = |https| {
            let mut s = Settings::default();
            s.web.https = https;
            let list = writes(&s).profile;
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
    fn optional_prefs_are_set_or_cleared() {
        let mut s = Settings::default();
        s.web.languages = Some("en-US, sv".into());
        s.web.download_dir = Some("/tmp/dl".into());
        let list = writes(&s).profile;
        assert!(list.contains(&Write::Str("intl.accept_languages", "en-US, sv".into())));
        assert!(list.contains(&Write::Str("download.default_directory", "/tmp/dl".into())));
        assert!(!list.contains(&Write::Clear("intl.accept_languages")));
    }

    #[test]
    fn local_state_and_enums_map_to_chrome_values() {
        let mut s = Settings::default();
        s.web.secure_dns = SecureDns::Custom("https://dns.quad9.net/dns-query".into());
        s.web.preload = Preload::Extended;
        s.web.memory_saver = false;
        s.web.hardware_acceleration = false;
        let w = writes(&s);
        assert_eq!(
            w.local,
            [
                Write::Str("dns_over_https.templates", "https://dns.quad9.net/dns-query".into()),
                Write::Str("dns_over_https.mode", "secure".into()),
                Write::Bool("hardware_acceleration_mode.enabled", false),
                Write::Int("performance_tuning.high_efficiency_mode.state", 0),
            ]
        );
        assert!(w.profile.contains(&Write::Int("net.network_prediction_options", 3)));
        s.web.secure_dns = SecureDns::Off;
        assert!(writes(&s).local.contains(&Write::Str("dns_over_https.mode", "off".into())));
    }

    #[test]
    fn site_defaults_skip_states_the_kind_does_not_take() {
        let mut s = Settings::default();
        s.web.site_defaults = vec![
            (Permission::Popups, PermissionState::Allow),
            (Permission::Sound, PermissionState::Ask),
            (Permission::Location, PermissionState::Block),
        ];
        let list = writes(&s).profile;
        assert!(list.contains(&Write::SiteDefault(Permission::Popups, Some(PermissionState::Allow))));
        assert!(list.contains(&Write::SiteDefault(Permission::Location, Some(PermissionState::Block))));
        assert!(list.contains(&Write::SiteDefault(Permission::Sound, None)));
    }

    #[test]
    fn one_scheme_mapping() {
        assert_eq!((website_scheme(PageScheme::System), website_scheme(PageScheme::Light), website_scheme(PageScheme::Dark)), (0, 1, 2));
        assert_eq!((scheme_code(false), scheme_code(true)), (1, 2));
    }
}
