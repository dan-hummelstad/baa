use serde::{Deserialize, Serialize};

/// A kind of site permission or content setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    Camera,
    Microphone,
    Location,
    Notifications,
    Clipboard,
    Midi,
    /// Anything rarer a site can ask for. Never a site setting.
    Other,
    // The kinds below are site settings only: a site never asks for them.
    Popups,
    Sound,
    JavaScript,
    Cookies,
    AutomaticDownloads,
    InsecureContent,
    Sensors,
    WindowManagement,
}

/// What a profile remembers about a site's permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionState {
    Ask,
    Allow,
    Block,
    /// Cookies only: kept until the browser quits.
    SessionOnly,
}

impl Permission {
    /// The site settings in the order the settings window lists them.
    pub const SITE_KINDS: [Permission; 14] = [
        Self::Location,
        Self::Camera,
        Self::Microphone,
        Self::Notifications,
        Self::Clipboard,
        Self::Midi,
        Self::Popups,
        Self::Sound,
        Self::JavaScript,
        Self::Cookies,
        Self::AutomaticDownloads,
        Self::InsecureContent,
        Self::Sensors,
        Self::WindowManagement,
    ];

    /// Whether `state` is a valid exception for this kind: Chrome's `valid_settings` per type
    /// (content_settings_registry.cc). Cookies never ask; sensors do.
    pub fn takes(self, state: PermissionState) -> bool {
        match self {
            Self::Popups | Self::Sound | Self::JavaScript | Self::InsecureContent => {
                matches!(state, PermissionState::Allow | PermissionState::Block)
            }
            Self::Cookies => state != PermissionState::Ask,
            _ => state != PermissionState::SessionOnly,
        }
    }

    /// Whether `state` is a valid site default: as [`Self::takes`], except Chrome refuses an
    /// always-allow default for camera and microphone (`IsDefaultSettingValid`).
    pub fn takes_default(self, state: PermissionState) -> bool {
        self.takes(state) && !(matches!(self, Self::Camera | Self::Microphone) && state == PermissionState::Allow)
    }

    /// Chrome's default for a profile whose user changed nothing.
    // ponytail: from the Chrome 156 source; the harness test in WP-B checks it against the engine.
    pub fn chrome_default(self) -> PermissionState {
        match self {
            Self::Popups | Self::InsecureContent => PermissionState::Block,
            Self::Sound | Self::JavaScript | Self::Cookies | Self::Sensors => PermissionState::Allow,
            _ => PermissionState::Ask,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_rules() {
        for kind in Permission::SITE_KINDS {
            assert!(kind.takes_default(kind.chrome_default()), "{kind:?}");
        }
    }

    #[test]
    fn validity_follows_chrome() {
        use PermissionState::*;
        assert!(!Permission::Cookies.takes(Ask));
        assert!(Permission::Cookies.takes(SessionOnly));
        assert!(Permission::Sensors.takes(Ask));
        assert!(!Permission::Popups.takes(Ask) && !Permission::Popups.takes(SessionOnly));
        assert!(Permission::Camera.takes(Allow) && !Permission::Camera.takes_default(Allow));
        assert!(Permission::Microphone.takes_default(Block) && !Permission::Microphone.takes_default(Allow));
        assert!(!Permission::Location.takes(SessionOnly));
    }
}
