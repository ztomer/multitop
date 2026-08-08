//! Long-running tasks: SSH command execution and output streaming.

mod painted;
mod spawn;
mod upgrade;

pub use painted::*;
pub use spawn::{deliver_sudo_password, MAX_SENTINEL_LINES, SENTINEL_TIMEOUT};
pub use spawn::{spawn_docker, spawn_fetch};
pub use upgrade::spawn_upgrade;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::ssh;
    use tokio::io::AsyncBufReadExt as _;
    use tokio::io::BufReader;
    use tokio::process::Command;

    /// Run `sh -c script` with piped stdio and put it through the handshake.
    #[allow(clippy::while_let_loop)]
    async fn run(script: &str) -> (bool, String) {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut lines = BufReader::new(stdout).lines();

        let ready = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            deliver_sudo_password(
                &mut lines,
                child.stdin.take(),
                "s3cret",
                std::time::Duration::from_millis(400),
            ),
        )
        .await
        .expect("the sentinel wait must be bounded in time, not only in lines");

        let mut rest = String::new();
        let drain = async {
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        rest.push_str(&line);
                        rest.push('\n');
                    }
                    _ => break,
                }
            }
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), drain).await;
        let status = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait())
            .await
            .expect("the child must exit; a still-open stdin pipe is what hangs it");
        assert!(status.is_ok());
        (ready, rest)
    }

    #[tokio::test]
    async fn the_password_is_written_once_the_remote_says_it_is_ready() {
        let script = format!(
            "printf '{}\\n'; IFS= read -r p; printf 'got:%s\\n' \"$p\"",
            ssh::PW_READY_SENTINEL
        );
        let (ready, rest) = run(&script).await;
        assert!(ready, "the sentinel was printed, so it must be seen");
        assert!(
            rest.contains("got:s3cret"),
            "the password must reach the remote's `read`, got {rest:?}"
        );
    }

    #[tokio::test]
    async fn lines_printed_before_the_sentinel_are_skipped() {
        let script = format!(
            "echo motd; echo 'Last login: today'; printf '{}\\n'; IFS= read -r p; printf 'got:%s\\n' \"$p\"",
            ssh::PW_READY_SENTINEL
        );
        let (ready, rest) = run(&script).await;
        assert!(ready);
        assert!(rest.contains("got:s3cret"), "got {rest:?}");
    }

    #[tokio::test]
    async fn a_missing_sentinel_still_closes_the_pipe() {
        let script = "IFS= read -r p; printf 'ended:%s\\n' \"$p\"";
        let (ready, rest) = run(script).await;
        assert!(!ready, "no sentinel was printed, so nothing may be sent");
        assert!(
            !rest.contains("s3cret"),
            "the password must not be sent to a remote that never asked: {rest:?}"
        );
        assert!(rest.contains("ended:"), "the child's `read` must have returned, which only happens when the write end is dropped -- got {rest:?}");
    }

    #[tokio::test]
    async fn a_silent_remote_is_not_sent_the_password() {
        let (ready, rest) = run("exit 0").await;
        assert!(!ready);
        assert!(rest.is_empty(), "got {rest:?}");
    }

    #[cfg(test)]
    mod painted_line_tests {
        #![allow(clippy::unwrap_used, clippy::panic)]

        use crate::ssh;
        use crate::tasks::*;

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
        fn an_overwritten_sentinel_is_still_scannable() {
            let line = format!("{}\rcarrying on", ssh::SUDO_FAILED_SENTINEL);
            assert!(
                painted_states(&line).any(|state| state.trim() == ssh::SUDO_FAILED_SENTINEL),
                "the scan sees every state"
            );
            assert_eq!(shown(&line), Some("carrying on"));
        }

        #[test]
        fn sudo_help_is_recognised_on_both_streams() {
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
            assert!(is_sudo_help("e: could not open lock file /var/lib/apt/lists/lock - open (13: permission denied)"));
            assert!(!is_sudo_help("permission denied (publickey)"));
        }

        #[test]
        fn the_lock_held_sentinel_is_scannable() {
            let line = format!("{}\r\n", ssh::LOCK_HELD_SENTINEL);
            assert!(painted_states(&line).any(|state| state.trim() == ssh::LOCK_HELD_SENTINEL));
        }
    }

    #[cfg(test)]
    mod marker_tests {
        #![allow(clippy::unwrap_used, clippy::panic)]

        use crate::ssh;
        use crate::tasks::*;

        #[test]
        fn both_markers_are_recognised_on_either_stream() {
            assert_eq!(
                marker(ssh::LOCK_HELD_SENTINEL),
                Some(Marker::LockHeld),
                "a pty merges stderr into stdout, so this must be seen anywhere"
            );
            assert_eq!(marker(ssh::SUDO_FAILED_SENTINEL), Some(Marker::SudoFailed));
            assert_eq!(marker("Reading package lists..."), None);
            assert_eq!(marker(""), None);
        }

        #[test]
        fn a_marker_survives_being_painted_over() {
            let line = format!("{}\rcarrying on", ssh::LOCK_HELD_SENTINEL);
            assert!(
                painted_states(&line).any(|state| marker(state.trim()) == Some(Marker::LockHeld)),
                "the scan sees every state"
            );
        }
    }
}
