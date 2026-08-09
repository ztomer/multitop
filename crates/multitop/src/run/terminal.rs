//! Signal handling and terminal mode management.
//!
//! Owns the `SIGTTIN`/`SIGTTOU`/`SIGCONT`/`SIGTERM`/`SIGHUP` handlers, the
//! raw-mode/alternate-screen enter/leave, the panic hook, and the resume path.

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;

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

/// Every terminal mode multitop turns on beyond the two `ratatui::init` knows
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

/// Take the modes off on a panic too.
///
/// The release profile aborts, so `Drop` does not run and the hook is the only
/// cover. `ratatui::init` installs one; it restores what `ratatui` turned on,
/// which is not what this program turned on.
pub(super) fn hook_terminal_modes_into_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        leave_terminal_modes(&mut std::io::stdout());
        previous(info);
    }));
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

/// Puts the terminal back on every exit path, including early returns.
pub struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        leave_terminal_modes(&mut std::io::stdout());
        ratatui::restore();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
        assert!(!on.is_empty(), "no modes were enabled");
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
