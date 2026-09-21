//! The one upgrade lock.
//!
//! There used to be two, written as quoted shell strings in the client --
//! `wrap_with_upgrade_lock` for a remote host and `wrap_with_local_upgrade_lock`
//! for the local one -- and they had drifted. The local one broke a stale lock
//! when its recorded PID was no longer running; the remote one only ever broke
//! one on a six-hour timestamp. So a run killed on a remote host blocked every
//! later upgrade of that host for six hours, while the identical kill locally
//! recovered at once. Two copies of one rule is how one of them stops being the
//! rule.
//!
//! It also closes the hole the remote version documented and left open:
//!
//! > The stamp is written just after the directory, not with it, and the
//! > automatic break needs it: a crash in that window leaves a directory with no
//! > `ts`, which no later run can time and therefore none will break.
//!
//! A directory's own mtime is set by the kernel when the directory is created,
//! in the same operation. Timing the lock by that rather than by a file written
//! afterwards means there is no window to crash in. The `ts` file is gone; the
//! `pid` file stays, because liveness is a better answer than age whenever it
//! is available.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How old a lock may be before it is broken on age alone.
///
/// Age is the fallback. A lock whose owner is demonstrably gone is broken
/// immediately whatever its age, so this only governs the case where the PID
/// cannot be read or has been reused.
pub const STALE_AFTER: Duration = Duration::from_secs(6 * 60 * 60);

/// Held for as long as the command runs; released on drop.
///
/// Drop is the whole point. The shell versions released the lock with an `EXIT`
/// trap *and* an explicit `rm` on the success path, which is two places to get
/// it right, and neither of them runs if the shell is killed with `SIGKILL`.
/// The stale-break below is what covers that, and it is why the break has to
/// work rather than merely exist.
#[derive(Debug)]
pub struct Guard {
    dir: PathBuf,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Guard {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.dir
    }
}

/// Why an acquisition did not happen.
#[derive(Debug, PartialEq, Eq)]
pub enum Denied {
    /// Another run holds it and is still alive, or too recent to break.
    Held,
    /// The lock could not be created for a reason that is not contention --
    /// a read-only home, no space, a permission. Reported rather than
    /// swallowed: "another upgrade is running" about a full disk sends the
    /// operator to look for a process that does not exist.
    Failed(String),
}

/// Take the lock, breaking a demonstrably dead one.
///
/// # Errors
///
/// [`Denied::Held`] when a live run owns it, [`Denied::Failed`] for anything
/// else.
pub fn acquire(dir: &Path) -> Result<Guard, Denied> {
    if let Some(parent) = dir.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            return Err(Denied::Failed(format!("{}: {e}", parent.display())));
        }
    }
    // A plain file at the lock path is not a lock; it is debris from an older
    // build that used one. Remove it, then try for real.
    if dir.is_file() {
        let _ = fs::remove_file(dir);
    }
    match fs::create_dir(dir) {
        Ok(()) => Ok(stamp(dir)),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            if breakable(dir) {
                let _ = fs::remove_dir_all(dir);
                return match fs::create_dir(dir) {
                    Ok(()) => Ok(stamp(dir)),
                    // Lost the race to another client that broke it first.
                    // That client now holds it legitimately.
                    Err(_) => Err(Denied::Held),
                };
            }
            Err(Denied::Held)
        }
        Err(e) => Err(Denied::Failed(e.to_string())),
    }
}

/// Record who owns it. Best-effort: a lock with no readable `pid` is still a
/// lock, it just falls back to being timed rather than tested.
fn stamp(dir: &Path) -> Guard {
    let _ = fs::write(dir.join("pid"), std::process::id().to_string());
    Guard {
        dir: dir.to_path_buf(),
    }
}

/// Whether an existing lock may be taken from its owner.
fn breakable(dir: &Path) -> bool {
    if let Some(pid) = read_pid(dir) {
        // A PID that answers is the owner, whatever the clock says: a six-hour
        // compile is not a stale lock.
        return !alive(pid);
    }
    // No readable PID. Fall back to the directory's own creation time, which
    // the kernel set in the same operation that made it -- there is no window
    // in which a lock exists but cannot be timed.
    age(dir).is_none_or(|a| a > STALE_AFTER)
}

fn read_pid(dir: &Path) -> Option<i32> {
    fs::read_to_string(dir.join("pid"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// How long ago the lock directory was made.
///
/// `None` when the filesystem will not say, which is treated as breakable: a
/// lock nothing can time and nothing can test is a lock that would otherwise be
/// permanent, and permanent is the one outcome with no way out but editing the
/// host by hand.
fn age(dir: &Path) -> Option<Duration> {
    let made = fs::metadata(dir).ok()?.modified().ok()?;
    SystemTime::now().duration_since(made).ok()
}

/// Whether a process exists. `kill(pid, 0)` asks without sending anything.
///
/// `EPERM` counts as alive: the process is there, this user simply may not
/// signal it. Reading that as dead would break the lock of a run started by
/// another account on the same host.
fn alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // SAFETY: `kill` with signal 0 sends nothing and only reports whether the
    // process exists. It takes two integers and touches no memory this side
    // owns, so there is no pointer or lifetime for the call to get wrong.
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Where the lock lives, under the same cache directory the agent binary does.
#[must_use]
pub fn default_path() -> PathBuf {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join(".cache").join("multitop").join("upgrade.lock")
}
