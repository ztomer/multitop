#[cfg(test)]
mod upload_failure_tests {

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

    use crate::ssh::command::upload_command;
    use std::io::Write as _;

    /// A spawned child that is killed and collected however this scope ends.
    ///
    /// The upload script is not a `true`: it starts an `sshd` and holds a
    /// port, so leaking one is not a slow test but a live server left on the
    /// machine. Nothing between here and the reap can be relied on to run -- a
    /// `write_all` into a pipe the far end already closed is the obvious panic
    /// -- so the reap lives in `Drop`, which unwinding runs and a call below
    /// the assertion does not.
    struct Reap(std::process::Child);

    impl Reap {
        fn spawn(command: &mut std::process::Command) -> Self {
            Self(command.spawn().unwrap())
        }

        /// The child's stdin pipe. Taken rather than borrowed: the writer's
        /// `ChildStdin` closes when it is dropped, and the script waits for
        /// that EOF before it can exit.
        fn take_stdin(&mut self) -> std::process::ChildStdin {
            self.0.stdin.take().unwrap()
        }

        fn pid(&self) -> u32 {
            self.0.id()
        }

        fn wait_success(&mut self) -> bool {
            self.0.wait().unwrap().success()
        }
    }

    impl Drop for Reap {
        fn drop(&mut self) {
            // `kill` stops it; `wait` collects it, and only the first ends a
            // leak. Both errors are the state this is aiming at -- a child
            // that has already exited is one there is nothing left to stop, and
            // std refuses to signal a pid it has already reaped rather than
            // risk signalling whatever inherited it.
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// Run the real upload script under `sh` against a scratch HOME, feeding it
    /// `payload` on stdin, and report (exit ok, whether the agent landed).
    fn run(payload: &[u8], expected: usize, home: &std::path::Path) -> (bool, bool) {
        let script = upload_command("deadbeef", "tok", expected).replace("~/", "$HOME/");
        let mut child = Reap::spawn(
            std::process::Command::new("sh")
                .arg("-c")
                .arg(&script)
                .env("HOME", home)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()),
        );
        child.take_stdin().write_all(payload).unwrap();
        let ok = child.wait_success();
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

    /// The guard's contract, tested rather than asserted: a panic between the
    /// spawn and the wait leaves no child.
    ///
    /// The leak this exists for is an `sshd` holding a port, and the child here
    /// is a stand-in -- `sleep` outlives whatever started it and leaves nothing
    /// behind, so the verdict can be read off a pid instead of off a port. The
    /// panic stands in for the one that motivated the guard (a `write_all` into
    /// a pipe the far end has already closed) and lands in exactly the place it
    /// does, between the spawn and the reap.
    ///
    /// `catch_unwind` on THIS thread rather than in a spawned one, and that is
    /// load-bearing: a child of a thread that has exited is re-parented and
    /// reaped by the OS, so a guard that killed but forgot to `wait` would look
    /// clean from a thread. `kill -0` succeeds for a live process AND for a
    /// zombie, so only a non-zero exit says both halves ran.
    ///
    /// `exec` so the spawned pid IS the sleeper: `sh -c 'sleep'` may or may not
    /// fork, and a forked grandchild is nobody's to reap, so a variant without
    /// it would leave an orphan of its own.
    #[test]
    fn a_panic_after_the_spawn_leaves_no_child() {
        // The guard is created INSIDE the closure. Created outside it, it would
        // still be in scope when the probe runs, so its `Drop` would fire at the
        // end of the test and the assertion would pass for a guard that had done
        // nothing -- which is the shape that has to be distinguished, not a
        // detail of the test.
        let (tx, rx) = std::sync::mpsc::channel();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let child = Reap::spawn(
                std::process::Command::new("sh")
                    .arg("-c")
                    .arg("exec sleep 30")
                    .stdin(std::process::Stdio::piped()),
            );
            tx.send(child.pid()).expect("report the pid");
            panic!("the failing upload's write_all lands here");
        }));
        assert!(unwound.is_err(), "the panic must have been caught");
        let pid = rx.recv().expect("the spawn reported its pid");
        let alive = std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .expect("probe the pid");
        assert!(
            !alive.success(),
            "pid {pid} survived the panic: it was not killed, or killed and never waited for"
        );
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
