//! Files only their owner may open: the `--serve` token and the TLS identity.
//!
//! Both are credentials, and both had to answer the same three questions -- what
//! mode is it created with, what happens when two starts race to create it, and
//! is an existing one that others can read accepted. One answer, here, so the
//! second credential cannot quietly get a weaker one than the first.

use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;

/// What [`read_or_create`] found or made.
pub enum Outcome {
    /// The file existed, is owner-only, and holds these bytes.
    Existing(Vec<u8>),
    /// The file was absent and this call created it, `0600`, holding these bytes.
    Created(Vec<u8>),
}

/// The permission bits of everyone but the owner.
const GROUP_AND_OTHER: u32 = 0o077;

/// Refuse a mode that lets anyone but the owner read or write: `ssh`'s rule for
/// a private key, and the error names the fix.
///
/// # Errors
///
/// The sentence to show, naming `path` and `chmod 600`.
pub fn refuse_if_open(path: &Path, mode: u32) -> Result<(), String> {
    if mode & GROUP_AND_OTHER == 0 {
        return Ok(());
    }
    let shown = path.display();
    Err(format!(
        "{shown} is open to other users (mode {:03o}); run: chmod 600 {shown}",
        mode & 0o777
    ))
}

/// The bytes at `path`, or -- when it is absent -- `make()`'s, written there
/// `0600`, exclusively, in a directory created `0700` if it does not exist.
///
/// `create_new` means a start never truncates what another start wrote between the
/// probe and the open; losing that race reads the winner's file instead.
///
/// # Errors
///
/// A sentence naming `path`: an existing file others can open, any I/O failure,
/// or `make`'s own error.
pub fn read_or_create(
    path: &Path,
    make: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<Outcome, String> {
    let shown = path.display();
    match std::fs::metadata(path) {
        Ok(meta) => return read_existing(path, &meta),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("cannot read {shown}: {e}")),
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let bytes = make()?;
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let meta = std::fs::metadata(path).map_err(|e| format!("cannot read {shown}: {e}"))?;
            return read_existing(path, &meta);
        }
        Err(e) => return Err(format!("cannot create {shown}: {e}")),
    };
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("cannot write {shown}: {e}"))?;
    Ok(Outcome::Created(bytes))
}

fn read_existing(path: &Path, meta: &std::fs::Metadata) -> Result<Outcome, String> {
    refuse_if_open(path, meta.permissions().mode())?;
    std::fs::read(path)
        .map(Outcome::Existing)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))
}
