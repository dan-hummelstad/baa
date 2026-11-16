use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};

use crate::{
    ChromeStyle, Cookies, DevToolsDock, ExternalLinks, Https, PageScheme, PermissionState, Preload, SecureDns,
    SidebarSide, ThemeMode, Zone,
};

/// A stored enum value this build does not know (one a newer baa wrote) loads as the fallback
/// instead of failing the whole session or settings. The enum derives with
/// `#[serde(remote = "Self")]`, which keeps the derived code as inherent fns these impls call;
/// no catch-all variant, so other crates' `match`es stay as they are.
macro_rules! tolerant {
    ($($t:ident => $fallback:expr),* $(,)?) => {$(
        impl Serialize for $t {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                $t::serialize(self, s)
            }
        }
        impl<'de> Deserialize<'de> for $t {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                Ok($t::deserialize(serde_json::Value::deserialize(d)?).unwrap_or($fallback))
            }
        }
    )*};
}

// The default, or the safest choice: never a weaker cookie, HTTPS or DNS setting than the
// default, and never an allow. Account and Permission have no safe variant (any one would turn
// something on); unknown ones are dropped from their lists by `known` instead.
tolerant! {
    Zone => Zone::Today,
    ThemeMode => ThemeMode::default(),
    SidebarSide => SidebarSide::default(),
    ChromeStyle => ChromeStyle::default(),
    DevToolsDock => DevToolsDock::default(),
    ExternalLinks => ExternalLinks::default(),
    PageScheme => PageScheme::default(),
    Cookies => Cookies::default(),
    Https => Https::default(),
    Preload => Preload::default(),
    SecureDns => SecureDns::default(),
    PermissionState => PermissionState::Ask,
}

/// A list without the entries this build cannot read.
pub(crate) fn known<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    let values = Vec::<serde_json::Value>::deserialize(d)?;
    Ok(values.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
}

/// `None` for a value this build cannot read.
pub(crate) fn known_or_none<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Option<T>, D::Error> {
    Ok(Option::<serde_json::Value>::deserialize(d)?.and_then(|v| serde_json::from_value(v).ok()))
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn stored_values_a_newer_baa_wrote_load_as_their_fallback() {
        use serde_json::{json, Value};
        let unknown = json!("FromANewerBaa");
        let mut settings = serde_json::to_value(Settings::default()).unwrap();
        for path in ["sidebar_side", "chrome", "devtools_dock", "external_links", "page_scheme", "appearance"] {
            settings[path] = unknown.clone();
        }
        for path in ["cookies", "https", "preload", "secure_dns"] {
            settings["web"][path] = unknown.clone();
        }
        settings["shared_accounts"] = json!(["GitHub", "FromANewerBaa"]);
        settings["kept_accounts"] = json!([[3, "Google"], [4, "FromANewerBaa"]]);
        settings["web"]["site_defaults"] = json!([
            ["Location", "FromANewerBaa"],
            ["Cookies", "FromANewerBaa"],
            ["FromANewerBaa", "Allow"],
            ["Popups", "Allow"],
        ]);
        let back: Settings = serde_json::from_value(settings).unwrap();
        let expected = Settings {
            shared_accounts: vec![Account::GitHub],
            kept_accounts: vec![(ProfileId(3), Account::Google)],
            web: WebPrefs {
                site_defaults: vec![
                    (Permission::Location, PermissionState::Ask),
                    (Permission::Popups, PermissionState::Allow),
                ],
                ..WebPrefs::default()
            },
            ..Settings::default()
        };
        assert_eq!(back, expected);
        // The data-carrying one, with an unknown variant that carries data too.
        assert_eq!(serde_json::from_value::<SecureDns>(json!({ "Doh3": "x" })).unwrap(), SecureDns::Automatic);

        let mut s = Session::first_run();
        let sp = s.active_space().unwrap();
        s.open_tab(sp, "https://a/", 0);
        let live = s.add_live_folder(sp, "Feed", LiveSource::Feed { url: "https://f/".into() });
        let mut session = serde_json::to_value(&s).unwrap();
        session["tabs"][0]["zone"] = unknown.clone();
        session["spaces"][0]["mode"] = unknown.clone();
        session["spaces"][0]["sidebar_side"] = unknown.clone();
        session["folders"][0]["live"] = json!({ "FromANewerBaa": { "id": 1 } });
        let back: Session = serde_json::from_value(session).unwrap();
        assert_eq!(back.tabs[0].zone, Zone::Today);
        assert_eq!(back.spaces[0].mode, ThemeMode::Auto);
        assert_eq!(back.spaces[0].sidebar_side, Some(SidebarSide::Left));
        assert_eq!(back.folder(live).unwrap().live, None, "a plain folder");

        // Known values still round-trip through the tolerant impls.
        let known = Settings { chrome: ChromeStyle::Ascii, web: WebPrefs { secure_dns: SecureDns::Custom("https://d/".into()), ..WebPrefs::default() }, ..Settings::default() };
        let v: Value = serde_json::to_value(&known).unwrap();
        assert_eq!(v["chrome"], "Ascii");
        assert_eq!(serde_json::from_value::<Settings>(v).unwrap(), known);
    }
}
