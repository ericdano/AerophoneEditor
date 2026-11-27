//! Where the app keeps its settings, setlists and caches. The first call
//! moves the folders used before the app was renamed (AE-30 Patch Editor)
//! to the new ones, so nothing is lost.

use std::sync::Once;

use directories::ProjectDirs;

/// The app's folders (macOS: ~/Library/Application Support and
/// ~/Library/Caches/com.ericdannewitz.AerophoneEditor; Windows:
/// %APPDATA%\ericdannewitz\AerophoneEditor).
pub fn dirs() -> Option<ProjectDirs> {
    static MIGRATE: Once = Once::new();
    let new = ProjectDirs::from("com", "ericdannewitz", "AerophoneEditor")?;
    MIGRATE.call_once(|| migrate(&new));
    Some(new)
}

fn migrate(new: &ProjectDirs) {
    let Some(old) = ProjectDirs::from("com.github", "ericdano", "AE30PatchEditor") else {
        return;
    };
    // On macOS the config and data folders are the same; once moved, the
    // old one no longer exists.
    for (from, to) in [
        (old.config_dir(), new.config_dir()),
        (old.data_dir(), new.data_dir()),
        (old.cache_dir(), new.cache_dir()),
    ] {
        if from.exists() && !to.exists() {
            if let Some(parent) = to.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::rename(from, to);
        }
    }
}
