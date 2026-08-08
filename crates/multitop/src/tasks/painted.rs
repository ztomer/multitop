#![allow(clippy::must_use_candidate)]
use crate::ssh;

pub fn painted_states(line: &str) -> impl DoubleEndedIterator<Item = &str> {
    line.trim_end_matches('\n')
        .split('\r')
        .map(|state| state.trim_end_matches('\r'))
}

/// A marker the remote prints for this program rather than for the operator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Marker {
    /// `sudo` refused the password we handed it.
    SudoFailed,
    /// Another run already holds the upgrade lock.
    LockHeld,
}

/// Recognise a marker on **either** stream, and take it out of the log.
///
/// It used to be one sentinel per stream -- `SUDO_FAILED` checked only on
/// stdout, `LOCK_HELD` only on stderr -- which describes the *local* shape,
/// where the two pipes stay separate.
///
/// A remote upgrade runs under `ssh -tt`, and **a pty has one stream**: sshd
/// merges the remote's stderr into it, so everything the remote writes arrives
/// on the local client's stdout. The held-lock sentinel therefore reached the
/// branch that was not looking for it. Two things followed. The detection was
/// dead for every remote host -- only the distinct exit code still caught it,
/// so the sentinel's whole purpose, surviving a lost exit status in a noisy
/// login shell, was gone. And because the stdout branch had no reason to skip
/// it, `__multitop_lock_held__` was printed into the operator's upgrade log
/// verbatim: an internal marker shown as output.
///
/// One scanner, both streams -- the same rule `is_sudo_help` is written under,
/// for the same reason: two streams disagreeing about what counts is how one of
/// them stops recognising it.
pub fn marker(trimmed: &str) -> Option<Marker> {
    if trimmed == ssh::SUDO_FAILED_SENTINEL {
        return Some(Marker::SudoFailed);
    }
    if trimmed == ssh::LOCK_HELD_SENTINEL {
        return Some(Marker::LockHeld);
    }
    None
}

/// Whether a line is sudo explaining that it could not ask for a password.
///
/// One copy, because the two streams disagreeing about what counts is how one
/// of them stops recognising it.
///
/// Also matches the shape a non-root `upgrade_cmd` produces: apt's "are you
/// root?" and dpkg's "permission denied" on its own lock files contain no
/// "sudo", so without these arms a command that merely forgot the `sudo`
/// prefix was reported as a failing command with no hint why.
pub fn is_sudo_help(lower: &str) -> bool {
    lower.contains("sudo")
        && (lower.contains("terminal")
            || lower.contains("password")
            || lower.contains("pre-authorized")
            || lower.contains("tty")
            || lower.contains("prompt on"))
        || lower.contains("are you root")
        || (lower.contains("permission denied")
            && (lower.contains("/var/lib/dpkg")
                || lower.contains("/var/lib/apt")
                || lower.contains("lock-frontend")))
}
