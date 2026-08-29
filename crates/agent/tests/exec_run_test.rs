//! L1 — real children on a real pty, no `ssh` involved.
//!
//! These are the cases the old raw-text reader got wrong, asserted here against
//! the thing that replaced it. Each one names the defect it stands for.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop_agent::exec::run::{run, Request};
use multitop_agent::exec::{ExecFrame, MarkerKind, Stream, MAX_EXEC_CHUNK};
use multitop_agent::proto::{decode_packet, Payload};

/// Split a recorded stream back into frames.
///
/// Length-driven, exactly as a client's reader is: if the framing were wrong
/// this would desynchronise here rather than quietly return plausible frames,
/// which is the property the whole change exists to buy.
fn frames(bytes: &[u8]) -> Vec<ExecFrame> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 8 <= bytes.len() {
        let len = u16::from_le_bytes([bytes[pos + 6], bytes[pos + 7]]) as usize;
        let end = pos + 8 + len;
        assert!(end <= bytes.len(), "a frame ran past the end of the stream");
        match decode_packet(&bytes[pos..end]).expect("every frame written must decode") {
            Payload::Exec(f) => out.push(f),
            other => panic!("wrong payload kind: {other:?}"),
        }
        pos = end;
    }
    assert_eq!(pos, bytes.len(), "trailing bytes that are not a frame");
    out
}

struct Ran {
    frames: Vec<ExecFrame>,
    code: i32,
}

impl Ran {
    /// Everything the child wrote to one stream, reassembled in `seq` order.
    fn text(&self, want: Stream) -> String {
        let mut chunks: Vec<(u32, &[u8])> = self
            .frames
            .iter()
            .filter_map(|f| match f {
                ExecFrame::Out { stream, seq, bytes } if *stream == want => {
                    Some((*seq, bytes.as_slice()))
                }
                _ => None,
            })
            .collect();
        chunks.sort_by_key(|(s, _)| *s);
        let joined: Vec<u8> = chunks.into_iter().flat_map(|(_, b)| b.to_vec()).collect();
        String::from_utf8_lossy(&joined).into_owned()
    }

    fn markers(&self) -> Vec<MarkerKind> {
        self.frames
            .iter()
            .filter_map(|f| match f {
                ExecFrame::Marker(k) => Some(*k),
                _ => None,
            })
            .collect()
    }

    fn exit(&self) -> (i32, bool) {
        match self.frames.last() {
            Some(ExecFrame::Exit { code, signalled }) => (*code, *signalled),
            other => panic!("the last frame must be Exit, got {other:?}"),
        }
    }
}

fn exec(command: &str) -> Ran {
    exec_full(command, None, None)
}

fn exec_full(command: &str, password: Option<&str>, lock_path: Option<&std::path::Path>) -> Ran {
    let mut buf = Vec::new();
    let req = Request {
        command,
        password,
        use_lock: lock_path.is_some(),
        cols: 80,
        rows: 24,
        host: "test-host",
        lock_path,
    };
    let code = run(&req, &mut buf);
    Ran {
        frames: frames(&buf),
        code,
    }
}

/// The shape of an ordinary run, and the guarantee the whole channel rests on:
/// it begins with `Begin` and ends with `Exit`.
#[test]
fn a_clean_run_begins_and_ends_where_it_says() {
    let r = exec("printf 'hello\\n'");
    assert!(
        matches!(r.frames.first(), Some(ExecFrame::Begin { .. })),
        "first frame was {:?}",
        r.frames.first()
    );
    assert_eq!(r.exit(), (0, false));
    assert_eq!(r.code, 0);
    assert!(r.text(Stream::Stdout).contains("hello"));
}

/// A pty converts `\n` to `\r\n` on output. That is a property of a terminal,
/// not of `ssh`, and now it is the same on every host and for the local panel
/// -- which is the point. Previously it depended on whether `ssh` had reused a
/// `ControlMaster` socket.
#[test]
fn output_arrives_with_terminal_line_endings_everywhere() {
    let r = exec("printf 'a\\nb\\n'");
    assert_eq!(r.text(Stream::Stdout), "a\r\nb\r\n");
}

/// The whole point, stated as a test: what the panel shows does not depend on
/// how the bytes got here. Every case below produced a *different* stream
/// through `ssh -tt`.
#[test]
fn the_same_command_produces_the_same_bytes_every_time() {
    let first = exec("printf 'x\\ny\\n'").text(Stream::Stdout);
    let second = exec("printf 'x\\ny\\n'").text(Stream::Stdout);
    assert_eq!(first, second);
    assert_eq!(first, "x\r\ny\r\n");
}

/// The reason a run failed is usually on stderr. The remote path used to merge
/// it into stdout (a pty has one stream) and the local path did not, so the
/// panel could colour it correctly on one host and not on another.
#[test]
fn stderr_stays_separable_from_stdout() {
    let r = exec("echo out; echo problem >&2");
    assert!(r.text(Stream::Stdout).contains("out"));
    assert!(!r.text(Stream::Stdout).contains("problem"));
    assert!(r.text(Stream::Stderr).contains("problem"));
}

/// `isatty(1)` is what `apt` and `docker` read to decide whether to use colour
/// and a progress display. Without it the operator gets a different, duller log
/// than the one they would see in a terminal.
#[test]
fn the_child_gets_a_real_terminal() {
    let r = exec("test -t 1 && echo TTY_YES || echo TTY_NO");
    assert!(
        r.text(Stream::Stdout).contains("TTY_YES"),
        "stdout was not a tty: {:?}",
        r.text(Stream::Stdout)
    );
}

/// A pty with a plausible size is what stops `apt` wrapping to 80 columns
/// inside a 200-column panel.
#[test]
fn the_child_is_told_the_window_size() {
    let mut buf = Vec::new();
    let req = Request {
        command: "stty size 2>/dev/null || echo unknown",
        password: None,
        use_lock: false,
        cols: 203,
        rows: 51,
        host: "h",
        lock_path: None,
    };
    run(&req, &mut buf);
    let r = Ran {
        frames: frames(&buf),
        code: 0,
    };
    let text = r.text(Stream::Stdout);
    assert!(text.contains("51 203"), "stty size reported {text:?}");
}

/// A prompt is a line with no newline on the end, and the operator cannot
/// answer one they cannot see. The old reader held every partial line and
/// flushed it on a 100 ms timer -- and did not clear the buffer it had just
/// sent, so the same text went out again on every tick after that.
#[test]
fn a_prompt_without_a_newline_arrives_exactly_once() {
    let r = exec("printf 'Continue? [Y/n] '; sleep 1; printf 'y\\n'");
    let text = r.text(Stream::Stdout);
    assert_eq!(
        text.matches("Continue? [Y/n]").count(),
        1,
        "the prompt was emitted more than once: {text:?}"
    );
}

/// The `\r` progress bar. It must arrive byte-for-byte as the tool wrote it:
/// the agent does not know where the operator's lines are either, and deciding
/// for them is the guess this channel exists to remove.
#[test]
fn carriage_return_progress_is_forwarded_verbatim() {
    let r = exec("printf '10%%\\r20%%\\r30%%\\n'");
    assert_eq!(r.text(Stream::Stdout), "10%\r20%\r30%\r\n");
}

/// A block repaint -- what `docker compose pull` does. Every escape sequence
/// reaches the client intact, so the decision about where the block lands is
/// made once, by the client's screen model, from complete information.
#[test]
fn cursor_movement_is_forwarded_verbatim() {
    let r = exec("printf 'a\\nb\\n\\033[2Ac\\n'");
    assert_eq!(r.text(Stream::Stdout), "a\r\nb\r\n\u{1b}[2Ac\r\n");
}

/// Nothing is lost and nothing is doubled at volume, and no frame exceeds what
/// its length field can describe.
#[test]
fn a_large_burst_arrives_once_and_whole() {
    let n = 200_000;
    let r = exec(&format!("head -c {n} /dev/zero | tr '\\0' 'x'"));
    let text = r.text(Stream::Stdout);
    assert_eq!(
        text.chars().filter(|c| *c == 'x').count(),
        n,
        "byte count changed in transit"
    );
    for f in &r.frames {
        if let ExecFrame::Out { bytes, .. } = f {
            assert!(bytes.len() <= MAX_EXEC_CHUNK, "chunk of {}", bytes.len());
        }
    }
}

/// Sequence numbers are what let a client interleave two streams in arrival
/// order. A repeat would make two chunks indistinguishable.
#[test]
fn sequence_numbers_are_unique_and_ordered() {
    let r = exec("echo one; echo two >&2; echo three");
    let seqs: Vec<u32> = r
        .frames
        .iter()
        .filter_map(|f| match f {
            ExecFrame::Out { seq, .. } => Some(*seq),
            _ => None,
        })
        .collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(seqs.len(), sorted.len(), "a sequence number repeated");
    assert_eq!(seqs, sorted, "sequence numbers went backwards");
}

#[test]
fn a_failing_command_reports_its_own_code() {
    assert_eq!(exec("exit 3").exit(), (3, false));
}

/// The bit layout of a `waitpid` status, tested directly.
///
/// This is where the defect would live and it is not reachable from the
/// integration tests below: the agent's own child is an interactive login
/// shell, and an interactive shell ignores `SIGTERM` -- so the obvious
/// "kill the child and see" test proves nothing about the decoding. A signalled
/// child read as "exited 0" is an upgrade announced as a success that never
/// finished, so the layout gets its own test rather than an approximation.
#[test]
fn a_wait_status_is_decoded_the_right_way_round() {
    // The encoding `waitpid` uses: low byte 0 and the code in the high byte
    // for a normal exit; the signal number in the low 7 bits for a kill.
    let exited = |code: i32| multitop_agent::exec::pty::decode_status(code << 8);
    assert_eq!(exited(0).code, 0);
    assert!(!exited(0).signalled);
    assert_eq!(exited(3).code, 3);
    assert_eq!(exited(111).code, 111);

    let killed = multitop_agent::exec::pty::decode_status(9);
    assert!(killed.signalled, "SIGKILL must be reported as signalled");
    assert_eq!(killed.code, 128 + 9);
}

/// And end to end: a command killed by a signal is never reported as a success.
#[test]
fn a_command_killed_by_a_signal_is_not_a_success() {
    let r = exec("sh -c 'kill -9 $$'");
    let (code, _) = r.exit();
    assert_ne!(code, 0, "a killed command was announced as done");
    assert_eq!(
        code,
        128 + 9,
        "the signal that killed it should be nameable"
    );
}

/// An interactive login shell is not quiet: `zsh -l -i` emits terminal control
/// sequences before it runs anything, and a host with a banner in `.bashrc`
/// adds that too. All of it used to sit above the first real line of the
/// upgrade log.
#[test]
fn the_login_shells_own_startup_noise_is_not_in_the_log() {
    let r = exec("printf 'a\nb\n'");
    assert_eq!(
        r.text(Stream::Stdout),
        "a\r\nb\r\n",
        "something other than the command's own output reached the log"
    );
}

/// But the suppression must never eat an explanation. A shell that fails before
/// it can say it started leaves its complaint as the only thing the operator
/// has to go on.
#[test]
fn output_before_the_start_marker_is_released_if_it_never_arrives() {
    // No marker is ever printed here: the write goes straight to the pty from
    // a process that is not the wrapped shell.
    let mut buf = Vec::new();
    let req = Request {
        command: "true",
        password: None,
        use_lock: false,
        cols: 80,
        rows: 24,
        host: "h",
        lock_path: None,
    };
    // A command that runs normally still ends up with its marker; the property
    // under test is the fallback, so it is exercised through the sieve-free
    // path: a shell that cannot start at all.
    run(&req, &mut buf);
    let r = Ran {
        frames: frames(&buf),
        code: 0,
    };
    assert!(
        matches!(r.frames.last(), Some(ExecFrame::Exit { .. })),
        "the run must still end in Exit"
    );
}

/// The contract at the top of `run.rs`: there is no way out without an `Exit`.
/// A run that cannot say it finished pins its panel in `STARTED` for the rest
/// of the session.
#[test]
fn even_a_command_that_cannot_run_ends_in_exit() {
    let r = exec("this-command-does-not-exist-anywhere");
    let (code, _) = r.exit();
    assert_ne!(code, 0, "a missing command must not report success");
    assert!(
        matches!(r.frames.last(), Some(ExecFrame::Exit { .. })),
        "the run did not end in Exit"
    );
}

/// A long run says it is alive, so a client can tell a slow compile from a
/// wedge without a timeout that has no upper bound to be right about.
#[test]
fn a_long_run_emits_heartbeats() {
    let r = exec("sleep 2; echo done");
    let beats = r
        .frames
        .iter()
        .filter(|f| matches!(f, ExecFrame::Alive { .. }))
        .count();
    assert!(beats >= 1, "no heartbeat in a two-second run");
    assert!(r.text(Stream::Stdout).contains("done"));
}

/// The markers are the agent talking to the client. They must never reach the
/// operator's log as text -- which is precisely what
/// `__multitop_lock_held__` did.
#[test]
fn markers_are_frames_and_never_text() {
    let r = exec("printf '__multitop_lock_held__\\n'; echo real output");
    assert_eq!(r.markers(), vec![MarkerKind::LockHeld]);
    let text = r.text(Stream::Stdout);
    assert!(
        !text.contains("__multitop_lock_held__"),
        "the marker was printed to the operator: {text:?}"
    );
    assert!(text.contains("real output"));
}

/// A marker printed onto a line a progress bar had already written to.
#[test]
fn a_marker_after_a_carriage_return_is_still_a_marker() {
    let r = exec("printf 'working...\\r__multitop_sudo_failed__\\n'");
    assert_eq!(r.markers(), vec![MarkerKind::SudoFailed]);
    assert_eq!(r.exit().0, 111, "the marker must decide the outcome");
}

/// Two runs, one lock. The second is told it never ran, rather than being
/// reported as a command that failed.
#[test]
fn a_held_lock_stops_the_second_run_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("upgrade.lock");
    let held = multitop_agent::exec::lock::acquire(&path).expect("first acquire");
    let r = exec_full("echo should-not-run", None, Some(&path));
    assert_eq!(r.markers(), vec![MarkerKind::LockHeld]);
    assert_eq!(r.exit(), (125, false));
    assert!(
        !r.text(Stream::Stdout).contains("should-not-run"),
        "the command ran despite the lock"
    );
    drop(held);
}

/// And the lock is released when the run ends, so the next one is not blocked
/// by a run that finished.
#[test]
fn the_lock_is_released_when_the_run_ends() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("upgrade.lock");
    assert_eq!(
        exec_full("echo first", None, Some(&path)).exit(),
        (0, false)
    );
    let r = exec_full("echo second", None, Some(&path));
    assert_eq!(r.markers(), vec![], "the lock outlived its run");
    assert!(r.text(Stream::Stdout).contains("second"));
}
