//! SSH transport and agent deployment.
//!
//! The agent is a static Linux binary cached on each host under
//! `~/.cache/multitop/agent-<hash>`. The warm path is a single SSH round
//! trip: a short bootstrap script execs the cached binary. Only when the
//! cache misses does the local side upload, which costs two more round trips
//! and then never happens again for that build.

use std::io;
use std::path::Path;
use std::process::Stdio;

use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};

use crate::config::Server;
pub use crate::ssh_opts::*;

/// Remote cache path for a given build of the agent.
pub fn agent_path(hash: &str) -> String {
    format!("$HOME/.cache/multitop/agent-{hash}")
}

/// The bootstrap script, fed to a remote `sh` on stdin.
///
/// Delivering it on stdin rather than as an `ssh` command argument means the
/// user's login shell never parses it — no quoting to get wrong, and the
/// script is guaranteed to run under POSIX `sh` even where the login shell is
/// fish or csh.
///
/// The script resolves the architecture itself so a cache hit is one round
/// trip on every host, rather than guessing an architecture locally and
/// paying an extra round trip whenever the guess is wrong.
use multitop_agent::SortBy;

pub fn bootstrap_script(mode: Mode, display_ip: &str, sort: SortBy) -> String {
    format!(
        "LC_ALL=C; LANG=C; export LC_ALL LANG\n\
         M=$(uname -m)\n\
         case \"$M\" in\n\
         x86_64|amd64) A=\"{x86}\" ;;\n\
         aarch64|arm64) A=\"{arm}\" ;;\n\
         *) A=\"\" ;;\n\
         esac\n\
         if [ -n \"$A\" ] && [ -x \"$A\" ]; then\n\
         exec \"$A\" {mode} {ip} 80 24 {sort}\n\
         fi\n\
         echo \"{NEED_AGENT} $M\"\n",
        x86 = agent_path(HASH_X86_64),
        arm = agent_path(HASH_AARCH64),
        mode = mode.word(),
        ip = sh_quote(display_ip),
        sort = sort.word(),
    )
}

/// Command that receives the agent on stdin and installs it atomically.
///
/// `token` makes the staging name unique so two panels bootstrapping the same
/// host cannot have one `mv` the file another is still writing.
pub fn upload_command(hash: &str, token: &str) -> String {
    let dir = "~/.cache/multitop";
    let final_path = format!("{dir}/agent-{hash}");
    let staging = format!("{final_path}.{token}");
    format!(
        "mkdir -p {dir} && cat > {staging} && chmod 755 {staging} && mv -f {staging} {final_path}"
    )
}

/// Shell command that removes stale agent binaries from the cache directory.
///
/// Keeps only the current x86_64 and aarch64 agent hashes, removing everything
/// else that matches the `agent-*` pattern. Safe to run concurrently — each
/// `rm` targets a specific file, not the directory.
pub fn cleanup_old_agents_command() -> String {
    let keep = [HASH_X86_64, HASH_AARCH64];
    let keep_patterns: Vec<String> = keep
        .iter()
        .filter(|h| !h.is_empty() && **h != "missing")
        .map(|h| format!("agent-{h}"))
        .collect();
    let mut cmd = String::from("cd ~/.cache/multitop 2>/dev/null && for f in agent-*; do\n");
    cmd.push_str("  case \"$f\" in\n");
    for pattern in &keep_patterns {
        cmd.push_str(&format!("    {pattern}) continue ;;\n"));
    }
    cmd.push_str("    agent-*) rm -f \"$f\" ;;\n");
    cmd.push_str("  esac\ndone");
    cmd
}

/// Parse a `===NEEDAGENT=== <arch>` line into its architecture field.
pub fn parse_need_agent(line: &str) -> Option<&str> {
    line.trim().strip_prefix(NEED_AGENT).map(str::trim)
}

fn ssh_command(server: &Server) -> Command {
    let mut cmd = Command::new("ssh");
    cmd.env("LC_ALL", "C").env("LANG", "C");
    cmd.args(SSH_OPTS);
    cmd.arg("-p").arg(server.port.to_string());
    cmd.arg(server.target().as_ref());
    cmd
}

pub fn ssh_command_tty(server: &Server) -> Command {
    let mut cmd = Command::new("ssh");
    cmd.env("LC_ALL", "C").env("LANG", "C");
    for opt in SSH_OPTS {
        if *opt != "-T" {
            cmd.arg(opt);
        }
    }
    cmd.arg("-tt");
    cmd.arg("-p").arg(server.port.to_string());
    cmd.arg(server.target().as_ref());
    cmd
}

pub fn is_local(server: &Server) -> bool {
    server.host == "localhost" || server.host == "127.0.0.1" || server.port == 0
}

pub fn spawn_local_agent(mode: Mode, sort: SortBy) -> io::Result<Child> {
    let (mut cmd, extra_args) = if let Ok(exe) = std::env::current_exe() {
        let parent = exe.parent().unwrap_or(Path::new(""));
        let grand = parent.parent().unwrap_or(Path::new(""));
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
    } else {
        (Command::new("multitop-agent"), vec![])
    };

    if !extra_args.is_empty() {
        cmd.args(extra_args);
    }

    cmd.args([mode.word(), "127.0.0.1", "80", "24", sort.word()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    cmd.spawn()
}

/// Start the agent, or learn that it needs uploading first.
///
/// stdout carries agent frames; stderr is folded in so an SSH failure
/// (`Permission denied`, `Host key verification failed`) lands in the panel
/// instead of vanishing.
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

/// Run an arbitrary command on the server (used for `upgrade_cmd`).
///
/// When a password is provided, `sudo -v` validates and caches the credential
/// for the default timeout (~15 min). The command then runs in a separate
/// shell that reuses the cached credential, so any `sudo` calls within the
/// command (e.g. aliases like `ud` = `sudo apt update && sudo apt upgrade`)
/// do not re-prompt.
///
/// For alias resolution, we use login+interactive shells (`zsh -l -i` / `bash -i`)
/// which source the user's full profile, ensuring aliases defined in `.zshrc`
/// / `.bashrc` are available before `eval` expands the command.
///
/// The command is embedded in a single-quoted zsh/bash `-c` argument, so
/// single quotes within the command are escaped via the `'\''` idiom to
/// prevent premature termination of the outer quote.
/// Wraps a shell command with an atomic remote lock using `mkdir`.
///
/// Only one concurrent upgrade can hold the lock across all clients/sessions
/// connected to the same server. Stale locks older than 6 hours are
/// automatically broken so a server crash / power loss doesn't permanently
/// block future upgrades.
///
/// The `LOCK_OK` flag ensures the inner command only runs if the lock was
/// actually acquired — a race between stale-lock removal and re-acquisition
/// won't silently run the upgrade without a lock.
fn wrap_with_upgrade_lock(inner: &str) -> String {
    let lockdir = "~/.cache/multitop/upgrade.lock";
    format!(
        "mkdir -p ~/.cache/multitop 2>/dev/null; \
         LOCK={ldir}; \
         [ -e \"$LOCK\" ] && [ ! -d \"$LOCK\" ] && rm -f \"$LOCK\" 2>/dev/null; \
         if mkdir \"$LOCK\" 2>/dev/null; then \
           LOCK_OK=1; \
         elif [ -f \"$LOCK/ts\" ] && [ \"$(($(date +%s) - $(cat \"$LOCK/ts\" 2>/dev/null)))\" -gt 21600 ] 2>/dev/null; then \
           rm -rf \"$LOCK\" 2>/dev/null; \
           mkdir \"$LOCK\" 2>/dev/null && LOCK_OK=1; \
         fi; \
         if [ \"${{LOCK_OK:-0}}\" -eq 1 ]; then \
           date +%s > \"$LOCK/ts\" 2>/dev/null; \
           trap 'rm -rf \"$LOCK\"' EXIT; \
           {inner}; \
           rc=$?; \
           rm -rf \"$LOCK\"; \
           exit $rc; \
         else \
           echo \"Upgrade already in progress on this server\" >&2; \
           exit 1; \
         fi",
        ldir = lockdir,
        inner = inner
    )
}

/// Wraps a local shell command with a PID-based lock.
///
/// Uses `mkdir` atomicity with a PID liveness check: if the lock directory
/// exists but the recorded PID is no longer running, the lock is broken.
/// A timestamp file provides a 6-hour staleness fallback if the PID file
/// is missing (e.g. disk full during `echo $$`).
///
/// The `LOCK_OK` flag ensures the inner command only runs if the lock was
/// actually acquired — a race between stale-lock removal and re-acquisition
/// won't silently run the upgrade without a lock.
fn wrap_with_local_upgrade_lock(inner: &str) -> String {
    format!(
        "mkdir -p ~/.cache/multitop 2>/dev/null; \
         LOCK=~/.cache/multitop/upgrade.lock; \
         [ -e \"$LOCK\" ] && [ ! -d \"$LOCK\" ] && rm -f \"$LOCK\" 2>/dev/null; \
         if mkdir \"$LOCK\" 2>/dev/null; then \
           LOCK_OK=1; \
         elif [ -f \"$LOCK/pid\" ] && ! kill -0 $(cat \"$LOCK/pid\" 2>/dev/null) 2>/dev/null; then \
           rm -rf \"$LOCK\" 2>/dev/null; \
           mkdir \"$LOCK\" 2>/dev/null && LOCK_OK=1; \
         elif [ -f \"$LOCK/ts\" ] && [ \"$(($(date +%s) - $(cat \"$LOCK/ts\" 2>/dev/null)))\" -gt 21600 ] 2>/dev/null; then \
           rm -rf \"$LOCK\" 2>/dev/null; \
           mkdir \"$LOCK\" 2>/dev/null && LOCK_OK=1; \
         fi; \
         if [ \"${{LOCK_OK:-0}}\" -eq 1 ]; then \
           echo $$ > \"$LOCK/pid\" 2>/dev/null; \
           date +%s > \"$LOCK/ts\" 2>/dev/null; \
           trap 'rm -rf \"$LOCK\"' EXIT; \
           {inner}; \
           rc=$?; \
           rm -rf \"$LOCK\"; \
           exit $rc; \
         else \
           echo \"Upgrade already in progress on this machine\" >&2; \
           exit 1; \
         fi",
        inner = inner
    )
}

pub fn spawn_command(server: &Server, command: &str, password: Option<&str>) -> io::Result<Child> {
    let quoted = sh_quote(command);
    let quoted_escaped = quoted.replace('\'', r"'\''");
    if is_local(server) {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "zsh".to_string());
        let use_lock = !crate::password_store::is_mock_enabled();
        let wrap = |inner: String| {
            if use_lock {
                wrap_with_local_upgrade_lock(&inner)
            } else {
                inner
            }
        };
        let wrapped = match password {
            Some(_pass) if crate::password_store::is_mock_enabled() => wrap(format!(
                "setopt expand_aliases 2>/dev/null; shopt -s expand_aliases 2>/dev/null; source ~/.zshrc 2>/dev/null; source ~/.zprofile 2>/dev/null; source ~/.bashrc 2>/dev/null; eval {quoted}"
            )),
            Some(pass) => wrap(format!(
                "echo {} | sudo -S -p '' -v 2>/dev/null && setopt expand_aliases 2>/dev/null; shopt -s expand_aliases 2>/dev/null; source ~/.zshrc 2>/dev/null; source ~/.zprofile 2>/dev/null; source ~/.bashrc 2>/dev/null; eval {quoted}",
                sh_quote(pass),
            )),
            None => wrap(format!(
                "setopt expand_aliases 2>/dev/null; shopt -s expand_aliases 2>/dev/null; source ~/.zshrc 2>/dev/null; source ~/.zprofile 2>/dev/null; source ~/.bashrc 2>/dev/null; eval {quoted}"
            )),
        };
        let child = Command::new(&shell)
            .arg("-c")
            .arg(wrapped)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        return Ok(child);
    }

    let remote_cmd = match password {
        Some(_pass) if crate::password_store::is_mock_enabled() => wrap_with_upgrade_lock(&format!(
            "if command -v zsh >/dev/null 2>&1; then zsh -l -i -c 'setopt expand_aliases 2>/dev/null; source ~/.zshrc 2>/dev/null; source ~/.zprofile 2>/dev/null; eval {quoted_escaped}'; elif command -v bash >/dev/null 2>&1; then bash -i -c 'shopt -s expand_aliases 2>/dev/null; source ~/.bashrc 2>/dev/null; source ~/.bash_profile 2>/dev/null; eval {quoted_escaped}'; else sh -c {quoted}; fi"
        )),
        Some(pass) => {
            let pass_q = sh_quote(pass);
            wrap_with_upgrade_lock(&format!(
                "echo {pass_q} | sudo -S -p '' -v 2>/dev/null && if command -v zsh >/dev/null 2>&1; then zsh -l -i -c 'setopt expand_aliases 2>/dev/null; source ~/.zshrc 2>/dev/null; source ~/.zprofile 2>/dev/null; eval {quoted_escaped}'; elif command -v bash >/dev/null 2>&1; then bash -i -c 'shopt -s expand_aliases 2>/dev/null; source ~/.bashrc 2>/dev/null; source ~/.bash_profile 2>/dev/null; eval {quoted_escaped}'; else sh -c {quoted}; fi"
            ))
        }
        None => wrap_with_upgrade_lock(&format!(
            "if command -v zsh >/dev/null 2>&1; then zsh -l -i -c 'setopt expand_aliases 2>/dev/null; source ~/.zshrc 2>/dev/null; source ~/.zprofile 2>/dev/null; eval {quoted_escaped}'; elif command -v bash >/dev/null 2>&1; then bash -i -c 'shopt -s expand_aliases 2>/dev/null; source ~/.bashrc 2>/dev/null; source ~/.bash_profile 2>/dev/null; eval {quoted_escaped}'; else sh -c {quoted}; fi"
        )),
    };

    let child = ssh_command_tty(server)
        .arg(remote_cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    Ok(child)
}

/// Ship the agent binary for `arch` to the server.
pub async fn upload_agent(server: &Server, arch: Arch, token: &str) -> Result<(), String> {
    let Some(bytes) = arch.binary() else {
        return Err(format!(
            "No {} agent was built into this binary. Rebuild with ./build.sh to include it.",
            arch.label()
        ));
    };

    let mut child = ssh_command(server)
        .arg(upload_command(arch.hash(), token))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("ssh: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(bytes)
            .await
            .map_err(|e| format!("upload: {e}"))?;
        stdin.shutdown().await.map_err(|e| format!("upload: {e}"))?;
    }

    let out = child
        .wait_with_output()
        .await
        .map_err(|e| format!("upload: {e}"))?;
    if out.status.success() {
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
    let detail = stderr.lines().next_back().unwrap_or("unknown error");
    Err(format!(
        "Could not install agent on {}: {detail}",
        server.host
    ))
}

/// True when at least one architecture was compiled in.
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
