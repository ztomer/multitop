//! Starting a host's `mcp_host`.
//!
//! Over ssh, riding the monitor stream's multiplexed connection, or `sh -c`
//! for a local panel. Nothing listens on the host; ssh starts the server for
//! this session and it ends with it.
//!
//! Process creation only. Everything a session says is `client.rs`, tested
//! over an in-memory pipe; this file is what needs a real host.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::task::JoinHandle;

use super::client::{Client, Error};
use crate::config::Server;
use crate::ssh;

/// A session with a running `mcp_host`.
pub type Session = Client<ChildStdout, ChildStdin>;

/// How long a closed session's stderr gets to reach its end.
/// Reason: its last line is WHY the session closed ("Connection refused",
/// "command not found"), written just before the process exits.
const EXIT_GRACE: Duration = Duration::from_secs(1);

/// The session, and the process behind it (killed when this is dropped).
pub struct Running {
    pub session: Session,
    _child: Child,
    stderr: JoinHandle<String>,
}

impl std::ops::Deref for Running {
    type Target = Session;
    fn deref(&self) -> &Session {
        &self.session
    }
}

impl std::ops::DerefMut for Running {
    fn deref_mut(&mut self) -> &mut Session {
        &mut self.session
    }
}

impl Running {
    /// Why the session ended: the process's last stderr line, once it has
    /// had [`EXIT_GRACE`] to finish writing; `fallback` when it said nothing.
    pub async fn last_words(self, fallback: String) -> String {
        last_words(self.stderr, fallback).await
    }
}

async fn last_words(stderr: JoinHandle<String>, fallback: String) -> String {
    let said = tokio::time::timeout(EXIT_GRACE, stderr)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    said.lines()
        .rfind(|l| !l.trim().is_empty())
        .map_or(fallback, |l| l.trim().to_string())
}

/// Start `command` on `server` and discover it.
///
/// # Errors
///
/// The process could not start, or the session failed to open; a closed
/// session carries ssh's or the server's last stderr line as its reason.
pub async fn open(server: &Server, command: &str, timeout: Duration) -> Result<Running, Error> {
    let mut cmd = if ssh::is_local(server) {
        let mut c = ssh::detached(Command::new("sh"));
        c.arg("-c").arg(command);
        c
    } else {
        let mut c = ssh::ssh_command(server);
        c.arg(command);
        c
    };
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Closed(format!("cannot start the session: {e}")))?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(Error::Closed("no pipes to the session".to_string()));
    };
    // Drained on its own task: a stderr pipe nobody reads fills, and a
    // server blocked writing to it looks exactly like a hang.
    let mut err = child.stderr.take();
    let stderr = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(e) = err.as_mut() {
            let _ = e.read_to_end(&mut buf).await;
        }
        String::from_utf8_lossy(&buf).into_owned()
    });
    match Client::connect(stdout, stdin, timeout).await {
        Ok(session) => Ok(Running {
            session,
            _child: child,
            stderr,
        }),
        Err(Error::Closed(why)) => Err(Error::Closed(last_words(stderr, why).await)),
        Err(e) => Err(e),
    }
}
