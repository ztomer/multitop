//! What the operator is told when a run ends.
//!
//! Split from the reader because the two answer different questions and are
//! read at different times. The reader is about bytes arriving in order; this is
//! about which of several true things is the one worth saying -- and nearly
//! every line of it exists because an earlier version said the wrong one. A
//! refused password reported as "the command failed" sent people to read an
//! upgrade script that was fine, for the six hours a stale lock lasts.

use multitop_agent::exec::{LOCK_HELD_CODE, SUDO_FAILED_CODE};

/// The offset a shell adds to a signal number when it reports a child that was
/// killed. `$?` is 137 for `SIGKILL`, which is 128 + 9.
const SIGNAL_EXIT_BASE: i32 = 128;
/// The highest signal number worth reading that convention into.
///
/// Bounded rather than "anything above 128" on purpose: a command is free to
/// exit 200 meaning something of its own, and calling that signal 72 would be
/// inventing a cause. 31 is the last of the standard signals.
const MAX_SIGNAL: i32 = 31;

use crate::config::Server;
use crate::fmt::status_line;

/// What one attempt learned.
///
/// Five flags rather than an enum on purpose: they are not alternatives. A run
/// can be refused by `sudo` *and* have printed the "no tty present" help, and
/// which of those the operator is told about changes what they go and fix.
#[allow(clippy::struct_excessive_bools)]
#[derive(Default)]
pub struct Report {
    pub exit: Option<(i32, bool)>,
    pub sudo_help: bool,
    pub sudo_rejected: bool,
    pub lock_held: bool,
    /// The architecture a host named when it had no agent to run.
    pub need_agent: Option<String>,
    /// Whatever came out that was not framing -- a login banner, an rc-file
    /// error. Kept so a failure can name its cause instead of reporting a
    /// connection that was never opened.
    pub preamble: Option<String>,
    pub stalled: bool,
    /// The last few stderr lines, which is where the reason usually is.
    ///
    /// Bounded and shown at the end rather than interleaved, because `apt`
    /// writes its progress display to stderr too and a hundred rewrites of one
    /// bar would otherwise push the actual error message out of the buffer.
    pub errbuf: Vec<String>,
}

/// How the run ended, in the operator's terms.
pub struct Outcome {
    pub note: String,
    pub success: bool,
}

#[must_use]
/// Say what actually happened.
///
/// Reporting every failure as "disconnected" blamed the network for a command
/// that merely exited non-zero, on a host the stats view was talking to
/// perfectly well at the time.
pub fn verdict(server: &Server, report: &Report) -> Outcome {
    if report.stalled {
        return Outcome {
            note: status_line(format!(
                "\u{26A0} lost contact with {} mid-upgrade \u{2014} it may still be running there",
                server.host
            )),
            success: false,
        };
    }
    let code_says = report.exit.map(|(code, _)| code);
    // A refused password and a held lock are not failing commands: in both
    // cases the command never ran, and saying "exited 1" sent operators to read
    // an upgrade script that was fine.
    //
    // Marker *or* exit code. The marker is the better signal -- it is a frame
    // and cannot be lost in a noisy shell -- but the code is what survives a
    // marker the agent never got to send, so both are honoured.
    if report.sudo_rejected || code_says == Some(SUDO_FAILED_CODE) {
        return Outcome {
            note: status_line(format!(
                "\u{26A0} sudo refused the stored password on {} \u{2014} the upgrade did not run. \
                 Set this host's password with {} in Settings.",
                server.host,
                crate::consts::SETTINGS_KEY
            )),
            success: false,
        };
    }
    if report.lock_held || code_says == Some(LOCK_HELD_CODE) {
        return Outcome {
            note: status_line(format!(
                "\u{26A0} another upgrade holds the lock on {} \u{2014} this one never ran. If no \
                 other run is active, remove ~/.cache/multitop/upgrade.lock.",
                server.host
            )),
            success: false,
        };
    }
    if let Some(arch) = &report.need_agent {
        return Outcome {
            note: status_line(format!(
                "\u{26A0} no agent could be installed on {} for {arch}",
                server.host
            )),
            success: false,
        };
    }
    let Some((code, signalled)) = report.exit else {
        let detail = report
            .preamble
            .clone()
            .unwrap_or_else(|| format!("the session to {} closed", server.host));
        return Outcome {
            note: status_line(format!(
                "\u{26A0} the upgrade never reported finishing \u{2014} {detail}"
            )),
            success: false,
        };
    };
    if code == 0 {
        return Outcome {
            note: status_line("\u{2500} done"),
            success: true,
        };
    }
    let what = if signalled {
        format!("\u{26A0} upgrade command was killed by a signal (exit {code})")
    } else if let Some(sig) = signal_behind(code) {
        // The agent's own child exited normally, so `signalled` is false -- but
        // a shell that reports 128+N is telling us its child was killed, and
        // "exited 137" alone sends an operator looking for a bug in a command
        // the OOM killer stopped.
        format!(
            "\u{26A0} upgrade command was killed by signal {sig} (exit {code}) \u{2014} host \
             reachable"
        )
    } else {
        format!("\u{26A0} upgrade command exited {code} \u{2014} host reachable, command failed")
    };
    Outcome {
        note: status_line(what),
        success: false,
    }
}

/// What to tell an operator whose `sudo` could not ask for a password.
pub fn sudo_tips(pass: Option<&str>) -> Vec<String> {
    let first = if pass.is_none() {
        "\x1b[33m\u{2192} Tip: Set password in settings ('e') to allow upgrades\x1b[0m"
    } else {
        "\x1b[33m\u{2192} Tip: Check password in settings ('e') or sudoer permissions\x1b[0m"
    };
    vec![
        first.to_string(),
        "\x1b[33m\u{2192} Tip: Add '<user> ALL=(ALL) NOPASSWD: ALL' to /etc/sudoers for passwordless sudo\x1b[0m"
            .to_string(),
    ]
}

/// The signal a shell is reporting through a 128+N status, if that is what this
/// is.
///
/// Bounded to real signal numbers rather than "anything over 128": a command is
/// free to exit 200 meaning something of its own, and calling that signal 72
/// would be inventing a cause.
const fn signal_behind(code: i32) -> Option<i32> {
    if code > SIGNAL_EXIT_BASE && code <= SIGNAL_EXIT_BASE + MAX_SIGNAL {
        Some(code - SIGNAL_EXIT_BASE)
    } else {
        None
    }
}
