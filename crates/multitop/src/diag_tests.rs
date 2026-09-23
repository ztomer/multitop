use super::*;
use crate::config::Server;
use crate::panel::UpgradeState;

fn server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "u".to_string(),
        upgrade_cmd: Some("true".to_string()),
        custom_command: None,
        mcp: None,
    }
}

#[test]
fn phases_are_stable_and_names_round_trip() {
    let names: Vec<(&str, u8)> = (0..6).map(|c| (Phase::from_code(c).name(), c)).collect();
    assert_eq!(names[0], ("Setup", 0));
    assert_eq!(names[1], ("Idle", 1));
    assert_eq!(names[2], ("Drawing", 2));
    assert_eq!(names[3], ("HandlingKey", 3));
    assert_eq!(names[4], ("Applying", 4));
    assert_eq!(names[5], ("Resizing", 5));
}

#[test]
fn a_request_is_consumed_exactly_once() {
    let d = Diag::new(Diag::default_dir());
    d.note_signal(signal_hook::consts::SIGUSR2);
    let (seq, name) = d.take_request().expect("the request must be there");
    assert_eq!(name, "USR2");
    assert_eq!(seq, 0);
    assert!(d.take_request().is_none(), "the request was not consumed");
}

#[test]
fn the_snapshot_renders_and_carries_no_secret_fields() {
    let s = Snapshot {
        mode: "Running".into(),
        filter: "/db".into(),
        selected: 1,
        in_flight: true,
        quit: QuitFlags::default(),
        vault_unlocked: true,
        active_confirm: None,
        tasks: Liveness {
            monitors_alive: 2,
            upgrades_alive: 1,
            upgrades_done: 0,
        },
        panels: vec![
            PanelDigest {
                host: "db-01".into(),
                mode: "Monitor".into(),
                state: "STARTED".into(),
                gen: 7,
                upgrade_gen: 7,
                view_len: 40,
                ring_len: 120,
                scroll: 0,
            },
            PanelDigest {
                host: "web-01".into(),
                mode: "Upgrade".into(),
                state: "DONE".into(),
                gen: 3,
                upgrade_gen: 2,
                view_len: 10,
                ring_len: 300,
                scroll: 41,
            },
        ],
        last_update: Some(1_700_000_000),
        upgrade_started_at: None,
    };
    let text = render_snapshot(&s);
    assert!(text.contains("db-01"), "{text}");
    assert!(text.contains("STARTED"), "{text}");
    assert!(text.contains("ring 300"), "{text}");
    assert!(text.contains("scroll 41"), "{text}");
    for forbidden in ["password", "sudo", "vault_password", "hunter2"] {
        assert!(
            !text.to_lowercase().contains(forbidden),
            "the dump leaked a {forbidden} marker: {text}"
        );
    }
}

#[test]
fn snapshot_app_reflects_live_state_and_liveness() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let mut app = App::new(vec![server("db-01"), server("web-01")]);
    app.selected_panel = 1;
    app.panels[0].upgrade_state = UpgradeState::STARTED;
    app.panels[0].mode = crate::panel::Mode::Upgrade;
    let mut tasks = Tasks::new(2);
    tasks.set_upgrade(0, tokio::spawn(std::future::pending::<()>()));
    tasks.set_aux(1, tokio::spawn(std::future::pending::<()>()));

    let snap = snapshot_app(&app, &tasks);
    assert_eq!(snap.selected, 1);
    assert!(snap.in_flight);
    assert_eq!(snap.panels.len(), 2);
    assert_eq!(snap.panels[0].state, "STARTED");
    assert_eq!(snap.panels[0].mode, "Upgrade");
    // A live upgrade task (alive) and a live view task; the monitor list is
    // only reachable from `run`, so its branch is covered by construction.
    assert_eq!(snap.tasks.monitors_alive, 0);
    assert_eq!(snap.tasks.upgrades_alive, 1);
    assert_eq!(snap.tasks.upgrades_done, 0);
}

#[cfg(unix)]
#[test]
fn both_tiers_land_and_are_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let d = Diag::new(dir.path().to_path_buf());
    d.set_phase(Phase::HandlingKey);
    d.bump_key();
    d.bump_applied();
    d.bump_drained(4);
    d.note_signal(signal_hook::consts::SIGUSR2);
    let (seq, name) = d.take_request().unwrap();

    let sig_path = d.write_signal_tier(seq, name).expect("signal tier");
    let snap = Snapshot {
        mode: "Running".into(),
        filter: String::new(),
        selected: 0,
        in_flight: true,
        panels: vec![PanelDigest {
            host: "db-01".into(),
            mode: "Upgrade".into(),
            state: "STARTED".into(),
            gen: 2,
            upgrade_gen: 2,
            view_len: 9,
            ring_len: 5,
            scroll: 3,
        }],
        quit: QuitFlags::default(),
        vault_unlocked: false,
        last_update: None,
        upgrade_started_at: None,
        active_confirm: None,
        tasks: Liveness {
            monitors_alive: 1,
            upgrades_alive: 1,
            upgrades_done: 0,
        },
    };
    let state_path = d.write_state_tier(seq, name, &snap).expect("state tier");

    for path in [&sig_path, &state_path] {
        let body = std::fs::read_to_string(path).unwrap();
        assert!(body.contains("HandlingKey"), "{}: {body}", path.display());
        assert!(path.exists());
    }
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [&sig_path, &state_path] {
            let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{} is not private", path.display());
        }
    }
    let sig_body = std::fs::read_to_string(&sig_path).unwrap();
    assert!(sig_body.contains("snapshot: none yet"), "{sig_body}");
    let state_body = std::fs::read_to_string(&state_path).unwrap();
    assert!(state_body.contains("panel 0: db-01"), "{state_body}");
}

/// Proves the whole round trip against real signals: the failure mode this
/// must catch is a handler that was never installed, in which case `kill`
/// does nothing and `signals_seen` stays zero until the timeout.
#[cfg(unix)]
#[test]
fn a_real_sigusr2_writes_a_signal_tier_dump() {
    let dir = tempfile::tempdir().unwrap();
    let d = Diag::new(dir.path().to_path_buf());
    install(&d);
    // Each wait gets its own full deadline: `ready` can take most of the
    // window on a loaded machine (the handler thread is the one being
    // diagnosed), and a deadline shared with the signal wait would leave
    // the signal wait nothing to poll -- a flake that blames the handler
    // for the harness it ran under.
    let ready_deadline = Instant::now() + std::time::Duration::from_secs(5);
    while !d.ready() && Instant::now() < ready_deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(d.ready(), "the diagnostic thread never became ready");

    std::process::Command::new("kill")
        .arg("-USR2")
        .arg(std::process::id().to_string())
        .status()
        .expect("send SIGUSR2");

    // `signals_seen` bumps before the dump is written, so counting it is
    // not enough to read the file: poll for the finished artifact itself,
    // or a read under load catches the file between `create` and `write`.
    let dump_deadline = Instant::now() + std::time::Duration::from_secs(5);
    let mut last_body = String::new();
    loop {
        let found: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with("-signal.txt"))
            .collect();
        if found.len() == 1 {
            let path = dir.path().join(&found[0]);
            last_body = std::fs::read_to_string(&path).unwrap_or_default();
        }
        if last_body.contains("trigger: USR2") {
            break;
        }
        assert!(
            Instant::now() < dump_deadline,
            "no complete signal-tier dump after SIGUSR2: files={found:?} body={last_body:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
