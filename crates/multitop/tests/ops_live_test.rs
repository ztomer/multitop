//! The Ops view against a LIVE machine: the estate's `.90`, over ssh.
//!
//! The question the in-memory fake cannot answer: does the view draw what
//! a REAL `mcp_host` says - the machine's tunnel and the stack that
//! declares it, the machine's capacity, the pools' state - over a real
//! session, on the estate's own box?
//!
//! It changes NOTHING on the host: it opens the server's own read-only
//! tools and asks.
//!
//! `#[ignore]`d by default, so the gate never depends on the estate being
//! up. Run it by hand:
//!
//! ```text
//! cargo test -p multitop --test ops_live_test -- --ignored --nocapture
//! ```
//!
//! The machine and the command are the environment's to say, so the test is
//! not pinned to one person's network:
//!
//! ```text
//! MULTITOP_LIVE_HOST=192.168.0.90 \
//! MULTITOP_LIVE_MCP='/home/ztomer/prj/storage-server/bin/mcp_host --root /home/ztomer/prj/storage-server' \
//!   cargo test -p multitop --test ops_live_test -- --ignored --nocapture
//! ```

use std::time::Duration;

use multitop::config::Server;
use multitop::ops::render::render;
use multitop::ops::{gather, OpsState, Source};

/// The machine the estate's storage server lives on.
const DEFAULT_HOST: &str = "192.168.0.90";
/// The command that starts that machine's server.
const DEFAULT_MCP: &str =
    "/home/ztomer/prj/storage-server/bin/mcp_host --root /home/ztomer/prj/storage-server";
/// The command that starts the SOLVER root's own server (the machine's
/// second instance, and the one that owns the tunnel today).
const DEFAULT_SOLVER_MCP: &str =
    "/home/ztomer/prj/solver/bin/mcp_host --root /home/ztomer/prj/solver";
/// How long the session may take to open.
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);
/// The width of a panel in the four-panel layout: the narrowest frame the
/// view draws, and the one an operator actually reads.
const NARROW_COLS: usize = 38;
/// The width of the whole screen when the view has the room.
const WIDE_COLS: usize = 100;

fn server(host: &str, mcp: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "ztomer".to_string(),
        name: std::env::var("MULTITOP_LIVE_NAME").ok(),
        mcp: Some(mcp.to_string()),
        ..Server::default()
    }
}

/// The frame the view draws for `server`, as plain lines, at `cols`.
async fn frame(server: &Server, cols: usize) -> Option<Vec<String>> {
    let source = Source::of(server)?;
    let mut session = match multitop::mcp::spawn::open(server, &source, OPEN_TIMEOUT).await {
        Ok(s) => s,
        Err(why) => {
            println!("the session did not open: {why}");
            return None;
        }
    };
    let snapshot = gather(
        &mut session,
        multitop::tasks::ops_poll::now_epoch(),
        multitop::ops::Previous::default(),
    )
    .await;
    let state = OpsState::Ready(Box::new(snapshot));
    let pal = multitop_agent::palette_view::PaletteView::for_theme(&multitop_agent::color::KARE);
    let lines = render(
        &multitop::ops::banner(server),
        &state,
        cols,
        60,
        multitop::tasks::ops_poll::now_epoch(),
        &pal,
    );
    Some(
        lines
            .iter()
            .map(|l| multitop_agent::escape::strip(l))
            .collect(),
    )
}

/// The machine's own server: the tunnel, the capacity and the pools are all
/// drawn, and the tunnel's section is never empty.
#[tokio::test]
#[ignore = "needs the estate"]
async fn the_machine_draws_its_tunnel_and_its_capacity() {
    let host = std::env::var("MULTITOP_LIVE_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_string());
    let mcp = std::env::var("MULTITOP_LIVE_MCP").unwrap_or_else(|_| DEFAULT_MCP.to_string());
    let Some(f) = frame(&server(&host, &mcp), WIDE_COLS).await else {
        panic!("the machine's server did not answer");
    };
    println!("--- the machine, {WIDE_COLS} columns ---");
    for line in &f {
        println!("{line}");
    }
    if let Some(narrow) = frame(&server(&host, &mcp), NARROW_COLS).await {
        println!("--- the machine, {NARROW_COLS} columns (a panel in the four-panel layout) ---");
        for line in &narrow {
            println!("{line}");
        }
    }
    assert!(
        f.iter().any(|l| l.contains("vpn")),
        "the tunnel's section is drawn one way or the other: {f:?}"
    );
    assert!(
        f.iter().any(|l| l.contains("disk") || l.contains("zfs")),
        "the machine's capacity or its pools: {f:?}"
    );
    // A tunnel that IS drawn names its container, and the stack that
    // declares it when the machine says which stack that is.
    for line in f.iter().filter(|l| l.contains("via")) {
        let name = line.split(" via ").nth(1).unwrap_or_default();
        assert!(!name.is_empty(), "the stack is named: {line}");
    }
}

/// The machine's SECOND instance, whose server owns the tunnel today: the
/// tunnel is drawn, and the stack that declares it is named.
#[tokio::test]
#[ignore = "needs the estate"]
async fn the_second_instance_draws_the_tunnel_with_its_stack() {
    let host = std::env::var("MULTITOP_LIVE_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_string());
    let mcp = std::env::var("MULTITOP_LIVE_SOLVER_MCP")
        .unwrap_or_else(|_| DEFAULT_SOLVER_MCP.to_string());
    let Some(f) = frame(&server(&host, &mcp), WIDE_COLS).await else {
        println!("the second instance ships no server of its own any more");
        return;
    };
    println!("--- the second instance, {WIDE_COLS} columns ---");
    for line in &f {
        println!("{line}");
    }
    let tunnel = f
        .iter()
        .find(|l| l.contains("via") || l.contains("solver-vpn"))
        .expect("the tunnel is drawn, with the stack that declares it");
    assert!(
        tunnel.contains("solver-vpn"),
        "the tunnel's container is named: {tunnel}"
    );
}
