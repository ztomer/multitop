use crate::config::Server;
use crate::ssh_opts::{
    active_multiplex_opts, sh_quote, Mode, HASH_AARCH64, HASH_X86_64, NEED_AGENT, SSH_OPTS,
};
use multitop_agent::SortBy;
use std::fmt::Write;
use tokio::process::Command;

//  and then never happens again for that build.

/// Remote cache path for a given build of the agent.
#[must_use]
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
#[must_use]
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
///
/// # Why the size is checked before the `mv`
///
/// `cat` cannot tell a finished stream from an interrupted one -- both end in
/// EOF. So a connection that dropped partway through a multi-megabyte upload
/// left `cat` succeeding on a short file, `chmod` succeeding, `mv` succeeding,
/// and the whole command **exiting 0 with a truncated binary installed as the
/// agent**. The local side reported a successful install; the next connection
/// then failed to exec it, and the panel blamed the architecture or the
/// bootstrap for a file this program had put there itself.
///
/// `expected` is the length of the bytes about to be written, which is the one
/// thing the remote cannot work out for itself. A short file now fails the
/// check, the staging file is removed rather than left to accumulate, and the
/// command exits non-zero so the failure is reported instead of installed.
#[must_use]
pub fn upload_command(hash: &str, token: &str, expected: usize) -> String {
    let dir = "~/.cache/multitop";
    let final_path = format!("{dir}/agent-{hash}");
    let staging = format!("{final_path}.{token}");
    // `tr -d` because `wc -c` pads its output with leading blanks on some
    // systems, and `[ "  123" = "123" ]` is false.
    format!(
        "mkdir -p {dir} && cat > {staging} \
         && [ \"$(wc -c < {staging} | tr -d '[:space:]')\" = \"{expected}\" ] \
         && chmod 755 {staging} && mv -f {staging} {final_path} \
         || {{ rm -f {staging}; echo \"agent upload was incomplete\" >&2; exit 1; }}"
    )
}

/// Shell command that removes stale agent binaries from the cache directory.
///
/// Keeps only the current `x86_64` and aarch64 agent hashes, removing everything
/// else that matches the `agent-*` pattern. Safe to run concurrently — each
/// `rm` targets a specific file, not the directory.
#[must_use]
pub fn cleanup_old_agents_command() -> String {
    let keep = [HASH_X86_64, HASH_AARCH64];
    let keep_patterns: Vec<String> = keep
        .iter()
        .filter(|h| !h.is_empty() && **h != "missing")
        .map(|h| format!("agent-{h}"))
        .collect();
    // Nothing to keep means nothing to do. Without this the `case` below has
    // no `continue` arm and the catch-all deletes *every* agent, including the
    // one currently in use.
    //
    // A build with no agent embedded -- a plain `cargo build`, as opposed to
    // `./build.sh` -- has `HASH_* == "missing"` for both, so that is the
    // command it generates today. It is unreachable only because the sweep runs
    // after a successful upload and an upload needs an embedded agent: the
    // safety lives in a different function's ordering rather than here. A sweep
    // that does not know what to keep must delete nothing.
    if keep_patterns.is_empty() {
        return String::from(":");
    }
    let mut cmd = String::from("cd ~/.cache/multitop 2>/dev/null && for f in agent-*; do\n");
    cmd.push_str("  case \"$f\" in\n");
    for pattern in &keep_patterns {
        let _ = writeln!(cmd, "    {pattern}) continue ;;");
    }
    cmd.push_str("    agent-*) rm -f \"$f\" ;;\n");
    cmd.push_str("  esac\ndone");
    cmd
}

/// Exit status the exec bootstrap uses when it found no agent to run.
///
/// Distinct from anything a user's command can return, so "this host has no
/// agent for its architecture" is never reported as "your upgrade command
/// failed".
pub const NEED_AGENT_CODE: i32 = 3;

/// The remote command for exec mode, as one argument.
///
/// # Why this one is an argument when the streaming bootstrap is on stdin
///
/// [`bootstrap_script`] is delivered on stdin precisely so the user's login
/// shell never parses it. That cannot work here, and the reason was measured
/// rather than assumed: a probe that wrote the script and then the request
/// frame to the same stdin got `no readable exec request on stdin` back from
/// all three test hosts. `sh` reading a script from a pipe reads ahead, and the
/// frame went into its buffer and died there.
///
/// So the bootstrap moves to argv and stdin is left for the frame. What makes
/// that acceptable is *what* is in the argument: an architecture `case` and two
/// paths built from this build's own hashes. The command and the sudo password
/// stay on stdin, where they were always the point -- `/proc/<pid>/cmdline` is
/// world-readable, and none of it appears there.
///
/// The quoting is ours, not `ssh`'s. `ssh` concatenates its arguments with
/// spaces and hands the string to the remote login shell, so passing
/// `["sh", "-c", script]` loses every quote in `script` -- measured too: `$(uname
/// -m)` was evaluated by the login shell and the case fell through to "no
/// agent".
#[must_use]
pub fn exec_bootstrap_arg() -> String {
    let script = format!(
        "M=$(uname -m); \
         case \"$M\" in \
         x86_64|amd64) A={x86} ;; \
         aarch64|arm64) A={arm} ;; \
         *) A=\"\" ;; \
         esac; \
         if [ -n \"$A\" ] && [ -x \"$A\" ]; then exec \"$A\" exec; fi; \
         echo \"{NEED_AGENT} $M\" >&2; exit {NEED_AGENT_CODE}",
        x86 = agent_path(HASH_X86_64),
        arm = agent_path(HASH_AARCH64),
    );
    format!("sh -c {}", sh_quote(&script))
}

/// Parse a `===NEEDAGENT=== <arch>` line into its architecture field.
pub fn parse_need_agent(line: &str) -> Option<&str> {
    line.trim().strip_prefix(NEED_AGENT).map(str::trim)
}

/// Put a child in its own process group, so it cannot reach this process's
/// controlling terminal.
///
/// Every child here inherits multitop's terminal, and multitop holds that
/// terminal in raw mode inside the alternate screen. A child that opens
/// `/dev/tty` -- which is exactly what `ssh` does for a passphrase or an
/// unknown host key, and `sudo` for a local password, whatever their stdin is
/// connected to -- then writes its prompt over the frame and reads the answer
/// out of the keystrokes the event loop is reading. Both sides get half the
/// input and the display is wrecked.
///
/// In its own group the child is no longer in the foreground group, so the
/// kernel refuses it the terminal (`SIGTTIN`/`SIGTTOU`) instead. Combined with
/// `BatchMode=yes`, which stops `ssh` reaching for it in the first place, the
/// failure becomes a message in the panel.
///
/// One helper rather than a call at each spawn site: there are four, and the
/// next one added would be the one that forgets.
#[must_use]
pub fn detached(mut cmd: Command) -> Command {
    #[cfg(unix)]
    cmd.process_group(0);
    cmd
}

#[must_use]
pub fn ssh_command(server: &Server) -> Command {
    let mut cmd = detached(Command::new("ssh"));
    cmd.env("LC_ALL", "C").env("LANG", "C");
    cmd.args(SSH_OPTS);
    cmd.args(active_multiplex_opts());
    cmd.arg("-p").arg(server.port.to_string());
    cmd.arg(server.target().as_ref());
    cmd
}

// The four sentinels and their two exit codes lived here. They are the agent's
// now -- `multitop_agent::exec` -- because both ends need one definition and
// only one of them may own it, and the end that owns the pty is the end that
// can tell what a line is. They reach this side as `MarkerKind` frames.

// `password_preamble` and `ssh_command_tty` lived here.
//
// The preamble is the agent's `exec::script::wrap` now, unchanged in shape and
// unchanged in its reason: `/proc/<pid>/cmdline` is world-readable, so the
// password goes through a shell variable and a builtin `printf`, never argv.
//
// `ssh_command_tty` is gone outright, and with it the last `-tt` in this
// program. It was the one thing asking `ssh` to allocate a pty, and what a pty
// meant depended on whether `ssh` had multiplexed -- which is the whole defect.
// The agent allocates the pty now, on the host, where the answer is the same
// every time.

#[must_use]
pub fn is_local(server: &Server) -> bool {
    server.host == "localhost" || server.host == "127.0.0.1" || server.port == 0
}
