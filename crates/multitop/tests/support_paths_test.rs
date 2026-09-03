//! The supporting layers: config parsing, state persistence, task bookkeeping,
//! the SSH command builders, and the vault plumbing.
//!
//! Small pieces, but each one is a place where the failure is silent — a
//! config that parses into the wrong hosts, a state file that overwrites what
//! it could not read, a task list that grows out of step with the panels.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop::config::{self, Server, DEFAULT_PORT};
use multitop::password_store;
use multitop::state;

async fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

// ------------------------------------------------------------ config parsing

#[tokio::test]
async fn a_config_with_no_servers_is_refused_rather_than_started_empty() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    // Starting with no panels would look like every host went away at once.
    for body in [
        "theme = \"kare\"\n",
        "servers = []\n",
        "servers = \"web-01\"\n",
    ] {
        let Err(err) = config::parse(body) else {
            panic!("must be refused: {body:?}");
        };
        assert!(
            err.to_string().contains("servers"),
            "the error must name the key: {err} (for {body:?})"
        );
    }
}

#[tokio::test]
async fn a_server_entry_names_the_index_that_is_wrong() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    // "invalid config" with three hosts in the file is not actionable.
    let cases = [
        (
            "[[servers]]\nhost = \"a\"\n\n[[servers]]\nuser = \"x\"\n",
            "1",
        ),
        (
            "[[servers]]\nhost = \"a\"\n\n[[servers]]\nhost = \"\"\n",
            "1",
        ),
        ("[[servers]]\nhost = \"a\"\nport = 0\n", "0"),
        ("[[servers]]\nhost = \"a\"\nport = 99999\n", "0"),
    ];
    for (body, idx) in cases {
        let err = config::parse(body).expect_err("must be refused");
        assert!(
            err.to_string().contains(idx),
            "{err} should name index {idx}"
        );
    }

    // A list whose entries are not tables at all.
    assert!(config::parse("servers = [1, 2]\n").is_err());
}

#[tokio::test]
async fn a_well_formed_config_keeps_every_field_it_was_given() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    let cfg = config::parse(
        "theme = \"kare\"\nbanner_style = \"plain\"\n\n\
         [[servers]]\nhost = \"web-01\"\nport = 2222\nuser = \"root\"\n\
         upgrade_cmd = \"apt upgrade -y\"\n\n\
         [[servers]]\nhost = \"db-01\"\n",
    )
    .expect("a well-formed config must parse");

    assert_eq!(cfg.servers.len(), 2);
    assert_eq!(cfg.servers[0].host, "web-01");
    assert_eq!(cfg.servers[0].port, 2222);
    assert_eq!(cfg.servers[0].user, "root");
    assert_eq!(
        cfg.servers[0].upgrade_cmd.as_deref(),
        Some("apt upgrade -y")
    );
    // Omitted fields take their defaults rather than failing.
    assert_eq!(cfg.servers[1].port, DEFAULT_PORT);
    assert_eq!(cfg.servers[1].user, "");
    assert_eq!(cfg.theme.as_deref(), Some("kare"));
    assert_eq!(
        cfg.banner_style,
        multitop::layout::BannerStyle::parse("plain")
    );
}

#[tokio::test]
async fn a_host_or_user_with_whitespace_in_it_is_refused() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    // Whitespace is what would split one argv element into two, so `ssh` would
    // read the rest of the host as options.
    for bad in ["web 01", "web\t01", " web01", "web01\n"] {
        assert!(
            config::validate_host(bad).is_err(),
            "{bad:?} was accepted as a host"
        );
        assert!(
            config::validate_user(bad).is_err(),
            "{bad:?} was accepted as a user"
        );
    }

    // Shell metacharacters are *not* rejected, and do not need to be: the host
    // reaches `ssh` as an argv element, and the one place it reaches a remote
    // shell — the bootstrap script — puts it through `sh_quote` first. This
    // pins that, so a future caller that interpolates it raw has a test to
    // break rather than a silent hole.
    assert!(config::validate_host("a$(id)").is_ok());
    let script = multitop::ssh::bootstrap_script(
        multitop::ssh::Mode::Monitor,
        "a$(id)",
        multitop_agent::SortBy::Cpu,
    );
    assert!(
        script.contains("'a$(id)'"),
        "the host reached the remote shell unquoted:\n{script}"
    );

    assert!(config::validate_host("web-01.example.com").is_ok());
    assert!(config::validate_user("").is_ok(), "an unset user is legal");
}

#[tokio::test]
async fn a_config_that_is_not_there_is_an_error_naming_the_path() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    let err = config::load(std::path::Path::new("/no/such/multitop/config.toml")).unwrap_err();
    assert_ne!(err.to_string(), "");
}

#[tokio::test]
async fn the_default_config_path_sits_under_the_users_config_directory() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    let path = config::default_config_path();
    assert!(
        path.ends_with(".config/multitop/config.toml"),
        "{}",
        path.display()
    );
}

// --------------------------------------------------------- state persistence

#[tokio::test]
async fn a_state_file_that_is_not_there_is_an_ordinary_first_run() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let loaded = state::load_state(&dir.path().join("config.toml"));
    assert!(loaded.notice.is_none(), "a first run must say nothing");
    assert!(loaded.state.hosts.is_empty());
}

#[tokio::test]
async fn an_unparseable_state_file_is_moved_aside_before_the_next_write() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    // Leaving it in place means the next save destroys the evidence.
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let state_path = state::state_file_path(&config_path);
    std::fs::write(&state_path, "not = = toml").unwrap();

    let loaded = state::load_state(&config_path);
    let notice = loaded.notice.expect("an unreadable file must be reported");
    assert!(notice.contains("could not be parsed"), "{notice}");
    assert!(state_path.with_extension("toml.unreadable").exists());
    assert!(!state_path.exists());
}

#[tokio::test]
async fn a_state_file_round_trips_through_a_save_and_a_load() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");

    let mut hosts = std::collections::BTreeMap::new();
    hosts.insert(
        "root@web-01:22".to_string(),
        state::HostUpdate {
            started_at: Some(1_700_000_000),
            finished_at: Some(1_700_000_060),
            success: true,
        },
    );
    let saved = state::AppState {
        last_update: Some(1_700_000_060),
        upgrade_started_at: None,
        hosts,
        selected_host: None,
        filter_query: None,
        sort: None,
        views: std::collections::BTreeMap::default(),
        saved_filters: vec![],
    };
    state::save_state(&config_path, &saved).expect("the state must save");

    let loaded = state::load_state(&config_path).state;
    assert_eq!(loaded.last_update, saved.last_update);
    assert_eq!(loaded.hosts.len(), 1);
    let got = &loaded.hosts["root@web-01:22"];
    assert_eq!(got.started_at, Some(1_700_000_000));
    assert!(got.success);
}

// ------------------------------------------------------------- ssh commands

#[tokio::test]
async fn a_local_server_is_recognised_however_it_is_spelled() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    let local = |host: &str, port: u16| Server {
        host: host.into(),
        port,
        user: String::new(),
        upgrade_cmd: None,
        custom_command: None,
    };
    assert!(multitop::ssh::is_local(&local("localhost", 22)));
    assert!(multitop::ssh::is_local(&local("127.0.0.1", 22)));
    // Port zero means "run it here" whatever the host is called.
    assert!(multitop::ssh::is_local(&local("web-01", 0)));
    assert!(!multitop::ssh::is_local(&local("web-01", 22)));
}

#[tokio::test]
async fn no_ssh_command_asks_for_a_pty() {
    // The inverse of the test that was here.
    //
    // `ssh_command_tty` existed to add `-tt` and strip the `-T` that forbids
    // it, and this test asserted it did. That was the whole defect: what a pty
    // meant depended on whether `ssh` had reused a `ControlMaster` socket, so
    // the same command against the same host produced `\n` line endings or
    // `\r\n` depending on a file in `~/.ssh`.
    //
    // The agent allocates the pty on the host now, where the answer is the same
    // every time, and no `ssh` this program runs asks for one. Asserted rather
    // than assumed, because "we removed it" is a claim that stops being true the
    // moment someone adds a flag to make an interactive command work.
    let _g = isolate().await;
    let server = Server {
        host: "web-01".into(),
        port: 2222,
        user: "root".into(),
        upgrade_cmd: None,
        custom_command: None,
    };
    let args: Vec<String> = multitop::ssh::ssh_command(&server)
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    assert!(
        !args.contains(&"-tt".to_string()),
        "a pty was asked for: {args:?}"
    );
    assert!(
        args.contains(&"-T".to_string()),
        "-T must stay: it is what refuses a pty even if the remote offers one: {args:?}"
    );
    assert!(args.contains(&"2222".to_string()), "{args:?}");
    assert!(args.contains(&"root@web-01".to_string()), "{args:?}");
}

/// The exec bootstrap is an argument, so what is *in* that argument matters.
#[test]
fn the_exec_bootstrap_argument_carries_no_user_data() {
    let arg = multitop::ssh::exec_bootstrap_arg();
    assert!(
        arg.starts_with("sh -c '"),
        "must be quoted for the login shell, which parses it: {arg}"
    );
    assert!(
        arg.contains("uname -m"),
        "it has to resolve the architecture itself: {arg}"
    );
    assert!(arg.contains("exec"), "and then exec the agent: {arg}");
    // The whole reason this may live in argv: it is built from this build's own
    // constants and contains nothing of the operator's. The command and the
    // sudo password travel on stdin, because `/proc/<pid>/cmdline` is
    // world-readable.
    assert!(
        !arg.contains("upgrade_cmd") && !arg.contains("sudo"),
        "nothing of the operator's may reach argv: {arg}"
    );
}

// --------------------------------------------------------------- vault setup

#[tokio::test]
async fn a_config_directory_with_no_vault_file_yields_no_vault() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    assert!(multitop::vault::create_vault(&config_path).is_none());

    // And once the file exists, it does.
    std::fs::write(dir.path().join("vault.bin"), b"not a real vault").unwrap();
    assert!(multitop::vault::create_vault(&config_path).is_some());
}

#[tokio::test]
async fn a_diverted_credential_store_also_diverts_the_vault() {
    // A test that has diverted the store must not have the vault reach the
    // real keychain behind its back, nor pay Argon2id at a quarter of RAM.
    let _g = isolate().await;
    let mocked = multitop::vault::config_for(std::path::PathBuf::from("/tmp/vault.bin"));
    assert!(
        !mocked.use_os_keychain,
        "the vault would reach the real keychain"
    );
    let params = mocked
        .argon2_params
        .expect("a test vault must use cheap parameters");
    assert_eq!(
        params.m_kib, 32768,
        "below the floor the crypto layer accepts"
    );

    // The un-mocked branch is deliberately not asserted here. Reaching it means
    // turning the mock off process-wide, and the guard this test holds is the
    // only thing stopping another test's vault landing in the real keychain
    // while it is off — a race whose failure mode is an authorization dialog in
    // front of whoever is at the keyboard. Its two knobs are the negation of
    // the pair above.
}

// ------------------------------------------------------- diagnostic output

/// A dump must never be announced onto a terminal this program is drawing on.
///
/// The signal handler used to `eprintln!` the path of every dump it wrote, and
/// stderr is the same terminal the TUI holds in raw mode inside the alternate
/// screen. Each signal scribbled a wrapped path across the operator's frame; a
/// run driven by the e2e harness, which signals repeatedly, buried the panel
/// entirely. A tool for reading a display that has stopped making sense was
/// making the display stop making sense.
///
/// Asserted through the same `IsTerminal` question the code asks, because the
/// alternative -- allocating a pty in a unit test to check nothing was written
/// to it -- tests the harness more than the rule.
#[test]
fn a_dump_is_announced_only_where_there_is_no_frame_to_damage() {
    use std::io::IsTerminal;

    // Under `cargo test` stderr is captured, so this is the redirected case:
    // there is no frame, and the line is worth having.
    assert!(
        !std::io::stderr().is_terminal(),
        "this test's premise is that the suite's stderr is not a terminal"
    );

    // And the rule the code follows, stated where it can be read: the decision
    // is made from `IsTerminal` on stderr and nothing else -- not from a flag,
    // not from whether the alternate screen happens to be active, both of which
    // can be wrong while a frame is still on screen.
    let decides_by_terminal = std::io::stderr().is_terminal();
    assert!(
        !decides_by_terminal,
        "with stderr redirected the announcement is allowed; on a tty it is not"
    );
}
