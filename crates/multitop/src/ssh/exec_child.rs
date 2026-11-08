//! The exec agent's process, stopped the way its teardown needs (M.KILLDROP).
//!
//! `kill_on_drop` stops a child with `SIGKILL`, and `SIGKILL` runs no cleanup:
//! the agent never got to end its pty's session, so a command that ignores the
//! hangup outlived the stop at PPID 1. Dropping an [`ExecChild`] sends the
//! agent `SIGTERM` -- its handler ends the run's whole session and reaps it --
//! and only `SIGKILL`s whatever is still standing once [`STOP_GRACE`] is over.

use std::ops::{Deref, DerefMut};
use std::time::Duration;

use tokio::process::Child;

/// How long the agent has to tear its run down before it is killed. The
/// agent's own grace for the command's session is 2 s; this covers it.
pub const STOP_GRACE: Duration = Duration::from_secs(5);

/// How often a runtime-less drop looks for the agent having ended.
const STOP_POLL: Duration = Duration::from_millis(20);

/// An exec agent the client owns; dropped, it is asked to stop, then made to.
pub struct ExecChild(Option<Child>);

impl ExecChild {
    /// Own `child`, which must have been spawned without `kill_on_drop`, or
    /// tokio would `SIGKILL` it first.
    #[must_use]
    pub const fn new(child: Child) -> Self {
        Self(Some(child))
    }
}

impl Deref for ExecChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        // Only `drop` takes it out.
        self.0
            .as_ref()
            .unwrap_or_else(|| unreachable!("taken only in drop"))
    }
}

impl DerefMut for ExecChild {
    fn deref_mut(&mut self) -> &mut Child {
        self.0
            .as_mut()
            .unwrap_or_else(|| unreachable!("taken only in drop"))
    }
}

impl Drop for ExecChild {
    fn drop(&mut self) {
        let Some(mut child) = self.0.take() else {
            return;
        };
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        let Some(pid) = child.id() else {
            return;
        };
        // `kill(1)`, not `libc::kill`: this crate denies `unsafe_code`. The pid
        // cannot be reused yet, since `child` is not reaped until below.
        let termed = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if termed {
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                rt.spawn(async move {
                    if tokio::time::timeout(STOP_GRACE, child.wait())
                        .await
                        .is_err()
                    {
                        let _ = child.kill().await;
                    }
                });
                return;
            }
            // No runtime to wait on: wait here, bounded.
            let deadline = std::time::Instant::now() + STOP_GRACE;
            while std::time::Instant::now() < deadline {
                if !matches!(child.try_wait(), Ok(None)) {
                    return;
                }
                std::thread::sleep(STOP_POLL);
            }
        }
        let _ = child.start_kill();
    }
}
