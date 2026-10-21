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
            let said = if h.gathered {
                "gathered".to_string()
            } else if h.fresh {
                "already fresh".to_string()
            } else {
                format!("failed: {}", h.why.as_deref().unwrap_or("no reason given"))
            };
            println!("{} · {said}", h.host);
        }
        pass.complete()
    });
    if code {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
