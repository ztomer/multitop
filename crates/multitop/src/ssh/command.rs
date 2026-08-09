use crate::config::Server;
use crate::ssh_opts::{sh_quote, Mode, HASH_AARCH64, HASH_X86_64, NEED_AGENT, SSH_OPTS};
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
    cmd.arg("-p").arg(server.port.to_string());
    cmd.arg(server.target().as_ref());
    cmd
}

/// Printed by the remote once it has turned echo off and is ready to read the
/// password. The caller waits for this line before writing, because the pty
/// echoes whatever arrives before `stty -echo` has run.
pub const PW_READY_SENTINEL: &str = "__multitop_pw_ready__";

/// Printed by the remote when `sudo` refuses the password it was handed.
///
/// Without a distinct signal, a rejected password and a failing upgrade command
/// are the same thing to the caller: `preamble && command` exits 1 either way.
/// The panel then said "upgrade command exited 1 -- host reachable, command
/// failed" for a command that never ran, which sent the user looking at their
/// upgrade script instead of at their password.
pub const SUDO_FAILED_SENTINEL: &str = "__multitop_sudo_failed__";

/// Exit status the remote uses for that case, so it survives even if the line
/// is lost in a noisy login shell.
pub const SUDO_FAILED_CODE: i32 = 111;

/// Printed by the remote when the upgrade lock is already held by another run.
///
/// A held lock used to end in `echo "Upgrade already in progress"; exit 1`,
/// which landed in the generic arm and the panel said "upgrade command exited 1
/// -- host reachable, command failed". It did not fail; it never ran. The lock
/// is only broken after six hours, so for six hours every attempt pointed the
/// operator at an apt command that was fine.
pub const LOCK_HELD_SENTINEL: &str = "__multitop_lock_held__";

/// Exit status for that case, so it is still recognisable if the line is lost.
pub const LOCK_HELD_CODE: i32 = 125;

/// Take the sudo password on stdin instead of putting it in the command.
///
/// The password used to be interpolated as `echo '<password>' | sudo -S`, and
/// that whole string was passed as argv -- to `ssh` locally, and by sshd to the
/// login shell remotely. Process arguments are not secret: `/proc/<pid>/cmdline`
/// is world-readable on Linux, so every user on a monitored host could read that
/// host's sudo password for as long as an upgrade ran.
///
/// Nothing here holds the password in an argument. `read` puts it in a shell
/// variable, and `printf` is a builtin in sh, bash and zsh, so piping it to
/// `sudo` spawns no process that could carry it. Echo is turned off first
/// because `-tt` allocates a pty, whose line discipline would otherwise print
/// the password straight back into the panel.
#[must_use]
pub fn password_preamble() -> String {
    format!(
        "stty -echo 2>/dev/null; printf '{PW_READY_SENTINEL}\\n'; IFS= read -r __mt_pw; \
         stty echo 2>/dev/null; printf '%s\\n' \"$__mt_pw\" | sudo -S -p '' -v 2>/dev/null; \
         __mt_rc=$?; unset __mt_pw; \
         if [ $__mt_rc -ne 0 ]; then printf '{SUDO_FAILED_SENTINEL}\\n'; exit {SUDO_FAILED_CODE}; fi"
    )
}

#[must_use]
pub fn ssh_command_tty(server: &Server) -> Command {
    let mut cmd = detached(Command::new("ssh"));
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

#[must_use]
pub fn is_local(server: &Server) -> bool {
    server.host == "localhost" || server.host == "127.0.0.1" || server.port == 0
}
