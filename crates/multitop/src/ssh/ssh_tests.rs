#[cfg(test)]
mod upload_failure_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use crate::ssh::upload_failure;

    // `sudo_preamble_tests` opened this file and is gone with the code it
    // covered -- `wrap_with_upgrade_lock` and `password_preamble`, which were
    // the old transport written as quoted shell.
    //
    // Every property they asserted still holds and is still tested, on the side
    // that now owns it:
    //
    // * a held lock is distinguishable from a failing command --
    //   `a_held_lock_stops_the_second_run_and_says_so` (agent, against a real
    //   lock rather than against a string that mentions one);
    // * a refused password is distinguishable from a failing command --
    //   `a_marker_after_a_carriage_return_is_still_a_marker` (agent);
    // * echo is off before the password is written -- the agent writes it only
    //   on the `PwReady` marker, which the preamble prints after `stty -echo`;
    // * the password is never an argument -- `password_argv_live_e2e`,
    //   repointed at the exec channel, which reads the real `/proc` of the real
    //   process tree.
    //
    // The old versions asserted those properties by grepping a *generated shell
    // string* for a sentinel. That checks the script was written as intended and
    // nothing about whether it behaves that way; the replacements run it.

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
