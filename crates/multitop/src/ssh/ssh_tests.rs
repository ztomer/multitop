#[cfg(test)]
mod sudo_preamble_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use crate::ssh::password_preamble;
    use crate::ssh::wrap_with_upgrade_lock;
    use crate::ssh::{LOCK_HELD_CODE, LOCK_HELD_SENTINEL, SUDO_FAILED_CODE, SUDO_FAILED_SENTINEL};

    /// A held lock must be distinguishable from a failing command. The wrapper
    /// used to print "Upgrade already in progress" and exit 1, which the panel
    /// reported as "upgrade command exited 1 -- command failed" for a command
    /// that never ran, for the six hours the stale lock lasts.
    #[test]
    fn a_held_lock_reports_itself() {
        let w = wrap_with_upgrade_lock("true");
        assert!(
            w.contains(LOCK_HELD_SENTINEL),
            "the remote must name the held lock on stderr: {w}"
        );
        assert!(
            w.contains(&format!("exit {LOCK_HELD_CODE}")),
            "and carry a distinct exit status, in case the line is lost: {w}"
        );
    }

    /// A rejected password must be distinguishable from a failing command.
    ///
    /// The preamble used to end in `[ $rc -eq 0 ]` and be joined to the upgrade
    /// with `&&`, so both cases exited 1 and the panel reported "upgrade
    /// command exited 1 -- host reachable, command failed" for a command that
    /// never ran. That sent the user to read their upgrade script when the
    /// problem was the password.
    #[test]
    fn a_refused_password_reports_itself() {
        let p = password_preamble();
        assert!(
            p.contains(SUDO_FAILED_SENTINEL),
            "the remote must name the failure on stdout: {p}"
        );
        assert!(
            p.contains(&format!("exit {SUDO_FAILED_CODE}")),
            "and carry a distinct exit status, in case the line is lost in \
             login-shell noise: {p}"
        );
        assert!(
            !p.trim_end().ends_with("[ $__mt_rc -eq 0 ]"),
            "the old shape: indistinguishable from any other failure"
        );
    }

    /// Echo has to be off before the password is read, or the pty prints it.
    #[test]
    fn echo_is_disabled_before_the_password_is_read() {
        let p = password_preamble();
        let stty = p.find("stty -echo").expect("echo must be turned off");
        let read = p.find("read -r __mt_pw").expect("the password is read");
        assert!(stty < read, "echo must be off first: {p}");
    }

    /// The password must never appear in an argument.
    #[test]
    fn the_password_is_never_an_argument() {
        let p = password_preamble();
        assert!(p.contains("sudo -S"), "sudo must read it from stdin: {p}");
        assert!(
            !p.contains("echo '") && !p.contains("--password"),
            "argv is world-readable through /proc on Linux: {p}"
        );
    }
}

#[cfg(test)]
mod upload_failure_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use crate::ssh::upload_failure;

    /// The remote's complaint wins over the local symptom.
    ///
    /// A refused write closes the pipe, so this side sees `Broken pipe` while
    /// the remote is saying "No space left on device". Returning the local
    /// error -- which is what `write_all(...)?` did -- named the symptom and
    /// threw away the cause, on the one screen the operator has to work from.
    /// Same class as the eighth pass's stderr finding in `spawn_upgrade`.
    #[test]
    fn the_remote_reason_beats_the_local_broken_pipe() {
        let msg = upload_failure(
            "web-01",
            "cat: write error: No space left on device\n",
            Some("upload: Broken pipe (os error 32)"),
        );
        assert!(
            msg.contains("No space left on device"),
            "the cause must survive: {msg}"
        );
        assert!(
            !msg.contains("Broken pipe"),
            "and the symptom must not stand in for it: {msg}"
        );
        assert!(msg.contains("web-01"), "the host is named: {msg}");
    }

    /// When the remote said nothing, the local error is the only thing there is
    /// to say -- and saying nothing at all is the defect this whole round keeps
    /// finding.
    #[test]
    fn a_silent_remote_leaves_the_local_error_standing() {
        let msg = upload_failure("db-02", "   \n\n", Some("upload: Broken pipe"));
        assert!(msg.contains("Broken pipe"), "{msg}");
    }

    /// Neither side said anything: still a sentence, never an empty one.
    #[test]
    fn a_failure_with_no_detail_at_all_still_names_the_host() {
        let msg = upload_failure("cache-03", "", None);
        assert!(msg.contains("cache-03"), "{msg}");
        assert!(msg.contains("unknown error"), "{msg}");
    }
}

#[cfg(test)]
mod upload_command_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use crate::ssh::command::upload_command;
    use std::io::Write as _;

    /// Run the real upload script under `sh` against a scratch HOME, feeding it
    /// `payload` on stdin, and report (exit ok, whether the agent landed).
    fn run(payload: &[u8], expected: usize, home: &std::path::Path) -> (bool, bool) {
        let script = upload_command("deadbeef", "tok", expected).replace("~/", "$HOME/");
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg(&script)
            .env("HOME", home)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload).unwrap();
        let ok = child.wait().unwrap().success();
        let landed = home.join(".cache/multitop/agent-deadbeef").exists();
        (ok, landed)
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("multitop_upload_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A complete upload installs the agent.
    #[test]
    fn a_complete_upload_lands() {
        let home = scratch("complete");
        let payload = b"ELF-ish agent bytes";
        let (ok, landed) = run(payload, payload.len(), &home);
        assert!(ok, "a complete upload must succeed");
        assert!(landed, "and the agent must be in place");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The regression, and it is the serious half.
    ///
    /// `cat` cannot tell a finished stream from an interrupted one -- both end
    /// in EOF -- so a connection that dropped partway through left `cat`,
    /// `chmod` and `mv` all succeeding and the whole command **exiting 0 with a
    /// truncated binary installed as the agent**. The local side reported a
    /// successful install; the next connection then failed to exec it, and the
    /// panel blamed the architecture or the bootstrap for a file this program
    /// had put there itself.
    #[test]
    fn a_truncated_upload_is_refused_rather_than_installed() {
        let home = scratch("truncated");
        // The stream stopped early: fewer bytes arrive than were promised.
        let (ok, landed) = run(b"ELF-ish", 19, &home);
        assert!(!ok, "a short upload must not report success");
        assert!(
            !landed,
            "and above all must not be installed as the agent -- \
             the next connection would exec it"
        );
        let staging = home.join(".cache/multitop/agent-deadbeef.tok");
        assert!(
            !staging.exists(),
            "the staging file must be cleaned up, not left to accumulate"
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}
