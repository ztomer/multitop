//! The HEADLESS modes: the HTTP companion (`--serve`) and the Ops poll
//! (`--ops-poll`).
//!
//! Both are the same binary with no terminal: one serves the cards to a
//! phone, the other asks the estate for them and keeps the answers. The
//! poll is a JOB (servers ROADMAP E36) - it was bound to the Ops view, so
//! it ran only while a human was watching, and the estate's fingerprint
//! tools fired 0.3 a day while the shared tools fired 87.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::config;

/// The `--serve` address: `:8080`, `8080` and `127.0.0.1:8080` all mean the
/// loopback port; anything else must parse as a socket address.
///
/// # Errors
///
/// The address is neither a port nor a socket address.
pub fn serve_socket_addr(addr: &str) -> Result<std::net::SocketAddr, String> {
    if let Ok(a) = addr.parse() {
        return Ok(a);
    }
    let a = if addr.starts_with(':') {
        format!("127.0.0.1{addr}")
    } else if addr.chars().all(|c| c.is_ascii_digit()) {
        format!("127.0.0.1:{addr}")
    } else {
        addr.to_string()
    };
    a.parse()
        .map_err(|e| format!("Invalid --serve address '{addr}': {e}"))
}

/// `--serve`: the headless HTTP companion, reusing the MTOP pipeline. Runs
/// until the server stops; the exit code is the outcome.
pub fn serve_headless(
    addr: &str,
    token_file: Option<PathBuf>,
    config_path: &Path,
    servers: Vec<config::Server>,
    initial_theme: Option<String>,
) -> ExitCode {
    // Need to load full config for health thresholds
    let cfg = config::load(config_path).unwrap_or_else(|_| config::Config {
        servers: servers.clone(),
        theme: initial_theme,
        upgrade_history_lines: crate::config::DEFAULT_UPGRADE_HISTORY_LINES,
        history_lines_raised_from: None,
        banner_style: crate::layout::BannerStyle::default(),
        plaintext_passwords: vec![],
        alert_cpu: None,
        alert_mem: None,
        alert_disk: None,
        alerts: vec![],
    });
    let addr = match serve_socket_addr(addr) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[Error] {e}");
            return ExitCode::FAILURE;
        }
    };
    // Any failure -- the CSPRNG, a file, its mode -- REFUSES to serve rather than
    // falling back to a weaker token or to plain HTTP on a LAN bind.
    let token_path = token_file.unwrap_or_else(|| crate::serve_token::default_path(config_path));
    let to_tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
    let ready = match crate::serve_start::prepare(
        addr,
        &token_path,
        to_tty,
        crate::pairing::route_source,
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[Error] {e} -- refusing to start --serve");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("{}", ready.banner);
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[Error] Could not start async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let code = runtime.block_on(async move {
        let live =
            std::sync::Arc::new(tokio::sync::RwLock::new(crate::server::LiveState::default()));
        let app_state = crate::server::AppState {
            live: live.clone(),
            token: ready.token,
            servers: servers.clone(),
            config: cfg,
        };
        // Sort for collectors
        let sort = multitop_agent::SortBy::Cpu;
        crate::server::spawn_collectors(servers, &live, sort);
        crate::server::serve(addr, app_state, ready.tls).await
    });
    match code {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[Error] {e}");
            ExitCode::FAILURE
        }
    }
}
/// `--ops-poll`: ONE pass of the Ops poll, and exit.
///
/// The poll is a JOB, not a screen (servers ROADMAP E36): it was bound to
/// the Ops view, so it ran only while a human was watching the panel, and
/// the estate's fingerprint tools fired 0.3 a day while the shared tools
/// fired 87. This is the pass the scheduler runs.
///
/// One pass, not a daemon: a pass that exits cannot be left half-dead, and
/// the next pass opens a fresh session. The exit code is the outcome - a
/// host that could not be reached is a failure the operator has to see.
#[must_use]
pub fn ops_poll_pass(servers: &[config::Server], config_path: &Path) -> ExitCode {
    let cache = crate::ops::cache::Cache::beside(config_path);
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[Error] Could not start async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let code = runtime.block_on(async {
        let pass = crate::ops::pass::run_real(
            servers,
            &cache,
            crate::tasks::ops_poll::now_epoch(),
            crate::tasks::ops_poll::OPS_POLL,
        )
        .await;
        for h in &pass.hosts {
            println!("{} · {}", h.host, said(h));
        }
        pass.complete()
    });
    if code {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// What the pass says about one host, in the operator's words.
///
/// A host that was gathered is "gathered" - and when a section's answer
/// did not arrive, the GAP is named beside it: the reading is not
/// complete, and the log says which fact is missing (servers ROADMAP
/// E74). A host the pass could not reach at all is "failed", with the
/// reason.
fn said(h: &crate::ops::pass::HostPass) -> String {
    let state = if h.gathered {
        "gathered"
    } else if h.fresh {
        "already fresh"
    } else {
        return format!("failed: {}", h.why.as_deref().unwrap_or("no reason given"));
    };
    if h.unknown.is_empty() {
        return state.to_string();
    }
    format!("{state} · {}", h.unknown.join(" · "))
}

#[cfg(test)]
#[path = "headless_tests.rs"]
mod tests;

/// `--ops-frame <HOST>`: the Ops frame for one host, printed, uncoloured.
///
/// The monitor draws its Ops view in its TUI and keeps no copy of the
/// DRAWING, so a check that compares the two clients' panels had to read
/// the monitor's screen through tmux and say out loud that the rendered
/// half was a stand-in (servers ROADMAP O90). This is the seam: one
/// command that prints the frame, from the client's own cache - the same
/// answers the view draws from, and the same renderer.
///
/// Read-only, and it asks the estate for nothing: the frame is drawn from
/// the answer the poll already found.
#[must_use]
pub fn ops_frame_pass(servers: &[config::Server], config_path: &Path, host: &str) -> ExitCode {
    match ops_frame_lines(servers, config_path, host) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("[Error] {why}");
            ExitCode::FAILURE
        }
    }
}

/// The Ops frame for `host`, as the lines the view draws.
///
/// The host is named by the name the operator gave it ("beelink") or by
/// its address ("192.168.0.33"). The frame is drawn from the client's own
/// cache - the answer the poll found - with the colour taken out, so a
/// checker can read it.
///
/// # Errors
///
/// The config names no such host, or this client holds no answer for it.
pub fn ops_frame_lines(
    servers: &[config::Server],
    config_path: &Path,
    host: &str,
) -> Result<Vec<String>, String> {
    let server = find_server(servers, host)
        .ok_or_else(|| format!("no host named `{host}` in the config"))?;
    let cache = crate::ops::cache::Cache::beside(config_path);
    let key = crate::ops::cache::host_key(server);
    let raw = cache
        .read(&key)
        .ok_or_else(|| format!("this client holds no answer for `{host}` yet"))?;
    let snapshot = crate::ops::parse_snapshot(&raw);
    let state = crate::ops::OpsState::Ready(Box::new(snapshot));
    // The frame, uncoloured: the picture is the glyph, and a checker reads
    // text.
    let pal = multitop_agent::palette_view::PaletteView::for_theme(&multitop_agent::color::PLAIN);
    Ok(crate::ops::render::render(
        &crate::ops::banner(server),
        &state,
        FRAME_COLS,
        FRAME_ROWS,
        crate::tasks::ops_poll::now_epoch(),
        &pal,
    ))
}

/// The host `name` names: by the name the operator gave it, or by its
/// address. The name is matched case-insensitively, because a name is
/// something a person types.
fn find_server<'a>(servers: &'a [config::Server], name: &str) -> Option<&'a config::Server> {
    let want = name.trim().to_lowercase();
    servers
        .iter()
        .find(|s| {
            s.name
                .as_deref()
                .is_some_and(|n| n.trim().to_lowercase() == want)
        })
        .or_else(|| servers.iter().find(|s| s.host.to_lowercase() == want))
}

/// How wide the printed frame is: the house's line budget, so nothing the
/// frame says is cut off by the width.
const FRAME_COLS: usize = 240;
/// How tall the printed frame is: every section, and nothing hidden.
const FRAME_ROWS: usize = 200;
