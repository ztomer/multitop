use super::*;

#[tokio::test]
async fn the_cleanup_command_keeps_the_agents_this_build_ships() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    let cmd = multitop::ssh::cleanup_old_agents_command();

    let kept: Vec<String> = [multitop::ssh::Arch::X86_64, multitop::ssh::Arch::Aarch64]
        .iter()
        .map(|a| a.hash().to_string())
        .filter(|h| h != "missing" && !h.is_empty())
        .collect();

    // The invariant, and the reason this test was rewritten: a catch-all delete
    // must never appear without something to keep.
    //
    // It used to assert the command *always* contained `agent-*) rm -f`, which
    // is the dangerous shape stated as the requirement. In a build with no
    // agent embedded -- a plain `cargo build` rather than `./build.sh` -- both
    // hashes are "missing", every keep-arm is filtered out, and what was left
    // was a loop that deleted every agent in the cache including the one in
    // use. That was unreachable only because the sweep runs after a successful
    // upload and an upload needs an embedded agent, so the safety lived in a
    // different function's ordering. The test asserted the landmine was there.
    if kept.is_empty() {
        assert!(
            !cmd.contains("rm -f"),
            "a build that ships no agent generated a command that deletes them:\n{cmd}"
        );
        return;
    }

    // Structure, folded in from a weaker duplicate in `app_test.rs` that
    // asserted `rm -f` unconditionally -- with a comment noting that the keep
    // list "may be empty" in a debug build, which is precisely the case where
    // that assertion demands the destructive command.
    assert!(cmd.starts_with("cd ~/.cache/multitop"), "{cmd}");
    assert!(cmd.contains("for f in agent-*"), "{cmd}");
    assert!(cmd.contains("case") && cmd.contains("esac"), "{cmd}");
    assert!(cmd.contains("done"), "{cmd}");
    assert!(cmd.contains("agent-*) rm -f"), "{cmd}");
    // Every hash this build has is spared.
    for hash in &kept {
        assert!(
            cmd.contains(&format!("agent-{hash}) continue")),
            "the current agent {hash} would be deleted:\n{cmd}"
        );
    }
}

// ------------------------------------------------------------ docker sorting

#[tokio::test]
async fn a_memory_tie_is_broken_by_cpu_and_then_by_name() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;

    let row = |name: &str, cpu: f64, mem: u64| Row {
        name: name.into(),
        status: "Up".into(),
        image: "nginx:latest".into(),
        cpu: format!("{cpu:.1}%"),
        cpu_pct: cpu,
        mem: "-".into(),
        mem_bytes: mem,
    };
    // All three use the same memory, so the order is decided by CPU; the last
    // two also tie on CPU, so their names decide.
    let rows = vec![
        row("charlie", 1.0, 1000),
        row("bravo", 1.0, 1000),
        row("alpha", 9.0, 1000),
    ];

    let frame = render("h", 100, 24, &rows, &PLAIN, SortBy::Mem);
    let body: Vec<&String> = frame
        .iter()
        .filter(|l| l.contains("alpha") || l.contains("bravo") || l.contains("charlie"))
        .collect();
    assert_eq!(body.len(), 3, "{frame:?}");
    assert!(
        body[0].contains("alpha"),
        "cpu did not break the memory tie: {body:?}"
    );
    assert!(
        body[1].contains("bravo"),
        "name did not break the cpu tie: {body:?}"
    );
    assert!(body[2].contains("charlie"), "{body:?}");
}

// ------------------------------------------------------ the rotation prompt

#[tokio::test]
async fn the_rotation_prompt_takes_two_passwords_in_turn() {
    // `r` asks for the current password, then the new one. Neither is checked
    // here: verifying means running the KDF, which belongs off the event loop.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("vault.bin");
    // A vault file only has to exist for `r` to be offered.
    std::fs::write(&vault_path, b"placeholder").unwrap();

    let mut app = App::new(vec![test_server("alpha")]);
    app.config_path = Some(dir.path().join("config.toml"));
    app.vault = Some(std::sync::Arc::new(multitop_vault::Vault::new(
        multitop::vault::config_for(vault_path),
    )));
    open(&mut app, 0, false);

    handle_key(&mut app, KeyCode::Char('r'));
    for c in "oldd".chars() {
        handle_key(&mut app, KeyCode::Char(c));
    }
    // A typo is corrected in place rather than by starting over.
    handle_key(&mut app, KeyCode::Backspace);
    // Keys the field has no use for leave it alone.
    handle_key(&mut app, KeyCode::Up);
    assert!(matches!(
        handle_key(&mut app, KeyCode::Enter),
        PasswordAction::None
    ));

    for c in "new-master".chars() {
        handle_key(&mut app, KeyCode::Char(c));
    }
    let action = handle_key(&mut app, KeyCode::Enter);
    let PasswordAction::RotateVaultPassword { current, new } = action else {
        panic!("the second Enter must start the rotation, got {action:?}");
    };
    assert_eq!(
        current, "old",
        "the corrected first password was not carried"
    );
    assert_eq!(new, "new-master");
}

#[tokio::test]
async fn an_empty_answer_to_the_rotation_prompt_changes_nothing() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("vault.bin");
    std::fs::write(&vault_path, b"placeholder").unwrap();

    let mut app = App::new(vec![test_server("alpha")]);
    app.config_path = Some(dir.path().join("config.toml"));
    app.vault = Some(std::sync::Arc::new(multitop_vault::Vault::new(
        multitop::vault::config_for(vault_path),
    )));
    open(&mut app, 0, false);

    handle_key(&mut app, KeyCode::Char('r'));
    assert!(matches!(
        handle_key(&mut app, KeyCode::Enter),
        PasswordAction::None
    ));
    let notice = app
        .password_manager
        .as_ref()
        .and_then(|m| m.notice.clone())
        .unwrap_or_default();
    assert!(notice.contains("not changed"), "{notice}");

    // Enter with nothing staged at all is also inert rather than a panic.
    assert!(matches!(
        handle_key(&mut app, KeyCode::Enter),
        PasswordAction::None
    ));
}

#[tokio::test]
async fn rotation_is_offered_only_when_there_is_a_vault_and_no_run_in_flight() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    open(&mut app, 0, false);

    handle_key(&mut app, KeyCode::Char('r'));
    let notice = app
        .password_manager
        .as_ref()
        .and_then(|m| m.notice.clone())
        .unwrap_or_default();
    assert!(notice.contains("No vault to rotate"), "{notice}");

    // And while one is running, `r` says so rather than starting a second.
    app.password_manager.as_mut().unwrap().rotating = true;
    handle_key(&mut app, KeyCode::Char('r'));
    let notice = app
        .password_manager
        .as_ref()
        .and_then(|m| m.notice.clone())
        .unwrap_or_default();
    assert!(notice.contains("already being changed"), "{notice}");
}

// -------------------------------------------------------------- config saves
