use crate::config::Server;
use crate::ssh::command::{
    bootstrap_script, cleanup_old_agents_command, detached, exec_bootstrap_arg, is_local,
    ssh_command, upload_command,
};
use crate::ssh_opts::{Arch, Mode, AGENT_AARCH64, AGENT_X86_64};
use multitop_agent::exec::ExecFrame;
use multitop_agent::SortBy;
use std::io;
use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};

/// Spawn a local agent process.
///
/// # Errors
///
/// Returns an error if spawning the local process fails.
pub fn spawn_local_agent(mode: Mode, sort: SortBy) -> io::Result<Child> {
    let mut cmd = local_agent_command();
    cmd.args([mode.word(), "127.0.0.1", "80", "24", sort.word()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    cmd.spawn()
}

/// The command that runs this build's agent locally, with its mode word not yet
/// appended.
///
/// One resolver rather than two: the streaming panel and the exec channel must
/// run the *same* binary, or a local upgrade could be served by a different
/// version from the one drawing the panel beside it.
fn local_agent_command() -> Command {
    let (cmd, extra_args) = std::env::current_exe().map_or_else(
        |_| (Command::new("multitop-agent"), vec![]),
        |exe| {
            let parent = exe.parent().unwrap_or_else(|| Path::new(""));
            let grand = parent.parent().unwrap_or_else(|| Path::new(""));
            let name = exe.file_name().and_then(|n| n.to_str()).unwrap_or("");

            if name == "multitop" || (name.starts_with("multitop") && !name.contains("test")) {
                (Command::new(exe), vec!["--agent".to_string()])
            } else if parent.join("multitop-agent").is_file() {
                (Command::new(parent.join("multitop-agent")), vec![])
            } else if grand.join("multitop-agent").is_file() {
                (Command::new(grand.join("multitop-agent")), vec![])
            } else if grand.join("multitop").is_file() {
                (
                    Command::new(grand.join("multitop")),
                    vec!["--agent".to_string()],
                )
            } else {
                (Command::new("multitop-agent"), vec![])
            }
        },
    );

    let mut cmd = detached(cmd);
    if !extra_args.is_empty() {
        cmd.args(extra_args);
    }
    cmd
}

/// Start the agent, or learn that it needs uploading first.
///
/// stdout carries agent frames; stderr is folded in so an SSH failure
/// (`Permission denied`, `Host key verification failed`) lands in the panel
/// instead of vanishing.
///
/// # Errors
///
/// Returns an error if spawning the agent command fails.
pub async fn spawn_agent(server: &Server, mode: Mode, sort: SortBy) -> io::Result<Child> {
    if is_local(server) {
        return spawn_local_agent(mode, sort);
    }
    let script = bootstrap_script(mode, &server.host, sort);
    let mut cmd = ssh_command(server);
    cmd.arg("sh")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn()?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(script.as_bytes()).await?;
    }
    Ok(child)
}

/// Start the agent in exec mode and hand it one request.
///
/// The request goes on stdin as a framed packet and stdin is then closed. The
/// agent reads exactly one and never looks at stdin again -- what the child
/// needs is written to its pty, not here.
///
/// Local and remote differ only in what is spawned. That is the point: the
/// agent owns the pty either way, so a local panel and a remote one produce the
/// same bytes. Before this, local ran `$SHELL -c` with two pipes and no pty
/// while remote ran `ssh -tt`, and the two disagreed about line endings, about
/// whether stderr was separate, and about whether the command saw a terminal.
///
/// # Errors
///
/// Returns an error if the process could not be spawned or the request could
/// not be written.
pub async fn spawn_exec(server: &Server, request: &ExecFrame) -> io::Result<Child> {
    let mut cmd = if is_local(server) {
        let mut c = local_agent_command();
        c.arg("exec");
        c
    } else {
        let mut c = ssh_command(server);
        c.arg(exec_bootstrap_arg());
        c
    };
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let packet = multitop_agent::proto::encode_packet(&multitop_agent::proto::Payload::Exec(
        request.clone(),
    ));
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&packet).await?;
        stdin.shutdown().await?;
        // Dropped here, closing the pipe. The agent has its request; leaving
        // this open would hold a descriptor for the length of an upgrade for
        // nothing.
    }
    Ok(child)
}

// `spawn_command`, `wrap_with_upgrade_lock`, `wrap_with_local_upgrade_lock` and
// the `Spawned` struct lived between here and the upload below.
//
// All four were the old transport. `spawn_command` ran `ssh -tt` remotely and
// `$SHELL -c` locally -- two different stream shapes for one feature -- and the
// two lock wrappers were the same rule written twice as quoted shell, which is
// how one of them came to be missing the PID check the other had. The lock is
// `multitop_agent::exec::lock` now: one implementation, in a language with a
// type checker, with tests.
//
// `Spawned` carried an `awaits_password` flag whose whole purpose was to stop
// the caller deciding a second time whether a password was coming, because the
// two answers could disagree. Nothing decides it twice any more: the password
// is a field in the request.

/// Ship the agent binary for `arch` to the server.
///
/// # Errors
///
/// Returns an error string if agent binary is missing or upload fails.
pub async fn upload_agent(server: &Server, arch: Arch, token: &str) -> Result<(), String> {
    let Some(bytes) = arch.binary() else {
        return Err(format!(
            "No {} agent was built into this binary. Rebuild with ./build.sh to include it.",
            arch.label()
        ));
    };

    let mut child = ssh_command(server)
        .arg(upload_command(arch.hash(), token, bytes.len()))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("ssh: {e}"))?;

    // A write failure is *not* returned here, and that is the whole point.
    //
    // The agent is several megabytes. If the remote side of the pipe has
    // already given up -- `mkdir` refused on a read-only home, no space left in
    // `~/.cache`, a quota -- the write fails with `Broken pipe`, and returning
    // that discards the child's stderr, which is where the actual reason is.
    // The operator was told "upload: Broken pipe" about a disk that was full.
    //
    // Same class as the eighth pass's stderr finding in `spawn_upgrade`: stderr
    // is where the reason lives, and the pipe closing is the *symptom* of it.
    // So the failure is remembered and the child is reaped either way; the
    // reason below wins whenever there is one.
    let wrote: Result<(), String> = match child.stdin.take() {
        Some(mut stdin) => stdin
            .write_all(bytes)
            .await
            .and(stdin.shutdown().await)
            .map_err(|e| format!("upload: {e}")),
        None => Ok(()),
    };

    let out = child
        .wait_with_output()
        .await
        .map_err(|e| format!("upload: {e}"))?;
    if out.status.success() {
        // A clean exit after a failed write is not a success: the remote
        // command may have ended before it had the whole binary.
        wrote?;
        // Clean up stale agent binaries left from previous builds.
        // We don't care if this fails — it's best-effort cleanup.
        let _ = ssh_command(server)
            .arg(cleanup_old_agents_command())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(upload_failure(
        &server.host,
        &stderr,
        wrote.as_ref().err().map(String::as_str),
    ))
}

/// Which explanation the operator gets when an upload fails.
///
/// The remote's own complaint wins whenever it made one. `Broken pipe` on this
/// side is what a remote refusal *looks like* locally -- the child had already
/// exited -- so reporting it in place of "No space left on device" names the
/// symptom and hides the cause. It stands in only when the remote said nothing
/// at all, where it is the only thing there is to say.
///
/// Separated from [`upload_agent`] because reaching that path needs a host that
/// refuses a multi-megabyte write partway through.
pub fn upload_failure(host: &str, stderr: &str, wrote: Option<&str>) -> String {
    let detail = stderr
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or_else(|| wrote.unwrap_or("unknown error"));
    format!("Could not install agent on {host}: {detail}")
}

/// True when at least one architecture was compiled in.
#[must_use]
pub fn any_agent_embedded() -> bool {
    AGENT_X86_64.is_some() || AGENT_AARCH64.is_some()
}

/// Probe the remote host's architecture by running `uname -m` over SSH.
pub async fn probe_remote_arch(server: &Server) -> Option<Arch> {
    let output = ssh_command(server)
        .arg("uname")
        .arg("-m")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?
        .wait_with_output()
        .await
        .ok()?;
    if output.status.success() {
        let arch_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Arch::from_uname(&arch_str)
    } else {
        None
    }
}
