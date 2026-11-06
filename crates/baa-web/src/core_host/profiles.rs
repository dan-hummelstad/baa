//! Where Spaces live on disk: one Chrome profile directory per Space under
//! `<data>/chromium`, deleted at the next launch, and the one-time move from
//! the old CEF layout.

use std::path::{Path, PathBuf};

use crate::ProfileId;

// Ids count down from u64::MAX; baa-model's count up from 1.
pub(crate) fn is_ephemeral(id: ProfileId) -> bool {
    id.0 > u64::MAX / 2
}

pub(crate) fn profile_dir(root: &Path, id: ProfileId) -> PathBuf {
    // Chrome rejects nested profile paths ("Cannot create profile"), so no spaces/ subdir.
    root.join(format!("space-{}", id.0))
}

/// Password managers (1Password, Bitwarden, ...) talk to their browser
/// extensions through a native messaging host they register with Chrome and
/// its relatives. Chromium reads `<user data>/NativeMessagingHosts`, so those
/// registrations are copied there and the extensions work as in Chrome.
/// (Apple's iCloud Passwords host cannot: macOS only lets Chrome and Edge
/// start it.)
pub fn share_native_messaging_hosts(root: &Path) {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let support = home.join("Library/Application Support");
    let sources: Vec<PathBuf> = [
        "Google/Chrome",
        "Chromium",
        "Microsoft Edge",
        "BraveSoftware/Brave-Browser",
        "Vivaldi",
    ]
    .iter()
    .map(|b| support.join(b).join("NativeMessagingHosts"))
    .chain(
        [
            "/Library/Google/Chrome/NativeMessagingHosts",
            "/Library/Application Support/Chromium/NativeMessagingHosts",
            "/Library/Microsoft/Edge/NativeMessagingHosts",
        ]
        .map(PathBuf::from),
    )
    .collect();
    let dest = root.join("NativeMessagingHosts");
    // Refreshed every launch (an update can move a host); the first browser listing one wins.
    let mut copied = std::collections::HashSet::new();
    for source in sources {
        let Ok(entries) = std::fs::read_dir(source) else { continue };
        let _ = std::fs::create_dir_all(&dest);
        for entry in entries.flatten() {
            let name = entry.file_name();
            if entry.path().extension().is_some_and(|e| e == "json") && copied.insert(name.clone()) {
                let _ = std::fs::copy(entry.path(), dest.join(name));
            }
        }
    }
}

const PENDING_DELETES: &str = "pending-deletes";

pub(crate) fn queue_delete(root: &Path, id: ProfileId) {
    use std::io::Write as _;
    let line = format!("{}\n", id.0);
    let written = std::fs::create_dir_all(root).and_then(|()| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join(PENDING_DELETES))?
            .write_all(line.as_bytes())
    });
    if let Err(err) = written {
        log::warn!("could not queue profile {id:?} for deletion: {err}");
    }
}

/// Runs before Chromium opens anything in `root`, so nothing holds the files.
pub fn purge_deleted_profiles(root: &Path) {
    let list = root.join(PENDING_DELETES);
    let Ok(text) = std::fs::read_to_string(&list) else {
        return;
    };
    // Parsing as a number keeps a tampered file from naming another path.
    for id in text.lines().filter_map(|line| line.trim().parse().ok()) {
        let dir = profile_dir(root, ProfileId(id));
        if let Err(err) = std::fs::remove_dir_all(&dir) {
            if dir.exists() {
                log::warn!("could not delete {}: {err}", dir.display());
            }
        }
    }
    let _ = std::fs::remove_file(list);
}

/// Moves `<data>/profile/space-*` (the CEF layout) to `<data>/chromium/`, once.
/// Cookies do not survive: CEF encrypted them with the mock keychain key, and
/// Chromium drops what it cannot decrypt, so sites ask to sign in again.
// ponytail: delete this function in the first release after v0.2.
pub fn migrate_cef_profiles(data: &Path) {
    let old = data.join("profile");
    if !old.exists() {
        return;
    }
    // Spaces deleted under CEF stay deleted.
    purge_deleted_profiles(&old);
    let new = data.join("chromium");
    let mut failed = false;
    for entry in std::fs::read_dir(&old).into_iter().flatten().flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("space-") || !entry.path().is_dir() {
            continue;
        }
        // A dev's earlier test data wins.
        if new.join(&name).exists() {
            continue;
        }
        let moved = std::fs::create_dir_all(&new).and_then(|()| std::fs::rename(entry.path(), new.join(&name)));
        match moved {
            Ok(()) => log::info!("migrated {}", name.to_string_lossy()),
            Err(err) => {
                log::warn!("could not migrate {}: {err}", name.to_string_lossy());
                failed = true;
            }
        }
    }
    // Never delete before every move has succeeded.
    if !failed {
        let _ = std::fs::remove_dir_all(&old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("baa-web-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn ephemeral_ids_never_meet_model_ids() {
        assert!(is_ephemeral(ProfileId(u64::MAX)));
        assert!(!is_ephemeral(ProfileId(1)));
        assert!(!is_ephemeral(ProfileId(1 << 40)));
    }

    #[test]
    fn queued_deletes_run_at_next_launch() {
        let root = temp("purge");
        let keep = profile_dir(&root, ProfileId(1));
        let gone = profile_dir(&root, ProfileId(2));
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::create_dir_all(gone.join("Cache")).unwrap();

        queue_delete(&root, ProfileId(2));
        std::fs::write(
            root.join(PENDING_DELETES),
            std::fs::read_to_string(root.join(PENDING_DELETES)).unwrap() + "../../etc\n",
        )
        .unwrap();
        purge_deleted_profiles(&root);
        share_native_messaging_hosts(&root);

        assert!(keep.exists());
        assert!(!gone.exists());
        assert!(!root.join(PENDING_DELETES).exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn migration_moves_spaces_and_keeps_existing_and_deleted_ones() {
        let data = temp("migrate");
        let old = data.join("profile");
        for space in ["space-1", "space-2", "space-3"] {
            std::fs::create_dir_all(old.join(space).join("Extensions")).unwrap();
            std::fs::write(old.join(space).join("Preferences"), space).unwrap();
        }
        std::fs::write(old.join("Local State"), "cef").unwrap();
        // space-2 exists under chromium/ already; space-3 was deleted under CEF.
        std::fs::create_dir_all(data.join("chromium/space-2")).unwrap();
        std::fs::write(data.join("chromium/space-2/Preferences"), "new").unwrap();
        queue_delete(&old, ProfileId(3));

        migrate_cef_profiles(&data);

        let read = |p: &str| std::fs::read_to_string(data.join(p)).unwrap();
        assert_eq!(read("chromium/space-1/Preferences"), "space-1");
        assert!(data.join("chromium/space-1/Extensions").is_dir());
        assert_eq!(read("chromium/space-2/Preferences"), "new");
        assert!(!data.join("chromium/space-3").exists());
        assert!(!old.exists());

        // A second call has nothing to do.
        migrate_cef_profiles(&data);
        assert_eq!(read("chromium/space-1/Preferences"), "space-1");
        std::fs::remove_dir_all(&data).unwrap();
    }
}
