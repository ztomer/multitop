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
    cmd.arg(server.target());
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
pub fn spawn_command(server: &Server, command: &str) -> io::Result<Child> {
    if is_local(server) {
        return Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn();
    }
    ssh_command(server)
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> Server {
        Server {
            host: "10.0.0.1".into(),
            port: 22,
            user: String::new(),
            upgrade_cmd: None,
        }
    }

    #[test]
    fn arch_from_uname() {
        assert_eq!(Arch::from_uname("x86_64"), Some(Arch::X86_64));
        assert_eq!(Arch::from_uname("amd64"), Some(Arch::X86_64));
        assert_eq!(Arch::from_uname("aarch64"), Some(Arch::Aarch64));
        assert_eq!(Arch::from_uname("arm64"), Some(Arch::Aarch64));
        assert_eq!(Arch::from_uname("  x86_64\n"), Some(Arch::X86_64));
    }

    #[test]
    fn unsupported_arch_is_none() {
        assert_eq!(Arch::from_uname("armv7l"), None);
        assert_eq!(Arch::from_uname("riscv64"), None);
        assert_eq!(Arch::from_uname(""), None);
    }

    #[test]
    fn sh_quote_wraps_plainly() {
        assert_eq!(sh_quote("10.0.0.1"), "'10.0.0.1'");
        assert_eq!(sh_quote(""), "''");
    }

    #[test]
    fn sh_quote_neutralises_quotes() {
        assert_eq!(sh_quote("a'b"), r"'a'\''b'");
    }

    /// The property that matters is not what the quoted text looks like but
    /// what a real shell makes of it: exactly one argument, byte-identical to
    /// the input, with nothing executed. Asserting on the literal spelling
    /// would pass for quoting that is subtly wrong.
    #[test]
    fn sh_quote_round_trips_through_a_real_shell() {
        for raw in [
            "10.0.0.1",
            "",
            "a'b",
            "x'; rm -rf /; echo '",
            "$(whoami)",
            "`id`",
            "a b\tc",
            "back\\slash",
            "new\nline",
            "* ? [glob]",
            "\u{4f60}\u{597d}",
        ] {
            let out = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("printf %s {}", sh_quote(raw)))
                .output()
                .expect("run sh");
            assert!(out.status.success(), "sh rejected {raw:?}");
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                raw,
                "quoting changed the value for {raw:?}"
            );
        }
    }

    #[test]
    fn bootstrap_execs_cached_agent() {
        let s = bootstrap_script(Mode::Monitor, "10.0.0.1", SortBy::Cpu);
        assert!(s.contains("exec \"$A\" monitor '10.0.0.1' 80 24 cpu"), "{s}");
        assert!(s.contains("[ -x \"$A\" ]"));
    }

    /// Resolving the architecture remotely is what keeps a warm start to one
    /// round trip on both x86 and ARM hosts.
    #[test]
    fn bootstrap_resolves_arch_remotely() {
        let s = bootstrap_script(Mode::Monitor, "1.2.3.4", SortBy::Cpu);
        assert!(s.contains("M=$(uname -m)"));
        assert!(s.contains("x86_64|amd64)"));
        assert!(s.contains("aarch64|arm64)"));
        assert!(s.contains(&agent_path(HASH_X86_64)));
        assert!(s.contains(&agent_path(HASH_AARCH64)));
    }

    #[test]
    fn bootstrap_reports_arch_on_miss() {
        let s = bootstrap_script(Mode::Monitor, "1.2.3.4", SortBy::Cpu);
        assert!(s.contains("echo \"===NEEDAGENT=== $M\""));
    }

    #[test]
    fn bootstrap_selects_docker_mode() {
        assert!(bootstrap_script(Mode::Docker, "1.2.3.4", SortBy::Cpu).contains("\" docker "));
    }

    #[test]
    fn bootstrap_quotes_the_display_address() {
        let s = bootstrap_script(Mode::Monitor, "a'b", SortBy::Cpu);
        assert!(s.contains(r"'a'\''b'"), "{s}");
    }

    /// An unknown architecture must fall through to the NEEDAGENT branch and
    /// produce a clear message, not exec an empty path.
    #[test]
    fn bootstrap_handles_unknown_arch() {
        let s = bootstrap_script(Mode::Monitor, "x", SortBy::Cpu);
        assert!(s.contains("*) A=\"\" ;;"));
        assert!(s.contains("[ -n \"$A\" ]"));
    }

    #[test]
    fn upload_is_atomic_and_unique() {
        let c = upload_command("deadbeef", "t1");
        assert!(c.contains("mkdir -p ~/.cache/multitop"));
        assert!(c.contains("chmod 755"));
        // Staged under a unique name, then renamed into place.
        assert!(c.contains("agent-deadbeef.t1"));
        assert!(c.contains(
            "mv -f ~/.cache/multitop/agent-deadbeef.t1 ~/.cache/multitop/agent-deadbeef"
        ));
        assert!(!upload_command("deadbeef", "t2").contains(".t1"));
    }

    /// The upload target and the bootstrap lookup must agree, or every start
    /// re-uploads the agent.
    #[test]
    fn upload_path_matches_bootstrap_path() {
        let boot = bootstrap_script(Mode::Monitor, "x", SortBy::Cpu);
        for arch in [Arch::X86_64, Arch::Aarch64] {
            let up = upload_command(arch.hash(), "tok");
            let suffix = format!("~/.cache/multitop/agent-{}", arch.hash());
            assert!(up.ends_with(&suffix), "{up}");
            assert!(
                boot.contains(&format!(".cache/multitop/agent-{}", arch.hash())),
                "{boot}"
            );
        }
    }

    #[test]
    fn need_agent_parsed() {
        assert_eq!(parse_need_agent("===NEEDAGENT=== x86_64"), Some("x86_64"));
        assert_eq!(parse_need_agent("===NEEDAGENT=== aarch64\n"), Some("aarch64"));
        assert_eq!(parse_need_agent("===MONITOR==="), None);
        assert_eq!(parse_need_agent("some output"), None);
    }

    #[test]
    fn need_agent_round_trips_with_arch() {
        for label in ["x86_64", "aarch64"] {
            let arch = Arch::from_uname(parse_need_agent(&format!("{NEED_AGENT} {label}")).unwrap());
            assert_eq!(arch.map(Arch::label), Some(label));
        }
    }

    #[test]
    fn ssh_opts_enable_multiplexing_and_keepalive() {
        let joined = SSH_OPTS.join(" ");
        assert!(joined.contains("ControlMaster=auto"));
        assert!(joined.contains("ControlPersist=30s"));
        assert!(joined.contains("ServerAliveInterval=15"));
        assert!(joined.contains("ConnectTimeout=10"));
    }

    #[test]
    fn target_includes_user_when_set() {
        let mut s = server();
        assert_eq!(s.target(), "10.0.0.1");
        s.user = "admin".into();
        assert_eq!(s.target(), "admin@10.0.0.1");
    }

    #[test]
    fn missing_agent_reports_how_to_fix() {
        if Arch::X86_64.binary().is_none() {
            let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
            let e = rt.block_on(upload_agent(&server(), Arch::X86_64, "t")).unwrap_err();
            assert!(e.contains("build.sh"), "{e}");
        }
    }
}
