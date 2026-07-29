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
            (Command::new(grand.join("multitop")), vec!["--agent".to_string()])
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
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(script.as_bytes()).await?;
        stdin.shutdown().await?;
    }
    Ok(child)
}

/// Run an arbitrary command on the server (used for `upgrade_cmd`).
pub fn spawn_command(server: &Server, command: &str, password: Option<&str>) -> io::Result<Child> {
    let quoted = sh_quote(command);
    if is_local(server) {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "zsh".to_string());
        // When a password is provided, first validate sudo credentials with
        // `sudo -S -v` (reads password from stdin, caches credentials for the
        // default timeout). Then run the actual command which reuses cached
        // credentials — this works even for multi-sudo commands like `us;ud`.
        let wrapped = match password {
            Some(pass) => format!(
                "echo {} | sudo -S -v 2>/dev/null; setopt expand_aliases 2>/dev/null; shopt -s expand_aliases 2>/dev/null; source ~/.zshrc 2>/dev/null; source ~/.zprofile 2>/dev/null; source ~/.bashrc 2>/dev/null; eval {quoted}",
                sh_quote(pass),
            ),
            None => format!(
                "setopt expand_aliases 2>/dev/null; shopt -s expand_aliases 2>/dev/null; source ~/.zshrc 2>/dev/null; source ~/.zprofile 2>/dev/null; source ~/.bashrc 2>/dev/null; eval {quoted}"
            ),
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
        Some(pass) => {
            let pass_q = sh_quote(pass);
            format!(
                "echo {pass_q} | sudo -S -v 2>/dev/null; if command -v zsh >/dev/null 2>&1; then zsh -c 'setopt expand_aliases 2>/dev/null; source ~/.zshrc 2>/dev/null; source ~/.zprofile 2>/dev/null; eval {quoted}'; elif command -v bash >/dev/null 2>&1; then bash -c 'shopt -s expand_aliases 2>/dev/null; source ~/.bashrc 2>/dev/null; eval {quoted}'; else sh -c {quoted}; fi"
            )
        }
        None => format!(
            "if command -v zsh >/dev/null 2>&1; then zsh -c 'setopt expand_aliases 2>/dev/null; source ~/.zshrc 2>/dev/null; source ~/.zprofile 2>/dev/null; eval {quoted}'; elif command -v bash >/dev/null 2>&1; then bash -c 'shopt -s expand_aliases 2>/dev/null; source ~/.bashrc 2>/dev/null; eval {quoted}'; else sh -c {quoted}; fi"
        ),
    };

    let child = ssh_command(server)
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
