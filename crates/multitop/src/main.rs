//! multitop — watch several servers at once in one terminal.

// The one crate with no FFI: unsafe stays denied here (see the workspace
// Cargo.toml note and tools/check_unsafe_scope.py).
#![deny(unsafe_code)]

use multitop::{config, run, ssh};

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
multitop - multi-server TUI dashboard over SSH

USAGE:
    multitop [OPTIONS]

OPTIONS:
    -c, --config <PATH>    Config file (default: ~/.config/multitop/config.toml)
    -r, --remote <HOSTS>   Override config with comma-separated remote hosts/IPs
        --local            Include local machine (localhost) in server list
        --local-only       Monitor local machine only (no config or SSH required)
        --serve [<ADDR>]   Run as headless HTTP companion (default :8080)
        --serve-token-file <PATH>  Token file for --serve, created 0600 if absent
                           (default: serve-token beside the config file)
    -h, --help             Print this help
    -V, --version          Print version
    --print-background    Which background the terminal reported, and from where

KEYS:
    ESC / q    Quit
    f          Toggle Fastfetch system info view on every panel
    d          Toggle the Docker view on every panel
    p          Ops view: each host's mcp_host (health, cron, containers, alerts)
    s          Back to live stats
    u          Run each server's configured upgrade_cmd
";

const SERVE_TOKEN_ON_ARGV: &str = "--serve-token is gone: a token on the command line \
is readable by every local user (ps). Put it in a file only you can read (chmod 600) and \
pass --serve-token-file <PATH>, or let --serve create one beside the config file.";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CliOptions {
    pub config_path: Option<PathBuf>,
    pub local: bool,
    pub local_only: bool,
    pub remote_hosts: Vec<String>,
    pub serve_addr: Option<String>,
    pub serve_token_file: Option<PathBuf>,
}

enum Startup {
    Run(CliOptions),
    Agent(Vec<String>),
    Print(String),
    Fail(String),
}

fn parse_cli<I: IntoIterator<Item = String>>(argv: I) -> Startup {
    let mut iter = argv.into_iter().peekable();
    let mut opts = CliOptions::default();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => return Startup::Print(USAGE.to_string()),
            "-V" | "--version" => {
                return Startup::Print(format!("multitop {}", env!("CARGO_PKG_VERSION")))
            }
            "--print-background" => {
                // Which branch the palette took, and what it decided. A probe
                // that reports "it looked fine" is how a feature ends up wired to
                // nothing and green: the background is detected, stored, adapted
                // -- and none of that is visible from the outside, so a real
                // check needs a real question. This is the app's own answer, on
                // the app's own terminal, through the same `detect` the render
                // path uses.
                let found = multitop::background::detect();
                return Startup::Print(format!(
                    "background: {:?}\n  source: {}\n  luminance: {:.4}\n  \"{}\" is set: {}",
                    found,
                    found.source(),
                    found
                        .or_theme(multitop_agent::color::KARE.own_background())
                        .relative_luminance(),
                    "COLORFGBG",
                    std::env::var("COLORFGBG").unwrap_or_else(|_| "<unset>".into()),
                ));
            }
            "-c" | "--config" => match iter.next() {
                Some(p) => opts.config_path = Some(PathBuf::from(p)),
                None => return Startup::Fail("--config requires a path".into()),
            },
            "-r" | "--remote" => match iter.next() {
                Some(remotes) => {
                    for h in remotes.split(',') {
                        let trimmed = h.trim();
                        if !trimmed.is_empty() {
                            opts.remote_hosts.push(trimmed.to_string());
                        }
                    }
                }
                None => return Startup::Fail("--remote requires host or IP list".into()),
            },
            "--local" => opts.local = true,
            "--local-only" => opts.local_only = true,
            "--serve" => {
                // --serve, --serve <addr>, --serve=<addr> all supported.
                // Use peek to see if next token is an addr (not a flag).
                let addr = match iter.peek() {
                    Some(next) if !next.starts_with('-') => iter.next().unwrap_or_default(),
                    _ => ":8080".to_string(),
                };
                opts.serve_addr = Some(addr);
            }
            "--serve-token-file" => match iter.next() {
                Some(p) => opts.serve_token_file = Some(PathBuf::from(p)),
                None => return Startup::Fail("--serve-token-file requires a path".into()),
            },
            // Refused, not accepted: a token on the command line is readable by every
            // local user through `ps` (ROADMAP A1).
            "--serve-token" => return Startup::Fail(SERVE_TOKEN_ON_ARGV.into()),
            "--agent" => {
                let rest: Vec<String> = iter.collect();
                return Startup::Agent(rest);
            }
            other => {
                if let Some(addr) = other.strip_prefix("--serve=") {
                    opts.serve_addr = Some(if addr.is_empty() {
                        ":8080".to_string()
                    } else {
                        addr.to_string()
                    });
                } else {
                    return Startup::Fail(format!("Unknown argument '{other}'\n\n{USAGE}"));
                }
            }
        }
    }
    Startup::Run(opts)
}

/// `ssh` is invoked by name; a missing binary should be reported up front
/// rather than as an identical failure on every panel.
fn require_ssh() -> Result<(), String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let found = std::env::split_paths(&path).any(|dir| dir.join("ssh").is_file());
    if found {
        Ok(())
    } else {
        Err("Missing required commands: ssh".to_string())
    }
}

fn resolve_servers(
    opts: &CliOptions,
    config_path: &Path,
) -> Result<(Vec<config::Server>, Option<String>), String> {
    let local_server = config::Server {
        host: "localhost".into(),
        port: 0,
        user: String::new(),
        upgrade_cmd: None,
        custom_command: None,
        mcp: None,
    };
    let mut initial_theme: Option<String> = None;

    let mut servers = if opts.local_only {
        vec![local_server]
    } else if !opts.remote_hosts.is_empty() {
        let mut list: Vec<config::Server> = opts
            .remote_hosts
            .iter()
            .map(|h| config::Server {
                host: h.clone(),
                port: 22,
                user: String::new(),
                upgrade_cmd: None,
                custom_command: None,
                mcp: None,
            })
            .collect();
        if opts.local && !list.iter().any(ssh::is_local) {
            list.insert(0, local_server);
        }
        list
    } else {
        let mut cfg = config::load(config_path).map_err(|e| e.0)?;
        initial_theme = cfg.theme;
        if opts.local && !cfg.servers.iter().any(ssh::is_local) {
            cfg.servers.insert(0, local_server);
        }
        cfg.servers
    };

    let mut seen_local = false;
    servers.retain(|s| {
        if ssh::is_local(s) {
            if seen_local {
                false
            } else {
                seen_local = true;
                true
            }
        } else {
            true
        }
    });

    if servers.is_empty() {
        return Err("No servers to monitor.".to_string());
    }
    Ok((servers, initial_theme))
}

/// The `--serve` address: `:8080`, `8080` and `127.0.0.1:8080` all mean the
/// loopback port; anything else must parse as a socket address.
fn serve_socket_addr(addr: &str) -> Result<std::net::SocketAddr, String> {
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
fn serve_headless(
    addr: &str,
    token_file: Option<PathBuf>,
    config_path: &Path,
    servers: Vec<config::Server>,
    initial_theme: Option<String>,
) -> ExitCode {
    // A plain String from here on: `Option` said "no token", which downstream read
    // as "no authentication". Any failure -- the CSPRNG, the file, its mode --
    // REFUSES to serve rather than falling back to something weaker: a `--serve` on a
    // LAN with a guessable token is worse than no server.
    let token_path = token_file.unwrap_or_else(|| multitop::serve_token::default_path(config_path));
    let (token, created) = match multitop::serve_token::load_or_create(&token_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("[Error] {e} -- refusing to start --serve");
            return ExitCode::FAILURE;
        }
    };
    // Need to load full config for health thresholds
    let cfg = config::load(config_path).unwrap_or_else(|_| config::Config {
        servers: servers.clone(),
        theme: initial_theme,
        upgrade_history_lines: crate::config::DEFAULT_UPGRADE_HISTORY_LINES,
        history_lines_raised_from: None,
        banner_style: multitop::layout::BannerStyle::default(),
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
    let to_tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
    eprintln!(
        "{}",
        multitop::serve_token::startup_banner(addr, &token, &token_path, created, to_tty)
    );
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
        let live = std::sync::Arc::new(tokio::sync::RwLock::new(
            multitop::server::LiveState::default(),
        ));
        let app_state = multitop::server::AppState {
            live: live.clone(),
            token: token.clone(),
            servers: servers.clone(),
            config: cfg,
        };
        // Sort for collectors
        let sort = multitop_agent::SortBy::Cpu;
        multitop::server::spawn_collectors(servers, &live, sort);
        multitop::server::serve(addr, app_state).await
    });
    match code {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[Error] {e}");
            ExitCode::FAILURE
        }
    }
}
fn main() -> ExitCode {
    let opts = match parse_cli(std::env::args().skip(1)) {
        Startup::Run(opts) => opts,
        Startup::Agent(agent_args) => {
            multitop_agent::run_agent(agent_args);
            return ExitCode::SUCCESS;
        }
        Startup::Print(text) => {
            println!("{text}");
            return ExitCode::SUCCESS;
        }
        Startup::Fail(e) => {
            eprintln!("[Error] {e}");
            return ExitCode::FAILURE;
        }
    };

    let config_path = opts
        .config_path
        .clone()
        .unwrap_or_else(config::default_config_path);

    let (servers, initial_theme) = match resolve_servers(&opts, &config_path) {
        Ok(res) => res,
        Err(e) => {
            eprintln!("[Error] {e}");
            return ExitCode::FAILURE;
        }
    };

    let has_remote = servers.iter().any(|s| !ssh::is_local(s));
    if has_remote {
        if let Err(e) = require_ssh() {
            eprintln!("[Error] {e}");
            return ExitCode::FAILURE;
        }
        if !ssh::any_agent_embedded() {
            eprintln!(
                "[Error] This build contains no agent binaries.\n\
                 \x20       Run ./build.sh to cross-compile them, then rebuild."
            );
            return ExitCode::FAILURE;
        }
    }

    // --serve: headless HTTP companion reusing the MTOP pipeline.
    if let Some(addr) = opts.serve_addr.clone() {
        return serve_headless(
            &addr,
            opts.serve_token_file,
            &config_path,
            servers,
            initial_theme,
        );
    }

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

    let code = match runtime.block_on(run::run(servers, config_path, initial_theme)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[Error] {e}");
            ExitCode::FAILURE
        }
    };

    // Detached blocking workers (keychain lookups, biometric prompts) are
    // joined when the runtime drops, and a worker parked on a system dialog
    // would otherwise hold the process on quit. Bound that drain so quit is
    // quit; this call consumes the runtime and drops it with the bound in force.
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Startup {
        parse_cli(args.iter().map(std::string::ToString::to_string))
    }

    #[test]
    fn no_arguments_uses_default_options() -> Result<(), String> {
        let Startup::Run(opts) = cli(&[]) else {
            return Err("expected Run".into());
        };
        assert_eq!(opts.config_path, None);
        assert!(!opts.local);
        assert!(!opts.local_only);
        assert_eq!(opts.remote_hosts, [] as [std::string::String; 0]);
        Ok(())
    }

    #[test]
    fn config_flag_overrides_the_path() -> Result<(), String> {
        for flag in ["-c", "--config"] {
            let Startup::Run(opts) = cli(&[flag, "/tmp/x.toml"]) else {
                return Err(format!("expected Run for {flag}"));
            };
            assert_eq!(opts.config_path, Some(PathBuf::from("/tmp/x.toml")));
        }
        Ok(())
    }

    #[test]
    fn remote_flag_parses_hosts() -> Result<(), String> {
        let Startup::Run(opts) = cli(&["--remote", "10.0.0.1,10.0.0.2"]) else {
            return Err("expected Run for remote flag".into());
        };
        assert_eq!(opts.remote_hosts, vec!["10.0.0.1", "10.0.0.2"]);
        Ok(())
    }

    #[test]
    fn local_flags_parsed() -> Result<(), String> {
        let Startup::Run(opts1) = cli(&["--local"]) else {
            return Err("expected Run".into());
        };
        assert!(opts1.local && !opts1.local_only);

        let Startup::Run(opts2) = cli(&["--local-only"]) else {
            return Err("expected Run".into());
        };
        assert!(opts2.local_only);
        Ok(())
    }

    #[test]
    fn config_flag_without_value_is_an_error() {
        assert!(matches!(cli(&["--config"]), Startup::Fail(_)));
    }

    #[test]
    fn help_and_version_print_and_exit() -> Result<(), String> {
        assert!(matches!(cli(&["-h"]), Startup::Print(_)));
        assert!(matches!(cli(&["--help"]), Startup::Print(_)));
        let Startup::Print(t) = cli(&["--version"]) else {
            return Err("expected Print".into());
        };
        assert!(t.contains(env!("CARGO_PKG_VERSION")));
        Ok(())
    }

    #[test]
    fn unknown_argument_is_rejected_with_usage() -> Result<(), String> {
        let Startup::Fail(e) = cli(&["--nope"]) else {
            return Err("expected Fail".into());
        };
        assert!(e.contains("--nope"));
        assert!(e.contains("USAGE"));
        Ok(())
    }

    #[test]
    fn a_serve_token_on_the_command_line_is_refused() -> Result<(), String> {
        // RED before A1: `--serve-token <TOKEN>` was accepted, which put the credential
        // in argv, where every local user reads it with `ps`.
        let Startup::Fail(e) = cli(&["--serve", "--serve-token", "x".repeat(64).as_str()]) else {
            return Err("a token on argv was accepted".into());
        };
        assert!(e.contains("--serve-token-file"), "{e}");
        let Startup::Run(opts) = cli(&["--serve", "--serve-token-file", "/tmp/t"]) else {
            return Err("expected Run for --serve-token-file".into());
        };
        assert_eq!(opts.serve_token_file, Some(PathBuf::from("/tmp/t")));
        Ok(())
    }

    #[test]
    fn usage_documents_every_key() {
        for key in ["ESC", "f", "d", "s", "u"] {
            assert!(USAGE.contains(key), "usage is missing {key}");
        }
    }
}
