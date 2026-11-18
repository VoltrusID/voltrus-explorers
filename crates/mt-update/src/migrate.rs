//! Moving an app's data out of its App Sandbox container.
//!
//! A DMG build that was sandboxed (CAN Explorer up to 1.1.x) kept everything under
//! `~/Library/Containers/<id>/Data/`: the web view's local storage (saved profiles, transmit lists, settings) and the
//! app data directory. The same app without the sandbox reads `~/Library/...` instead, so on its first launch it
//! would look empty. [`from_sandbox_container`] copies the old data across once, before the web view starts. The
//! container is left as it was, so going back to an older sandboxed version still finds its data.
//!
//! Licence keys are not files: they are Keychain items of the app (see `mt-store`), found by the unsandboxed app as
//! long as it is signed by the same team with the same bundle id.

use std::path::{Path, PathBuf};

/// Pairs of (path inside the container's `Data`, path under the home folder) to copy for bundle id `id`.
fn plan(id: &str) -> Vec<(PathBuf, PathBuf)> {
    vec![
        // WebKit keeps a sandboxed app's website data without the bundle id level (the container is per app).
        (PathBuf::from("Library/WebKit/WebsiteData"), PathBuf::from(format!("Library/WebKit/{id}/WebsiteData"))),
        (PathBuf::from(format!("Library/WebKit/{id}")), PathBuf::from(format!("Library/WebKit/{id}"))),
        (PathBuf::from(format!("Library/Application Support/{id}")), PathBuf::from(format!("Library/Application Support/{id}"))),
        (PathBuf::from(format!("Library/Preferences/{id}.plist")), PathBuf::from(format!("Library/Preferences/{id}.plist"))),
    ]
}

/// Copies the data of the sandboxed app `id` into the unsandboxed locations, once. Call it first thing in `run()`,
/// before the Tauri builder creates the web view. Does nothing when the app runs sandboxed, when there is no
/// container, or when a destination already exists (a later launch, or data the unsandboxed app made itself).
/// Returns what was copied and what failed, for logging.
pub fn from_sandbox_container(id: &str) -> Report {
    if std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_some() {
        return Report::default();
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Report::default() };
    copy_plan(&home, &home.join("Library/Containers").join(id).join("Data"), id)
}

#[derive(Debug, Default)]
pub struct Report {
    pub copied: Vec<PathBuf>,
    /// Destination and error of each copy that failed (the destination is left absent, so the next launch retries).
    pub failed: Vec<(PathBuf, String)>,
}

impl Report {
    /// Writes the failures to stderr (Console.app shows them for the app).
    pub fn log(&self) {
        for (path, e) in &self.failed {
            eprintln!("mt-update: could not copy sandbox data to {}: {e}", path.display());
        }
    }
}

fn copy_plan(home: &Path, container: &Path, id: &str) -> Report {
    let mut report = Report::default();
    if !container.is_dir() {
        return report;
    }
    for (from, to) in plan(id) {
        let (src, dst) = (container.join(&from), home.join(&to));
        // A symlink in the container (macOS links ~/Library/Preferences-style folders there) is not data of ours.
        let Ok(meta) = std::fs::symlink_metadata(&src) else { continue };
        if meta.file_type().is_symlink() || dst.exists() {
            continue;
        }
        if let Some(parent) = dst.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                report.failed.push((dst, e.to_string()));
                continue;
            }
        }
        // Copy into a temporary name and rename, so a quit halfway leaves no half-copied folder to block a retry.
        let tmp = dst.with_extension("mt-migrating");
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_file(&tmp);
        match copy_tree(&src, &tmp).and_then(|()| std::fs::rename(&tmp, &dst)) {
            Ok(()) => report.copied.push(dst),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
                let _ = std::fs::remove_file(&tmp);
                report.failed.push((dst, e.to_string()));
            }
        }
    }
    report
}

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mt-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copies_web_storage_and_app_data_once() {
        let home = scratch("once");
        let id = "com.example.app";
        let container = home.join("Library/Containers").join(id).join("Data");
        write(&container.join("Library/WebKit/WebsiteData/Default/salt"), "s");
        write(&container.join("Library/WebKit/WebsiteData/Default/a/b/LocalStorage/localstorage.sqlite3"), "profiles");
        write(&container.join(format!("Library/Application Support/{id}/licence/key")), "k");

        let report = copy_plan(&home, &container, id);
        assert_eq!(report.copied.len(), 2, "{report:?}");
        assert!(report.failed.is_empty());
        let ls = home.join(format!("Library/WebKit/{id}/WebsiteData/Default/a/b/LocalStorage/localstorage.sqlite3"));
        assert_eq!(std::fs::read_to_string(&ls).unwrap(), "profiles");
        assert!(home.join(format!("Library/Application Support/{id}/licence/key")).exists());
        // The container is untouched.
        assert!(container.join("Library/WebKit/WebsiteData/Default/salt").exists());

        // Second launch: destinations exist, nothing is overwritten.
        std::fs::write(&ls, "newer").unwrap();
        assert!(copy_plan(&home, &container, id).copied.is_empty());
        assert_eq!(std::fs::read_to_string(&ls).unwrap(), "newer");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn no_container_no_copy() {
        let home = scratch("none");
        assert!(copy_plan(&home, &home.join("Library/Containers/x/Data"), "x").copied.is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_in_the_container_are_skipped() {
        let home = scratch("links");
        let id = "com.example.app";
        let container = home.join("Library/Containers").join(id).join("Data");
        write(&home.join("elsewhere/WebsiteData/x"), "x");
        std::fs::create_dir_all(container.join("Library/WebKit")).unwrap();
        std::os::unix::fs::symlink(home.join("elsewhere/WebsiteData"), container.join("Library/WebKit/WebsiteData")).unwrap();
        assert!(copy_plan(&home, &container, id).copied.is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }
}
