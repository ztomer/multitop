//! Writing a vault file without ever leaving a half-written one in its place.
//!
//! Temp file, fsync, rename, fsync the directory. The temp file is locked while
//! it is being written, which is what tells a later writer whether a leftover
//! belongs to a live process or to one that died.

use std::io::Write as _;

use super::VaultHeader;

/// Atomically write vault file (tmp + rename + dir fsync) with advisory file locking.
///
/// # Errors
/// Returns `VaultError::Io` if file operations fail,
/// or `VaultError::Io` if locking/syncing fails.
pub fn atomic_write_vault(
    path: &std::path::Path,
    header: &VaultHeader,
    ciphertext: &[u8],
) -> Result<(), crate::VaultError> {
    use fs2::FileExt;
    use std::fs::File;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(crate::VaultError::Io)?;
        #[cfg(unix)]
        {
            let mut perms = std::fs::metadata(parent)
                .map_err(crate::VaultError::Io)?
                .permissions();
            perms.set_mode(0o700);
            std::fs::set_permissions(parent, perms).map_err(crate::VaultError::Io)?;
        }
    }

    // Open the vault file for writing and acquire exclusive lock
    let tmp_path = path.with_extension("bin.tmp");
    let mut file = open_tmp_reclaiming_stale(&tmp_path)?;

    // Acquire exclusive lock on the temp file
    file.lock_exclusive()
        .map_err(|e| crate::VaultError::Io(std::io::Error::other(e)))?;

    // Past this point every early return must take the temp file with it. The
    // open above refuses to overwrite an existing one, so a single leftover
    // makes every future save fail with `AlreadyExists` -- for good, and with
    // nothing pointing at the cause.
    let mut tmp_guard = TmpFileGuard {
        path: &tmp_path,
        armed: true,
    };

    let header_bytes = header.to_bytes();
    file.write_all(&header_bytes)
        .map_err(crate::VaultError::Io)?;
    file.write_all(ciphertext).map_err(crate::VaultError::Io)?;
    file.flush().map_err(crate::VaultError::Io)?;
    file.sync_all().map_err(crate::VaultError::Io)?;

    // Release lock before rename (lock is on temp file)
    file.unlock()
        .map_err(|e| crate::VaultError::Io(std::io::Error::other(e)))?;

    std::fs::rename(&tmp_path, path).map_err(crate::VaultError::Io)?;
    // The temp file is the vault now; there is nothing left to clean up.
    tmp_guard.armed = false;

    // Sync directory to ensure rename is persisted
    if let Some(parent) = path.parent() {
        let dir = File::open(parent).map_err(crate::VaultError::Io)?;
        dir.sync_all().map_err(crate::VaultError::Io)?;
    }

    Ok(())
}

/// Deletes the half-written temp file unless disarmed after a successful rename.
struct TmpFileGuard<'a> {
    path: &'a std::path::Path,
    armed: bool,
}

impl Drop for TmpFileGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(self.path);
        }
    }
}

/// Create the temp file, clearing away debris left by a writer that died.
///
/// `create_new` is deliberate -- it stops two writers clobbering one another --
/// but on its own it turns any leftover temp file into a permanent failure:
/// every later save returns `AlreadyExists`, so the vault quietly stops being
/// able to store anything. A process killed between creating the temp file and
/// the rename leaves exactly that, and so does a panic, since the release
/// profile aborts and runs no destructors.
///
/// A writer that is still working holds an exclusive lock on its temp file. So a
/// temp file whose lock can be taken belongs to nobody and is debris; one whose
/// lock is held belongs to a live writer and is left alone.
fn open_tmp_reclaiming_stale(
    tmp_path: &std::path::Path,
) -> Result<std::fs::File, crate::VaultError> {
    use fs2::FileExt;
    use std::fs::OpenOptions;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;

    let mut open_opts = OpenOptions::new();
    open_opts.write(true).create_new(true);
    #[cfg(unix)]
    open_opts.mode(0o600);

    match open_opts.open(tmp_path) {
        Ok(file) => Ok(file),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let stale = std::fs::File::open(tmp_path).map_err(crate::VaultError::Io)?;
            stale.try_lock_exclusive().map_err(|_| {
                crate::VaultError::Io(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "another process is writing the vault",
                ))
            })?;
            let _ = stale.unlock();
            drop(stale);
            std::fs::remove_file(tmp_path).map_err(crate::VaultError::Io)?;
            open_opts.open(tmp_path).map_err(crate::VaultError::Io)
        }
        Err(e) => Err(crate::VaultError::Io(e)),
    }
}
