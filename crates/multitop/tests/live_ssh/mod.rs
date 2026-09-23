//! The host the live ssh suites run against.
//!
//! `ssh::is_local` treats `localhost`, `127.0.0.1` and port 0 as this
//! machine and runs the agent here with no ssh at all, so a live suite aimed
//! at one of them passes without ever opening a connection - which is what
//! the old default, `127.0.0.1`, did. These suites exist to test the ssh
//! path, so a target that would skip it is refused, and there is no default.
//!
//! To test over ssh against this machine, name it through an ssh alias -
//! `Host multitop-loop` / `HostName 127.0.0.1` in `~/.ssh/config`, with sshd
//! running (Remote Login on macOS): ssh resolves the alias, and multitop
//! never sees a loopback name.

use multitop::config::Server;
use multitop::ssh::is_local;

const HOST_VAR: &str = "MULTITOP_TEST_SSH_HOST";
const USER_VAR: &str = "MULTITOP_TEST_SSH_USER";
const PORT_VAR: &str = "MULTITOP_TEST_SSH_PORT";
/// ssh's own default port, used when `MULTITOP_TEST_SSH_PORT` is unset.
const SSH_PORT: u16 = 22;

/// The live-suite server for `cmd`, or why it would not test ssh.
///
/// # Errors
///
/// No host named, a port that is not a number, or a target multitop runs
/// locally.
pub fn target(
    host: Option<&str>,
    user: &str,
    port: Option<&str>,
    cmd: &str,
) -> Result<Server, String> {
    let host = host.filter(|h| !h.is_empty()).ok_or_else(|| {
        format!("{HOST_VAR} is not set: name an ssh host (an ~/.ssh/config alias works)")
    })?;
    let port = port.map_or(Ok(SSH_PORT), |p| {
        p.parse::<u16>().map_err(|e| format!("{PORT_VAR}={p}: {e}"))
    })?;
    let server = Server {
        host: host.to_string(),
        port,
        user: user.to_string(),
        upgrade_cmd: Some(cmd.to_string()),
        custom_command: None,
        mcp: None,
    };
    if is_local(&server) {
        return Err(format!(
            "{host} port {port} is local to multitop (ssh::is_local): it runs the agent \
             here without ssh. Point {HOST_VAR} at an ssh alias for this machine \
             instead (Host multitop-loop / HostName 127.0.0.1)"
        ));
    }
    Ok(server)
}

/// The live-suite server for `cmd`, from the environment.
///
/// # Panics
///
/// When [`target`] refuses it: a live test must not pass without ssh.
pub fn ssh_server(cmd: &str) -> Server {
    let env = |k: &str| std::env::var(k).ok();
    let user = env(USER_VAR)
        .or_else(|| env("USER"))
        .unwrap_or_else(|| "root".to_string());
    target(
        env(HOST_VAR).as_deref(),
        &user,
        env(PORT_VAR).as_deref(),
        cmd,
    )
    .unwrap_or_else(|why| panic!("{why}"))
}
