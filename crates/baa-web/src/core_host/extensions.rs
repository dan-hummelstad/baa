//! Extensions as BaaCore lists them (the same rows serve the toolbar and the
//! extensions page).

use std::{ffi::c_void, path::PathBuf};

use gpui::Modifiers;

use super::{
    ffi::{self, Core, Profile, TabId},
    from_c,
};
use crate::{Extension, ExtensionAction, ExtensionChange, ExtensionCommand, ExtensionInfo, SiteAccess};

/// Every user-visible extension of `profile`, in Chrome's order.
pub fn read(core: &Core, profile: *mut Profile) -> Vec<(Extension, ExtensionInfo)> {
    extern "C" fn visit(out: *mut c_void, extension: *const ffi::Extension) {
        let out = unsafe { &mut *out.cast::<Vec<(Extension, ExtensionInfo)>>() };
        out.push(convert(unsafe { &*extension }));
    }
    let mut out = Vec::new();
    (core.profile_extensions)(profile, visit, (&mut out as *mut Vec<_>).cast());
    out
}

fn convert(raw: &ffi::Extension) -> (Extension, ExtensionInfo) {
    let text = |ptr| unsafe { from_c(ptr) };
    let some = |ptr| Some(text(ptr)).filter(|s| !s.is_empty());
    let id = text(raw.id);
    let name = text(raw.name);
    (
        Extension {
            id: id.clone(),
            name: name.clone(),
            icon: some(raw.icon_path).map(PathBuf::from),
            popup: some(raw.popup_url),
            options: some(raw.options_url),
        },
        ExtensionInfo {
            id,
            name,
            description: text(raw.description),
            version: text(raw.version),
            enabled: raw.enabled,
            permissions: text(raw.permissions).lines().filter(|l| !l.is_empty()).map(String::from).collect(),
            site_access: match raw.site_access {
                ffi::SITE_ACCESS_ON_CLICK => Some(SiteAccess::OnClick),
                ffi::SITE_ACCESS_ON_SPECIFIC_SITES => Some(SiteAccess::OnSpecificSites),
                ffi::SITE_ACCESS_ON_ALL_SITES => Some(SiteAccess::OnAllSites),
                _ => None,
            },
            options: some(raw.options_url),
            unpacked: raw.unpacked,
            needs_approval: raw.needs_approval,
        },
    )
}

/// Tab 0: the defaults.
pub fn actions(core: &Core, profile: *mut Profile, tab: TabId) -> Vec<ExtensionAction> {
    extern "C" fn visit(out: *mut c_void, action: *const ffi::Action) {
        let (out, raw) = unsafe { (&mut *out.cast::<Vec<ExtensionAction>>(), &*action) };
        let colour = |rgba: u32| (rgba & 0xff != 0).then_some(rgba);
        out.push(unsafe {
            ExtensionAction {
                extension_id: from_c(raw.extension_id),
                badge_text: from_c(raw.badge_text),
                title: from_c(raw.title),
                badge_rgba: colour(raw.badge_rgba),
                text_rgba: colour(raw.text_rgba),
                enabled: raw.enabled,
            }
        });
    }
    let mut out = Vec::new();
    (core.profile_extension_actions)(profile, tab, visit, (&mut out as *mut Vec<_>).cast());
    out
}

pub fn commands(core: &Core, profile: *mut Profile) -> Vec<ExtensionCommand> {
    extern "C" fn visit(out: *mut c_void, command: *const ffi::Command) {
        let (out, raw) = unsafe { (&mut *out.cast::<Vec<ExtensionCommand>>(), &*command) };
        out.push(unsafe {
            ExtensionCommand {
                extension_id: from_c(raw.extension_id),
                name: from_c(raw.name),
                description: from_c(raw.description),
                key: from_c(raw.key),
                modifiers: modifiers_of(raw.modifiers),
                action: raw.action,
            }
        });
    }
    let mut out = Vec::new();
    (core.profile_extension_commands)(profile, visit, (&mut out as *mut Vec<_>).cast());
    out
}

pub fn modifiers_of(bits: u32) -> Modifiers {
    Modifiers {
        platform: bits & ffi::MOD_CMD != 0,
        control: bits & ffi::MOD_CTRL != 0,
        alt: bits & ffi::MOD_ALT != 0,
        shift: bits & ffi::MOD_SHIFT != 0,
        function: false,
    }
}

/// Fn has no `BAA_MOD_*` bit and is dropped.
pub fn modifier_bits(modifiers: Modifiers) -> u32 {
    [(modifiers.platform, ffi::MOD_CMD), (modifiers.control, ffi::MOD_CTRL), (modifiers.alt, ffi::MOD_ALT), (modifiers.shift, ffi::MOD_SHIFT)]
        .iter()
        .filter(|(on, _)| *on)
        .fold(0, |bits, (_, bit)| bits | bit)
}

/// `BAA_EXTENSION_*` change kind and value.
pub fn change_code(change: ExtensionChange) -> (i32, i32) {
    match change {
        ExtensionChange::Enable(on) => (ffi::EXTENSION_ENABLE, on as i32),
        ExtensionChange::Remove => (ffi::EXTENSION_REMOVE, 0),
        ExtensionChange::Reload => (ffi::EXTENSION_RELOAD, 0),
        ExtensionChange::SiteAccess(access) => (
            ffi::EXTENSION_SITE_ACCESS,
            match access {
                SiteAccess::OnClick => ffi::SITE_ACCESS_ON_CLICK,
                SiteAccess::OnSpecificSites => ffi::SITE_ACCESS_ON_SPECIFIC_SITES,
                SiteAccess::OnAllSites => ffi::SITE_ACCESS_ON_ALL_SITES,
            },
        ),
    }
}

/// The extension a popup url belongs to.
pub fn id_of(url: &str) -> Option<&str> {
    url.strip_prefix("chrome-extension://")?.split('/').next().filter(|id| !id.is_empty())
}
