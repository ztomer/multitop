//! Point the local-agent resolver at this build's own `multitop`.
//!
//! `spawn_exec` and `spawn_local_agent` run `MULTITOP_AGENT_EXE`, or the
//! running executable when that is a `multitop` build. A test binary is
//! neither, so the tests name this build's `multitop` through the seam --
//! cargo hands an integration test the path of every binary in its package --
//! and it serves as its own agent with `--agent`, exactly as the installed
//! binary does. This holds in every build layout; the old sibling-directory
//! guess held in one.
//!
//! A test that forgets the seam does not fail where it should: it falls back
//! to whatever `multitop-agent` is on `PATH` -- an installed release on a dev
//! Mac, nothing at all on CI -- or passes only because a sibling test in the
//! same binary set it first. `local_server_seam_test` holds every local
//! server in the suite to `local_server` below.

use std::sync::Once;

use multitop::config::Server;

/// Set the seam. Only a test that spawns the agent without a `Server` calls
/// this directly; everything else gets it through `local_server`.
pub fn use_this_builds_agent() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| std::env::set_var("MULTITOP_AGENT_EXE", env!("CARGO_BIN_EXE_multitop")));
}

/// A local server (port 0: no ssh, the agent runs here) whose agent is
/// this build's. Every test's local server comes from here, so none can
/// run against another agent or depend on a sibling test setting the seam.
/// Customise with struct update: `Server { user: .., ..local_server(h) }`.
pub fn local_server(host: &str) -> Server {
    use_this_builds_agent();
    Server {
        host: host.to_string(),
        port: 0,
        user: String::new(),
        upgrade_cmd: None,
        custom_command: None,
        mcp: None,
    }
}
