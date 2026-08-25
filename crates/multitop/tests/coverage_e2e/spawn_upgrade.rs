use super::*;

// ===========================================================================
// tasks.rs — the spawn_upgrade streaming paths (integration via local shell)
// ===========================================================================

#[tokio::test]
async fn spawn_upgrade_streams_output_for_local_command() {
    let _g = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    let server = Server {
        host: "127.0.0.1".into(),
        port: 0,
        user: "testuser".into(),
        upgrade_cmd: Some("echo hello-from-upgrade".into()),
    };
    let (tx, mut rx) = mpsc::channel(128);

    let handle = multitop::tasks::spawn_upgrade(0, 1, server, None, tx);
    let msgs = collect_messages(&mut rx).await;

    // Verify we got output.
    let has_output = msgs
        .iter()
        .any(|m| matches!(m, Msg::AuxLine { line, .. } if line.contains("hello-from-upgrade")));
    assert!(has_output, "upgrade streamed output");

    // Verify we got completion.
    let has_done = msgs.iter().any(|m| matches!(m, Msg::AuxDone { .. }));
    assert!(has_done, "upgrade sent AuxDone");

    handle.abort();
}

#[tokio::test]
async fn spawn_upgrade_no_password_succeeds() {
    let _g = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    // A simple command that needs no password.
    let server = Server {
        host: "127.0.0.1".into(),
        port: 0,
        user: "testuser".into(),
        upgrade_cmd: Some("echo no-pw-needed".into()),
    };
    let (tx, mut rx) = mpsc::channel(128);

    let handle = multitop::tasks::spawn_upgrade(0, 1, server, None, tx);
    let msgs = collect_messages(&mut rx).await;

    let has_done = msgs.iter().any(|m| matches!(m, Msg::AuxDone { .. }));
    assert!(has_done, "upgrade without password sent AuxDone");

    let has_output = msgs
        .iter()
        .any(|m| matches!(m, Msg::AuxLine { line, .. } if line.contains("no-pw-needed")));
    assert!(has_output, "upgrade streamed output");

    handle.abort();
}

#[tokio::test]
async fn spawn_upgrade_collapses_carriage_returns() {
    let _g = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    // printf with \r simulates a progress bar rewriting itself.
    let server = Server {
        host: "127.0.0.1".into(),
        port: 0,
        user: "testuser".into(),
        upgrade_cmd: Some("printf '10%%\\r20%%\\r30%%\\n'".into()),
    };
    let (tx, mut rx) = mpsc::channel(128);

    let handle = multitop::tasks::spawn_upgrade(0, 1, server, None, tx);
    let msgs = collect_messages(&mut rx).await;

    // The progress bar collapsed to one line ("30%"), not three.
    let progress_lines: Vec<&str> = msgs
        .iter()
        .filter_map(|m| match m {
            Msg::AuxLine { line, .. } if line.contains('%') => Some(line.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(progress_lines, vec!["30%"], "carriage returns collapsed");

    handle.abort();
}

async fn collect_messages(rx: &mut tokio::sync::mpsc::Receiver<Msg>) -> Vec<Msg> {
    let mut msgs = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(msg)) => {
                let done = matches!(msg, Msg::AuxDone { .. });
                msgs.push(msg);
                if done {
                    break;
                }
            }
            _ => break,
        }
    }
    msgs
}

// ===========================================================================
// password_actions.rs — import, rotate, cycle banner
// ===========================================================================

#[test]
fn password_action_cycle_banner() {
    let _g = isolate_keychain();

    let mut a = App::new(vec![test_server("h1")]);

    let action = PasswordAction::CycleBannerStyle;
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let mut tasks = Tasks::new(1);
    apply(action, &mut a, &tx, &mut tasks);

    // Banner style cycled.
    assert!(matches!(
        a.banner_style,
        multitop::layout::BannerStyle::Wide
    ));
}

#[test]
fn password_action_import_ssh_hosts_no_op_when_empty() {
    let _g = isolate_keychain();
    let mut a = App::new(vec![test_server("h1")]);

    // Without a real ~/.ssh/config, import is a no-op (no panic).
    let action = PasswordAction::ImportSshHosts;
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let mut tasks = Tasks::new(1);
    apply(action, &mut a, &tx, &mut tasks);
}
