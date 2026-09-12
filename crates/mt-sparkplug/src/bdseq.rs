//! bdSeq persistence: the birth/death sequence counter must survive restarts
//! so each broker session gets a fresh bdSeq (hosts use it to tell sessions
//! apart). Stored as a decimal text file, one `u8` — the spec keeps bdSeq in
//! 0..=255 with wraparound.

use std::path::Path;

use crate::SparkplugError;

/// Load the persisted bdSeq; a missing file means "first session" → 0.
pub fn load(path: &Path) -> Result<u8, SparkplugError> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .trim()
            .parse::<u8>()
            .map_err(|_| SparkplugError::BdSeqCorrupt { path: path.to_path_buf() }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(source) => Err(SparkplugError::BdSeqIo {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Persist the bdSeq, creating parent directories as needed.
pub fn store(path: &Path, bd_seq: u8) -> Result<(), SparkplugError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|source| SparkplugError::BdSeqIo {
            path: dir.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(path, bd_seq.to_string()).map_err(|source| SparkplugError::BdSeqIo {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_starts_at_zero() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(&dir.path().join("absent")).unwrap(), 0);
    }

    #[test]
    fn store_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/bd_seq");
        store(&path, 42).unwrap();
        assert_eq!(load(&path).unwrap(), 42);
    }

    #[test]
    fn corrupt_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bd_seq");
        std::fs::write(&path, "not-a-number").unwrap();
        assert!(matches!(load(&path), Err(SparkplugError::BdSeqCorrupt { .. })));
    }

    #[test]
    fn counter_wraps_at_256() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bd_seq");
        store(&path, 255).unwrap();
        assert_eq!(load(&path).unwrap().wrapping_add(1), 0);
    }
}
