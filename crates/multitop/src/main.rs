//! multitop — watch several servers at once in one terminal.

use multitop::{config, run, ssh};

use std::path::PathBuf;
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
}

enum Startup {
    Run(CliOptions),
    Agent(Vec<String>),
    Print(String),
    Fail(String),
}

fn parse_cli<I: IntoIterator<Item = String>>(argv: I) -> Startup {
    let mut args = argv.into_iter();
    let mut opts = CliOptions::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Startup::Print(USAGE.to_string()),
            "-V" | "--version" => {
                return Startup::Print(format!("multitop {}", env!("CARGO_PKG_VERSION")))
            }
            "-c" | "--config" => match args.next() {
                Some(p) => opts.config_path = Some(PathBuf::from(p)),
                None => return Startup::Fail("--config requires a path".into()),
            },
            "-r" | "--remote" => match args.next() {
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
            "--agent" => {
                let rest: Vec<String> = args.collect();
                return Startup::Agent(rest);
            }
            other => return Startup::Fail(format!("Unknown argument '{other}'\n\n{USAGE}")),
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

    let local_server = config::Server {
        host: "localhost".into(),
        port: 0,
        user: String::new(),
        upgrade_cmd: None,
    };

    let config_path = opts
        .config_path
        .clone()
        .unwrap_or_else(config::default_config_path);
    let mut initial_theme: Option<String> = None;

    let mut servers = if opts.local_only {
        vec![local_server]
    } else if !opts.remote_hosts.is_empty() {
        let mut list: Vec<config::Server> = opts
            .remote_hosts
            .into_iter()
            .map(|h| config::Server {
                host: h,
                port: 22,
                user: String::new(),
                upgrade_cmd: None,
            })
            .collect();
        if opts.local && !list.iter().any(ssh::is_local) {
            list.insert(0, local_server);
        }
        list
    } else {
        let mut cfg = match config::load(&config_path).map_err(|e| e.0) {
            Ok(c) => c,
            Err(e) => {
                if opts.local {
                    config::Config {
                        servers: Vec::new(),
                        theme: None,
                        upgrade_history_lines: config::DEFAULT_UPGRADE_HISTORY_LINES,
                        show_sparklines: false,
                    }
                } else {
                    eprintln!("[Error] {e}");
                    return ExitCode::FAILURE;
                }
            }
        };
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
        eprintln!("[Error] No servers to monitor.");
        return ExitCode::FAILURE;
    }

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

    match runtime.block_on(run::run(servers, config_path, initial_theme)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[Error] {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Startup {
        parse_cli(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_arguments_uses_default_options() {
        match cli(&[]) {
            Startup::Run(opts) => {
                assert_eq!(opts.config_path, None);
                assert!(!opts.local);
                assert!(!opts.local_only);
                assert!(opts.remote_hosts.is_empty());
            }
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn config_flag_overrides_the_path() {
        for flag in ["-c", "--config"] {
            match cli(&[flag, "/tmp/x.toml"]) {
                Startup::Run(opts) => {
                    assert_eq!(opts.config_path, Some(PathBuf::from("/tmp/x.toml")))
                }
                _ => panic!("expected Run for {flag}"),
            }
        }
    }

    #[test]
    fn remote_flag_parses_hosts() {
        match cli(&["--remote", "10.0.0.1,10.0.0.2"]) {
            Startup::Run(opts) => {
                assert_eq!(opts.remote_hosts, vec!["10.0.0.1", "10.0.0.2"]);
            }
            _ => panic!("expected Run for remote flag"),
        }
    }

    #[test]
    fn local_flags_parsed() {
        match cli(&["--local"]) {
            Startup::Run(opts) => assert!(opts.local && !opts.local_only),
            _ => panic!("expected Run"),
        }
        match cli(&["--local-only"]) {
            Startup::Run(opts) => assert!(opts.local_only),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn config_flag_without_value_is_an_error() {
        assert!(matches!(cli(&["--config"]), Startup::Fail(_)));
    }

    #[test]
    fn help_and_version_print_and_exit() {
        assert!(matches!(cli(&["-h"]), Startup::Print(_)));
        assert!(matches!(cli(&["--help"]), Startup::Print(_)));
        match cli(&["--version"]) {
            Startup::Print(t) => assert!(t.contains(env!("CARGO_PKG_VERSION"))),
            _ => panic!("expected Print"),
        }
    }

    #[test]
    fn unknown_argument_is_rejected_with_usage() {
        match cli(&["--nope"]) {
            Startup::Fail(e) => {
                assert!(e.contains("--nope"));
                assert!(e.contains("USAGE"));
            }
            _ => panic!("expected Fail"),
        }
    }

    #[test]
    fn usage_documents_every_key() {
        for key in ["ESC", "f", "d", "s", "u"] {
            assert!(USAGE.contains(key), "usage is missing {key}");
        }
    }
}
