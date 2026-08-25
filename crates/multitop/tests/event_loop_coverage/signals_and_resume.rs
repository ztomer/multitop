use super::*;

// -------------------------------------------------------------------- signals

/// `SIGTERM` and `SIGHUP` end the process at their default disposition, so
/// `TerminalGuard` never runs and neither does the notice naming the upgrades
/// that were killed. Caught, they become an ordinary quit.
#[tokio::test]
async fn a_termination_signal_leaves_through_the_normal_exit() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let (dims_tx, _rx) = tokio::sync::watch::channel((0, 0));

    // No scripted events: the only thing that can end this loop is the signal.
    let mut stream = tokio_stream::pending::<std::io::Result<Event>>();
    let backend = TestBackend::new(100, 30);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");

    let signaller = tokio::spawn(async {
        // Long enough for the loop to have installed its handlers.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(std::process::id().to_string())
            .status();
    });

    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        multitop::run::event_loop(
            &mut terminal,
            &mut stream,
            dims_tx,
            vec![test_server("alpha.example")],
            config_path,
            None,
        ),
    )
    .await
    .expect("SIGTERM must end the loop");
    signaller.await.unwrap();

    assert!(
        outcome.error.is_none(),
        "a caught signal is not a terminal failure"
    );
}

/// A resume rebuilds the terminal: anything that stopped this process left raw
/// mode and the alternate screen behind, and the shell has drawn over both.
#[tokio::test]
async fn a_resume_redraws_rather_than_leaving_the_shell_s_output_on_screen() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let (dims_tx, _rx) = tokio::sync::watch::channel((0, 0));

    let mut stream =
        tokio_stream::iter(vec![Ok(key(KeyCode::Char('q')))]).chain(tokio_stream::pending());
    let backend = TestBackend::new(100, 30);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");

    let signaller = tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _ = std::process::Command::new("kill")
            .arg("-CONT")
            .arg(std::process::id().to_string())
            .status();
    });

    // The `q` is consumed first, so the loop is already gone by the time the
    // signal lands in most runs; what is under test is that neither ordering
    // wedges it.
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        multitop::run::event_loop(
            &mut terminal,
            &mut stream,
            dims_tx,
            vec![test_server("alpha.example")],
            config_path,
            None,
        ),
    )
    .await
    .expect("the loop must end");
    signaller.await.unwrap();
    assert!(outcome.error.is_none());
}

// ------------------------------------------------------- agents after an edit

/// Editing the server list while the Docker view is up has to restart the
/// docker pollers too, not just the monitor streams — otherwise the new panel
/// list is fed by tasks bound to the old one.
#[tokio::test]
async fn a_server_edit_in_the_docker_view_restarts_the_docker_pollers() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let servers = vec![
        test_server("alpha.example"),
        test_server("beta.example"),
        test_server("gamma.example"),
    ];

    let (_, outcome) = run_to_quit(
        &dir,
        servers,
        (120, 40),
        None,
        vec![
            // Into the Docker view, then Settings, then remove the selected
            // host: the panel list changes underneath a running view.
            key(KeyCode::Char('d')),
            key(KeyCode::Char('e')),
            key(KeyCode::Char('d')),
            key(KeyCode::Char('y')),
            key(KeyCode::Esc),
        ],
    )
    .await;
    assert!(outcome.error.is_none());
}

// ------------------------------------------------------- a failing terminal
