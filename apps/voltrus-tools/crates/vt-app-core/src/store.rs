//! Where the workspace lives on disk.
//!
//! Saves are atomic (write a sibling, then rename). A file that no longer
//! parses is moved aside, never overwritten, so a bug can't eat someone's
//! connection list.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::Workspace;

const FILE: &str = "workspace.json";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("no folder for app data on this system")]
    NoDataDir,
    #[error("could not read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    /// The file exists but doesn't parse. It was moved to `kept`.
    #[error("{path} could not be read ({reason}); it was kept as {kept}")]
    Corrupt {
        path: PathBuf,
        kept: PathBuf,
        reason: String,
    },
    #[error("could not save {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
}

/// `<data dir>/Voltrus Tools/workspace.json`, e.g. under
/// `~/Library/Application Support` on macOS.
///
/// `dirs` builds it from `$HOME`, which the App Sandbox points at the app's
/// container: the Mac App Store build saves under
/// `~/Library/Containers/com.joyodigitama.voltrus-tools/Data/Library/Application Support/`.
/// Everything the app keeps (the licence cache, protocol certificates) goes
/// beside this file.
pub fn default_path() -> Result<PathBuf, StoreError> {
    dirs::data_dir()
        .map(|dir| dir.join("Voltrus Tools").join(FILE))
        .ok_or(StoreError::NoDataDir)
}

/// Load the workspace. A missing file is a fresh start; a corrupt one is
/// moved aside and reported, and the caller starts fresh.
pub fn load(path: &Path) -> Result<Workspace, StoreError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Workspace::default()),
        Err(source) => {
            return Err(StoreError::Read {
                path: path.into(),
                source,
            });
        }
    };
    match serde_json::from_str::<Workspace>(&text) {
        Ok(mut workspace) => {
            workspace.repair();
            Ok(workspace)
        }
        Err(error) => {
            let kept = kept_path(path);
            fs::rename(path, &kept).map_err(|source| StoreError::Write {
                path: kept.clone(),
                source,
            })?;
            Err(StoreError::Corrupt {
                path: path.into(),
                kept,
                reason: error.to_string(),
            })
        }
    }
}

pub fn save(path: &Path, workspace: &Workspace) -> Result<(), StoreError> {
    let write = || -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_vec_pretty(workspace).map_err(io::Error::other)?;
        let staging = path.with_extension("json.saving");
        let mut file = create_private(&staging)?;
        file.write_all(&text)?;
        file.sync_all()?;
        fs::rename(&staging, path)
    };
    write().map_err(|source| StoreError::Write {
        path: path.into(),
        source,
    })
}

/// Create (or truncate) `path` readable by its owner only: the workspace holds
/// passwords, NKey seeds and `.creds`.
fn create_private(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        options.mode(0o600);
        let file = options.open(path)?;
        // `mode` only applies to a new file; a staging file left by a crash
        // keeps its own.
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        options.open(path)
    }
}

fn kept_path(path: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    path.with_extension(format!("broken-{stamp}.json"))
}

#[cfg(test)]
mod tests {
    use vt_protocol::{ModbusMasterSpec, SessionSpec};

    use super::*;

    #[test]
    fn a_saved_workspace_loads_back_identical() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(FILE);
        let mut workspace = Workspace::default();
        let plc = workspace.add_connection(
            "PLC",
            SessionSpec::ModbusMaster(ModbusMasterSpec::default()),
        );
        workspace.open(plc);
        save(&path, &workspace).unwrap();
        assert_eq!(load(&path).unwrap(), workspace);
    }

    #[cfg(unix)]
    #[test]
    fn the_workspace_file_is_private_to_its_owner() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        // A staging file left behind by a crash, readable by everyone.
        fs::write(path.with_extension("json.saving"), "").unwrap();
        fs::set_permissions(
            path.with_extension("json.saving"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        save(&path, &Workspace::default()).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_corrupt_file_is_kept_aside_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        fs::write(&path, "{ not json").unwrap();
        let Err(StoreError::Corrupt { kept, .. }) = load(&path) else {
            panic!("corrupt file must be reported");
        };
        assert!(!path.exists());
        assert_eq!(fs::read_to_string(kept).unwrap(), "{ not json");
        // The next start is a fresh one.
        assert_eq!(load(&path).unwrap(), Workspace::default());
    }
}
