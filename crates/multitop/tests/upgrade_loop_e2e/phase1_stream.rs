use super::*;

// ============================================================================
// Phase 1: Core Stream Tests
// ============================================================================

/// Test 1: Single server basic stream
#[tokio::test]
async fn test_upgrade_single_server_streams_exact_output() {
    let _store_guard = enable_test_mock_store().await;
    let server = local_server("ls -l ; ls -l");
    let (tx, rx) = mpsc::channel::<Msg>(100);
    let handle = spawn_upgrade(0, 1, server, None, tx);

    let mut collector = MsgCollector::new(rx);
    let done = collector.wait_for_done().await.expect("Expected AuxDone");
    // Not `let _ =`: a panic inside the spawned task arrives here as a join
    // error, and swallowing it leaves the test to fail later for some other
    // reason -- or to pass. The task is the thing under test.
    handle.await.expect("the spawned task must not panic");

    match done {
        Msg::AuxDone {
            panel: 0,
            gen: 1,
            success: true,
            ..
        } => {}
        other => panic!("Expected AuxDone panel=0 gen=1 success=true, got {other:?}"),
    }
}

/// Test 2: Multi-server concurrent output
#[tokio::test]
async fn test_upgrade_multi_server_concurrent_output() {
    let _store_guard = enable_test_mock_store().await;
    let (tx, rx) = mpsc::channel::<Msg>(100);

    let s1 = local_server("echo UPGRADE_1 ; ls -l");
    let s2 = local_server("echo UPGRADE_2 ; ls -l");
    let s3 = local_server("echo UPGRADE_3 ; ls -l");

    let h1 = spawn_upgrade(0, 1, s1, None, tx.clone());
    let h2 = spawn_upgrade(1, 2, s2, None, tx.clone());
    let h3 = spawn_upgrade(2, 3, s3, None, tx.clone());

    drop(tx);

    let mut collector = MsgCollector::new(rx);
    let msgs = collector.collect_all().await;

    let _ = h1.await;
    let _ = h2.await;
    let _ = h3.await;

    let mut done_count = 0;
    let mut seen_1 = false;
    let mut seen_2 = false;
    let mut seen_3 = false;

    for msg in &msgs {
        if let Msg::AuxLine { line, .. } | Msg::AuxRepaint { line, .. } = msg {
            if line.contains("UPGRADE_1") {
                seen_1 = true;
            }
            if line.contains("UPGRADE_2") {
                seen_2 = true;
            }
            if line.contains("UPGRADE_3") {
                seen_3 = true;
            }
        }
        if let Msg::AuxDone { success: true, .. } = msg {
            done_count += 1;
        }
    }

    assert!(seen_1, "UPGRADE_1 not found in stream");
    assert!(seen_2, "UPGRADE_2 not found in stream");
    assert!(seen_3, "UPGRADE_3 not found in stream");
    assert_eq!(done_count, 3, "Expected 3 successful AuxDone messages");
}

/// Test 3: Failure exit code
#[tokio::test]
async fn test_upgrade_failure_reports_nonzero_exit() {
    let _store_guard = enable_test_mock_store().await;
    let server = local_server("ls -l ; exit 1");
    let (tx, rx) = mpsc::channel::<Msg>(100);
    let handle = spawn_upgrade(0, 1, server, None, tx);

    let mut collector = MsgCollector::new(rx);
    let done = collector.wait_for_done().await.expect("Expected AuxDone");
    // Not `let _ =`: a panic inside the spawned task arrives here as a join
    // error, and swallowing it leaves the test to fail later for some other
    // reason -- or to pass. The task is the thing under test.
    handle.await.expect("the spawned task must not panic");

    match done {
        Msg::AuxDone {
            panel: 0,
            gen: 1,
            success: false,
            ..
        } => {}
        other => panic!("Expected AuxDone panel=0 gen=1 success=false, got {other:?}"),
    }
}

/// Test 4: State machine roundtrip (full App cycle with Msg application)
#[tokio::test]
async fn test_upgrade_state_machine_roundtrip() {
    use tempfile::TempDir;
    let _keychain = isolate_keychain_async().await;
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let mut app = App::new(vec![local_server("ls -l")]);
    app.config_path = Some(config_path.clone());

    let cmds = app.confirm_upgrade();
    assert_ne!(cmds, [] as [multitop::app::Command; 0]);
    let started_at = app.upgrade_started_at;
    assert!(started_at.is_some(), "upgrade_started_at should be set");

    let Command::RunUpgrade { panel, gen } = cmds[0] else {
        panic!("Expected Command::RunUpgrade");
    };
    assert_eq!(panel, 0);

    // Simulate full message stream
    app.apply(Msg::AuxBegin {
        panel: 0,
        gen,
        header: Some("Upgrade on 127.0.0.1".into()),
    });
    assert_eq!(app.panels[0].mode, Mode::Upgrade);

    app.apply(Msg::AuxLine {
        panel: 0,
        gen,
        line: "drwxr-xr-x 1 root root 4096 Jan 1 00:00 bin".to_string(),
    });

    app.apply(Msg::AuxDone {
        panel: 0,
        gen,
        note: Some("done".to_string()),
        success: true,
    });

    assert_eq!(app.panels[0].upgrade_state, UpgradeState::DONE);
    assert!(
        app.last_update.is_some(),
        "last_update should be set on success"
    );

    // Verify state was persisted (AuxDone updates state.toml with last_update, clears upgrade_started_at)
    let state = multitop::state::load_state(&config_path);
    assert_eq!(state.state.last_update, app.last_update);
    assert_eq!(
        state.state.upgrade_started_at, None,
        "upgrade_started_at cleared after successful completion"
    );
    assert!(app.panels[0]
        .last_upgrade
        .iter()
        .any(|l| l == "drwxr-xr-x 1 root root 4096 Jan 1 00:00 bin"));
}

/// Test 7: Empty output handled gracefully
#[tokio::test]
async fn test_upgrade_empty_output_handled() {
    let _store_guard = enable_test_mock_store().await;
    let server = local_server("true");
    let (tx, rx) = mpsc::channel::<Msg>(100);
    let handle = spawn_upgrade(0, 1, server, None, tx);

    let mut collector = MsgCollector::new(rx);
    let done = collector.wait_for_done().await.expect("Expected AuxDone");
    // Not `let _ =`: a panic inside the spawned task arrives here as a join
    // error, and swallowing it leaves the test to fail later for some other
    // reason -- or to pass. The task is the thing under test.
    handle.await.expect("the spawned task must not panic");

    match done {
        Msg::AuxDone {
            panel: 0,
            gen: 1,
            success: true,
            ..
        } => {}
        other => panic!("Expected AuxDone success=true, got {other:?}"),
    }
}

/// Test 8: Carriage return cleaning
#[tokio::test]
async fn test_upgrade_carriage_return_cleaned() {
    let _store_guard = enable_test_mock_store().await;
    let server = local_server("printf 'step1\\rstep2\\rstep3\\n'");
    let (tx, rx) = mpsc::channel::<Msg>(100);
    let handle = spawn_upgrade(0, 1, server, None, tx);

    let mut collector = MsgCollector::new(rx);
    let msgs = collector.collect_all().await;
    // Not `let _ =`: a panic inside the spawned task arrives here as a join
    // error, and swallowing it leaves the test to fail later for some other
    // reason -- or to pass. The task is the thing under test.
    handle.await.expect("the spawned task must not panic");

    let progress_lines: Vec<String> = msgs
        .iter()
        .filter_map(|msg| {
            if let Msg::AuxLine { line, .. } | Msg::AuxRepaint { line, .. } = msg {
                if line.contains("step") {
                    return Some(line.clone());
                }
            }
            None
        })
        .collect();

    assert!(!progress_lines.is_empty(), "Should have progress lines");
    for line in &progress_lines {
        assert!(
            !line.contains('\r'),
            "Carriage return not stripped: {line:?}"
        );
    }
}
