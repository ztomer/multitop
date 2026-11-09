//! Signal handling and terminal mode management.
//!
//! Owns the `SIGTTIN`/`SIGTTOU`/`SIGCONT`/`SIGTERM`/`SIGHUP` handlers, the
//! raw-mode/alternate-screen enter/leave, the panic hook, and the resume path.

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use std::io::Write as _;

/// Signals that decide whether this process may keep the terminal.
///
/// `SIGTTIN` and `SIGTTOU` stop a process that touches the controlling terminal
/// from a background process group. Left at their default disposition they stop
/// *this* process, and a stopped process runs no destructors: `TerminalGuard`
/// never fires, so the terminal is abandoned in raw mode inside the alternate
/// screen and the shell's prompt is drawn on top of the last frame. That is what
/// `[1] + suspended (tty input)` looks like from the user's seat -- a crash with
/// a wrecked terminal.
///
/// Merely *registering* a handler is the fix: a caught signal does not stop the
/// process, so the offending read or write fails and the loop carries on.
/// `SIGCONT` repairs the aftermath of a stop this cannot prevent -- `SIGSTOP`,
/// or anything that landed before these were installed -- by rebuilding the
/// terminal state on resume.
///
/// `SIGTERM` and `SIGHUP` are here for the same reason from the other
/// direction: at their default disposition they end the process outright, so
/// `TerminalGuard` never runs and neither does the notice naming the upgrades
/// that were killed. Caught, they become an ordinary quit -- the loop leaves
/// through the path that restores the terminal and says what it interrupted.
/// `SIGHUP` in particular arrives exactly when the terminal is going away,
/// which is when a running `apt upgrade` most needs saying out loud.
#[cfg(unix)]
pub(super) struct TerminalSignals {
    ttin: tokio::signal::unix::Signal,
    ttou: tokio::signal::unix::Signal,
    cont: tokio::signal::unix::Signal,
    term: tokio::signal::unix::Signal,
    hup: tokio::signal::unix::Signal,
}

/// What the loop should do about a signal it just caught.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SignalAction {
    /// Caught, therefore not fatal and not a stop. Nothing to do.
    Ignore,
    /// The process has just been resumed; the terminal needs rebuilding.
    Resumed,
    /// Someone outside asked this process to end.
    Quit,
}

#[cfg(unix)]
impl TerminalSignals {
    /// Numbers come from `libc`, never from this file. `SIGTTIN` and `SIGTTOU`
    /// happen to agree across platforms, but `SIGCONT` does not -- it is 18 on
    /// Linux and 19 on the BSDs, macOS included -- and a hardcoded 18 registers
    /// a handler for `SIGTSTP` on a Mac while silently reporting success. The
    /// test below caught exactly that.
    pub(super) fn install() -> std::io::Result<Self> {
        use tokio::signal::unix::{signal, SignalKind};
        Ok(Self {
            ttin: signal(SignalKind::from_raw(libc::SIGTTIN))?,
            ttou: signal(SignalKind::from_raw(libc::SIGTTOU))?,
            cont: signal(SignalKind::from_raw(libc::SIGCONT))?,
            term: signal(SignalKind::from_raw(libc::SIGTERM))?,
            hup: signal(SignalKind::from_raw(libc::SIGHUP))?,
        })
    }
}

/// Wait for the next signal that bears on who owns the terminal.
///
/// One future over all of them, because `tokio::select!` evaluates every
/// branch's future up front and several branches cannot each hold
/// `&mut signals`.
#[cfg(unix)]
pub(super) async fn next_terminal_signal(signals: &mut Option<TerminalSignals>) -> SignalAction {
    match signals.as_mut() {
        Some(s) => {
            tokio::select! {
                // Caught, therefore not fatal and not a stop. Nothing to do but
                // carry on -- the read or write that provoked it has already
                // failed and the loop handles that.
                _ = s.ttin.recv() => SignalAction::Ignore,
                _ = s.ttou.recv() => SignalAction::Ignore,
                _ = s.cont.recv() => SignalAction::Resumed,
                _ = s.term.recv() => SignalAction::Quit,
                _ = s.hup.recv() => SignalAction::Quit,
            }
        }
        None => std::future::pending().await,
    }
}

#[cfg(not(unix))]
pub(super) async fn next_terminal_signal(_signals: &mut Option<()>) -> SignalAction {
    std::future::pending().await
}

/// Every terminal mode multitop turns on beyond the two `ratatui` knows
/// about, in one place.
///
/// There were three copies of this list -- startup, the resume path, and the
/// guard -- and a fourth reader that had never been told: `ratatui`'s panic
/// hook restores raw mode and the alternate screen and nothing else. Mouse
/// reporting therefore survived every panic, and the shell that inherited the
/// terminal printed an escape sequence each time the pointer crossed the
/// window. Add a mode here and [`leave_terminal_modes`] must take it away
/// again; the test below is what makes that true rather than remembered.
pub(super) fn enter_terminal_modes(w: &mut impl std::io::Write) -> std::io::Result<()> {
    execute!(w, EnableMouseCapture)
}

/// The exact inverse of [`enter_terminal_modes`]. Never propagates: it runs on
/// the way out, including from a panic hook, and refusing to finish the rest of
/// the restore because one mode would not come off is the bug the guard exists
/// to remove.
pub(super) fn leave_terminal_modes(w: &mut impl std::io::Write) {
    let _ = execute!(w, DisableMouseCapture);
}

/// Take the terminal over: raw mode, the alternate screen, and a panic hook
/// that gives both back -- what `ratatui::try_init` does, except the hook.
///
/// The release profile aborts, so `Drop` does not run on a panic and the hook
/// is the only cover. `ratatui`'s own hook restores only what `ratatui` turned
/// on (not mouse capture), and reports a failed restore with `eprintln!`, which
/// panics inside the panic hook when the terminal is gone. This one takes off
/// every mode [`enter_terminal_modes`] set and reports through
/// [`restore_terminal`], which cannot panic.
pub(super) fn take_terminal() -> std::io::Result<ratatui::DefaultTerminal> {
    use crossterm::terminal::{enable_raw_mode, EnterAlternateScreen};
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        leave_terminal_modes(&mut std::io::stdout());
        restore_terminal(ratatui::try_restore, &mut std::io::stderr());
        previous(info);
    }));
    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen)?;
    ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))
}

/// Rebuild the terminal after a resume. Anything that stopped this process left
/// raw mode and the alternate screen behind, and the shell has since drawn over
/// both.
#[cfg(unix)]
pub(super) fn reclaim_terminal() {
    use crossterm::terminal::{enable_raw_mode, EnterAlternateScreen};
    let _ = enable_raw_mode();
    let _ = execute!(std::io::stdout(), EnterAlternateScreen);
    let _ = enter_terminal_modes(&mut std::io::stdout());
}

#[cfg(not(unix))]
pub(super) fn reclaim_terminal() {}

/// Put the terminal back, and say on `report` if that failed.
///
/// Never panics. `ratatui::restore` says it with `eprintln!`, which PANICS when
/// the write fails -- and the write fails exactly when the restore did: the
/// terminal window was closed, `SIGHUP` became a quit, the restore hit a dead
/// tty, and so did the report. A panic inside `TerminalGuard::drop` is an abort
/// under the release profile (and a panic inside a panic everywhere, since the
/// panic hook restores again): every `multitop` SIGABRT crash report of
/// 2026-11-08 was this stack (M.CIPARITY).
pub(super) fn restore_terminal(
    restore: impl FnOnce() -> std::io::Result<()>,
    report: &mut impl std::io::Write,
) {
    if let Err(err) = restore() {
        // Ignored: the terminal is gone and there is nobody left to tell.
        let _ = writeln!(report, "Failed to restore terminal: {err}");
    }
}

/// Write a line to stderr on the way out, and never panic doing it.
///
/// Everything said after the TUI ends is said exactly when the terminal may
/// have gone away -- a closed window is a `SIGHUP`, which is a quit -- and
/// `eprintln!` panics on a failed write, an abort under the release profile.
/// What could not be said is dropped: there is nobody left to read it.
pub fn say_on_stderr(line: std::fmt::Arguments<'_>) {
    let _ = writeln!(std::io::stderr(), "{line}");
}

/// Puts the terminal back on every exit path, including early returns.
pub struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        leave_terminal_modes(&mut std::io::stdout());
        restore_terminal(ratatui::try_restore, &mut std::io::stderr());
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    /// Every mode turned on has to come off again. `ratatui`'s own panic hook
    /// restores raw mode and the alternate screen and nothing else, so a mode
    /// added here and not removed survives into the shell that inherits the
    /// terminal — which is how mouse reporting outlived every panic and made
    /// the prompt print an escape sequence on every pointer move.
    #[test]
    fn every_mode_that_goes_on_comes_back_off() {
        let mut on = Vec::new();
        enter_terminal_modes(&mut on).unwrap();
        let mut off = Vec::new();
        leave_terminal_modes(&mut off);

        let (on, off) = (
            String::from_utf8(on).unwrap(),
            String::from_utf8(off).unwrap(),
        );
        multitop_testassert::assert_not_empty!(on, "no modes were enabled");
        assert_eq!(
            on.matches("\x1b[?").count(),
            off.matches("\x1b[?").count(),
            "enter set {on:?} but leave cleared {off:?}"
        );
        // The inverse of a `h` (set) is the same parameter with `l` (reset).
        for seq in on.split('\x1b').filter(|s| s.ends_with('h')) {
            let reset = format!("{}l", &seq[..seq.len() - 1]);
            assert!(
                off.contains(&reset),
                "{seq:?} is never reset; leave was {off:?}"
            );
        }
    }

    /// Leaving never propagates: it runs on the way out, including from a
    /// panic hook, and refusing to finish the rest of the restore because one
    /// mode would not come off is the bug the guard exists to remove.
    #[test]
    fn leaving_does_not_stop_at_a_writer_that_fails() {
        struct Broken;
        impl std::io::Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("terminal already gone"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::other("terminal already gone"))
            }
        }
        leave_terminal_modes(&mut Broken);
    }

    /// Set in the child [`a_restore_with_stderr_gone_does_not_panic`] spawns.
    const DEAD_STDERR_CHILD: &str = "MULTITOP_TEST_DEAD_STDERR_CHILD";

    /// The child half: a restore that failed, reported to the process's real
    /// stderr, which the parent has closed. Does nothing in a normal run.
    #[test]
    fn restore_reports_into_a_dead_stderr() {
        if std::env::var_os(DEAD_STDERR_CHILD).is_none() {
            return;
        }
        restore_terminal(
            || Err(std::io::Error::other("the terminal went away")),
            &mut std::io::stderr(),
        );
    }

    /// The crash every `multitop` SIGABRT report of 2026-11-08 shows: the
    /// terminal goes away, the restore fails, and reporting that failure on the
    /// same dead terminal panicked inside `TerminalGuard::drop`. Reproduced for
    /// real: this test binary is re-run with its stderr a pipe nobody reads, so
    /// every write to it fails (`EPIPE`; Rust ignores `SIGPIPE`), as a write to a
    /// closed tty does (`EIO`).
    #[test]
    fn a_restore_with_stderr_gone_does_not_panic() {
        let name = format!(
            "{}::restore_reports_into_a_dead_stderr",
            module_path!().split_once("::").map_or("", |(_, rest)| rest)
        );
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([name.as_str(), "--exact", "--nocapture", "--test-threads=1"])
            .env(DEAD_STDERR_CHILD, "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("re-run this test binary");
        // Close the only reader before the child can write.
        drop(child.stderr.take());
        let out = child.wait_with_output().expect("reap the child");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("running 1 test\n"),
            "the child did not run exactly the restore test: {stdout}"
        );
        assert!(
            out.status.success(),
            "reporting a failed restore to a dead stderr panicked: {stdout}"
        );
    }

    /// A failed restore is still said when there is somewhere to say it.
    #[test]
    fn a_failed_restore_is_reported() {
        let mut said = Vec::new();
        restore_terminal(|| Err(std::io::Error::other("no tty")), &mut said);
        let said = String::from_utf8(said).unwrap();
        assert!(
            said.contains("Failed to restore terminal: no tty"),
            "{said:?}"
        );
        let mut quiet = Vec::new();
        restore_terminal(|| Ok(()), &mut quiet);
        assert!(quiet.is_empty(), "a restore that worked said {quiet:?}");
    }

    /// Without handlers installed there is no signal to wait for, so the arm
    /// must never resolve — resolving would spin the select loop.
    #[tokio::test]
    async fn with_no_handlers_the_signal_arm_never_fires() {
        let mut none = None;
        let waited = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            next_terminal_signal(&mut none),
        )
        .await;
        assert!(
            waited.is_err(),
            "the signal arm resolved with nothing installed"
        );
    }

    /// Numbers come from `libc`, never from this file: `SIGCONT` is 18 on
    /// Linux and 19 on the BSDs, macOS included, and a hardcoded 18 registers
    /// a handler for `SIGTSTP` on a Mac while reporting success.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_resume_is_recognised_as_a_resume() {
        let mut signals = Some(TerminalSignals::install().expect("install handlers"));

        // Through `kill(1)` rather than `raise(3)`: this workspace denies
        // `unsafe`, and a signal sent from outside is closer to the real thing
        // anyway. `SIGCONT` to a process that is already running does nothing
        // but deliver — the handler is what is under test.
        for (arg, want) in [
            ("-CONT", SignalAction::Resumed),
            ("-HUP", SignalAction::Quit),
        ] {
            let sent = std::process::Command::new("kill")
                .arg(arg)
                .arg(std::process::id().to_string())
                .status()
                .expect("send the signal");
            assert!(sent.success(), "kill {arg} failed");

            let action = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                next_terminal_signal(&mut signals),
            )
            .await
            .unwrap_or_else(|_| panic!("{arg} was never caught"));
            assert_eq!(action, want, "{arg}");
        }
    }
}
