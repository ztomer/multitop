//! Point the local-agent resolver at this build's own `multitop`.
//!
//! `spawn_exec` and `spawn_local_agent` run `MULTITOP_AGENT_EXE`, or the
//! running executable when that is a `multitop` build. A test binary is
//! neither, so the tests name this build's `multitop` through the seam --
//! cargo hands an integration test the path of every binary in its package --
//! and it serves as its own agent with `--agent`, exactly as the installed
//! binary does. This holds in every build layout; the old sibling-directory
//! guess held in one.

use std::sync::Once;

pub fn use_this_builds_agent() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| std::env::set_var("MULTITOP_AGENT_EXE", env!("CARGO_BIN_EXE_multitop")));
}
