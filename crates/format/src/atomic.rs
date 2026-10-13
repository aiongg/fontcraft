//! Atomic file writes: a crash or a full disk never leaves a half-written document. The work is
//! done by the suite's shared `craft-atomic` crate.

use std::path::Path;

use crate::FormatError;

/// Writes `bytes` to a temporary file next to `path`, flushes it to disk and renames it over
/// `path`. The old file stays intact until the new one is complete. A symlink is followed, so the
/// file it points to is replaced and the link stays; the replaced file's permissions are kept. A
/// file the user cannot write is refused rather than replaced.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), FormatError> {
    // craft-atomic's errors already name the file and say whether the original was kept.
    craft_atomic::atomic_write(path, bytes).map_err(|e| FormatError::Io(e.to_string()))
}

/// The browser has no file system to save to. Commands refuse before they get here; this keeps a
/// stray call from reaching `std`, whose process id panics on this target.
#[cfg(target_arch = "wasm32")]
pub fn write_atomic(path: &Path, _bytes: &[u8]) -> Result<(), FormatError> {
    Err(FormatError::Io(format!("cannot save {}: the browser has no file system", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_replaces() {
        let dir = std::env::temp_dir().join(format!("fontcraft-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.fontcraft");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temporary file is left behind");
        assert!(write_atomic(&dir.join("missing-folder").join("x"), b"x").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_stays_a_link_and_permissions_are_kept() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("fontcraft-atomic-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("real.fontcraft");
        let link = dir.join("link.fontcraft");
        std::fs::write(&target, b"old").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write_atomic(&link, b"new").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o640);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_file_is_refused_and_kept() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("fontcraft-atomic-ro-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("locked.fontcraft");
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        // Root may write any file; the refusal only applies to users the mode binds.
        if std::fs::OpenOptions::new().write(true).open(&path).is_err() {
            let err = write_atomic(&path, b"new").unwrap_err().to_string();
            assert!(err.contains("read-only"), "{err}");
            assert_eq!(std::fs::read(&path).unwrap(), b"old");
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temporary file is left behind");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
