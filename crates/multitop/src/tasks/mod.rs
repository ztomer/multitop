//! Long-running tasks: SSH command execution and output streaming.

mod painted;
mod spawn;
mod upgrade;
mod verdict;

pub use painted::*;
pub use spawn::{spawn_docker, spawn_fetch};
pub use upgrade::spawn_upgrade;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    // The sudo-handshake tests that lived here are gone with
    // `deliver_sudo_password`. They drove a real `sh` through a
    // `__multitop_pw_ready__` line and asserted the password reached its `read`
    // and was withheld from a remote that never asked. Both properties still
    // hold and are still tested -- on the side that now owns them, against a
    // real pty, in `crates/agent/tests/exec_run_test.rs`.
    //
    // The marker-scanning tests are gone for the same reason: a sentinel is a
    // `MarkerKind` frame now and is recognised on the host, by the process that
    // owns the pty. Nothing on this side reads one out of a line any more, so a
    // test that it can would be testing a path with no callers.

    fn shown(line: &str) -> Option<&str> {
        painted_states(line).rfind(|state| !state.trim().is_empty())
    }

    #[test]
    fn a_rewritten_line_logs_only_what_it_ended_on() {
        let progress = "Downloading 10%\rDownloading 47%\rDownloading 100%\rDone.";
        assert_eq!(
            painted_states(progress).count(),
            4,
            "all states are scanned"
        );
        assert_eq!(shown(progress), Some("Done."));
    }

    #[test]
    fn a_crlf_line_is_one_state() {
        assert_eq!(shown("plain output\r\n"), Some("plain output"));
        assert_eq!(shown("plain output\n"), Some("plain output"));
    }

    #[test]
    fn a_bar_with_a_trailing_rewrite_shows_the_last_drawn_state() {
        assert_eq!(shown("[##    ] 20%\r[#####] 90%\r"), Some("[#####] 90%"));
    }

    #[test]
    fn sudo_help_is_recognised_whichever_stream_it_came_on() {
        assert!(is_sudo_help(
            "sudo: no tty present and no askpass program specified"
        ));
        assert!(is_sudo_help(
            "sudo: a terminal is required to read the password"
        ));
        assert!(!is_sudo_help("installing sudo-1.9.0"));
    }

    #[test]
    fn a_command_that_needs_root_is_recognised_as_help() {
        assert!(is_sudo_help("e: could not open lock file /var/lib/dpkg/lock-frontend - open (13: permission denied)"));
        assert!(is_sudo_help(
            "e: unable to acquire the dpkg frontend lock. are you root?"
        ));
        assert!(is_sudo_help(
            "e: could not open lock file /var/lib/apt/lists/lock - open (13: permission denied)"
        ));
        assert!(!is_sudo_help("permission denied (publickey)"));
    }
}
