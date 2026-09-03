//! multitop — watch several servers at once in one terminal.

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
        --serve-token <TOKEN>  Bearer token for --serve (auto-generated if omitted)
    -h, --help             Print this help
    -V, --version          Print version

KEYS:
    ESC / q    Quit
    f          Toggle Fastfetch system info view on every panel
    d          Toggle the Docker view on every panel
    s          Back to live stats
    u          Run each server's configured upgrade_cmd
";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CliOptions {
    pub config_path: Option<PathBuf>,
    pub local: bool,
    pub local_only: bool,
    pub remote_hosts: Vec<String>,
    pub serve_addr: Option<String>,
    pub serve_token: Option<String>,
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
            "--serve-token" => match iter.next() {
                Some(t) => opts.serve_token = Some(t),
                None => return Startup::Fail("--serve-token requires a value".into()),
            },
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

#[allow(clippy::too_many_lines)]
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
        let token = opts.serve_token.or_else(|| {
            // Auto-generate a token if not supplied, but only when binding to loopback
            // or when explicitly requested. For now, generate if not supplied and print it.
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            )
            .hash(&mut hasher);
            let t = format!("{:016x}", hasher.finish());
            Some(t)
        });
        // Need to load full config for health thresholds
        let cfg = config::load(&config_path).unwrap_or_else(|_| config::Config {
            servers: servers.clone(),
            theme: initial_theme.clone(),
            upgrade_history_lines: crate::config::DEFAULT_UPGRADE_HISTORY_LINES,
            history_lines_raised_from: None,
            banner_style: multitop::layout::BannerStyle::default(),
            plaintext_passwords: vec![],
            alert_cpu: None,
            alert_mem: None,
            alert_disk: None,
            alerts: vec![],
        });
        let addr: std::net::SocketAddr = if let Ok(a) = addr.parse() {
            a
        } else {
            // Allow :8080, 8080, 127.0.0.1:8080
            let a = if addr.starts_with(':') {
                format!("127.0.0.1{addr}")
            } else if addr.chars().all(|c| c.is_ascii_digit()) {
                format!("127.0.0.1:{addr}")
            } else {
                addr.clone()
            };
            match a.parse() {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("[Error] Invalid --serve address '{addr}': {e}");
                    return ExitCode::FAILURE;
                }
            }
        };
        if let Some(ref t) = token {
            eprintln!("multitop --serve on http://{addr} token={t}");
            eprintln!("  curl -H \"Authorization: Bearer {t}\" http://{addr}/api/hosts");
        } else {
            eprintln!("multitop --serve on http://{addr} (no token)");
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
        return match code {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("[Error] {e}");
                ExitCode::FAILURE
            }
        };
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
    fn usage_documents_every_key() {
        for key in ["ESC", "f", "d", "s", "u"] {
            assert!(USAGE.contains(key), "usage is missing {key}");
        }
    }
}
