//! Atomic file writes: a crash or a full disk never leaves a half-written document.

use std::io::Write as _;
use std::path::Path;

use crate::FormatError;

/// Writes `bytes` to a temporary file next to `path`, flushes it to disk and renames it over
/// `path`. The old file stays intact until the new one is complete. A symlink is followed, so the
/// file it points to is replaced and the link stays; the replaced file's permissions are kept.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), FormatError> {
    let resolved;
    let path = match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            resolved = std::fs::canonicalize(path).map_err(|e| FormatError::Io(format!("cannot follow the link {}: {e}", path.display())))?;
            resolved.as_path()
        }
        _ => path,
    };
    let old_permissions = std::fs::metadata(path).ok().map(|m| m.permissions());
    let io = |what: &str, e: std::io::Error| FormatError::Io(format!("{what} {}: {e}", path.display()));
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().ok_or_else(|| FormatError::Io(format!("{} is not a file path", path.display())))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(format!(".{}.tmp", std::process::id()));
    let tmp = dir.join(tmp_name);
    let result = (|| {
        let mut f = std::fs::File::create(&tmp).map_err(|e| io("cannot create a temporary file for", e))?;
        f.write_all(bytes).map_err(|e| io("cannot write", e))?;
        f.sync_all().map_err(|e| io("cannot flush", e))?;
        if let Some(perm) = &old_permissions {
            std::fs::set_permissions(&tmp, perm.clone()).map_err(|e| io("cannot keep the permissions of", e))?;
        }
        std::fs::rename(&tmp, path).map_err(|e| io("cannot replace", e))
    })();
    if result.is_err() {
        // Best effort: the error we report is the write failure, not the clean-up.
        let _ = std::fs::remove_file(&tmp);
    }
    result
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
}
