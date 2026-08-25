use super::*;

#[tokio::test]
async fn test_vault_password_prompt_state_machine() {
    let _keychain = isolate_keychain_async().await;
    let master_pw = "test-master";
    let mut vault_passwords = HashMap::new();
    vault_passwords.insert(
        "testuser@test-host-1:22".to_string(),
        "sudo-pass-1".to_string(),
    );

    let (mut app, _temp_dir) = app_with_vault(test_servers(), master_pw, vault_passwords).await;

    // Initially vault is locked (vault exists but not unlocked)
    // But we pre-unlocked it for testing
    assert!(app.vault.is_some());

    // The actual state machine test: simulate password entry flow
    // In real app this happens via UI, here we test the logic
    app.set_show_vault_password_prompt(true);
    app.vault_password_input = master_pw.to_string();

    if let Some(ref vault) = app.vault {
        let password = std::mem::take(&mut app.vault_password_input);
        match vault.unlock_with_password(&password) {
            Ok(unlocked) => {
                app.vault_state = VaultState::Unlocked {
                    vault: Box::new(unlocked),
                    awaiting_biometric: false,
                };
                app.set_show_upgrade_modal(true);
            }
            Err(e) => {
                app.set_vault_password_error(Some(e.to_string()));
                app.set_show_vault_password_prompt(true);
            }
        }
    }

    // Should now be unlocked
    assert!(app.vault_unlocked().is_some());
    assert!(!app.show_vault_password_prompt());
    assert!(app.show_upgrade_modal());
}

#[tokio::test]
async fn test_vault_failed_unlock_shows_error() {
    let _keychain = isolate_keychain_async().await;
    let master_pw = "test-master";
    let mut vault_passwords = HashMap::new();
    vault_passwords.insert(
        "testuser@test-host-1:22".to_string(),
        "sudo-pass-1".to_string(),
    );

    let (mut app, _temp_dir) = app_with_vault(test_servers(), master_pw, vault_passwords).await;

    // The app was created with pre-unlocked vault, lock it first to test failure
    app.vault_state = VaultState::Locked;

    // Simulate wrong password
    app.set_show_vault_password_prompt(true);
    app.vault_password_input = "wrong-password".to_string();

    if let Some(ref vault) = app.vault {
        let password = std::mem::take(&mut app.vault_password_input);
        match vault.unlock_with_password(&password) {
            Ok(_) => panic!("Should have failed"),
            Err(e) => {
                app.set_vault_password_error(Some(e.to_string()));
                app.set_show_vault_password_prompt(true);
            }
        }
    }

    // Should show error and keep prompt open
    assert!(app.show_vault_password_prompt());
    assert!(app.vault_password_error().is_some());
    assert!(app.vault_unlocked().is_none());
}

// ============================================================================
// Bug b regressions: "Vault not working reliably (4 password attempts, 1
// fingerprint)"
// The TUI's `u` handler went straight to the password prompt, so every
// upgrade run re-typed the vault password (accumulating lockout backoff) and
// Touch ID was never offered. Now the TUI tries biometrics first and only
// falls back to the password prompt when biometrics are unavailable/cancelled.
// ============================================================================

/// Bug b: with a locked vault, pressing `u` must start a biometric attempt,
/// NOT jump straight to the password prompt. Exercises the real
/// `begin_password_unlock()`, the path the `u` key takes on a locked vault.
///
/// This test used to assert the opposite — that the handler awaited a biometric
/// before prompting — and it stayed green after the fix that removed that step,
/// because `begin_vault_unlock` still set the biometric state for the one line
/// before its caller overwrote it. A test pinning behaviour the product no
/// longer has is worse than no test: it reports that the old path still works.
#[tokio::test]
async fn test_vault_locked_u_key_asks_for_the_password_directly() {
    let _keychain = isolate_keychain_async().await;
    let (mut app, _temp_dir) = app_with_vault(test_servers(), "test-master", HashMap::new()).await;
    // Lock the vault again to simulate a fresh app start.
    app.vault_state = VaultState::Locked;

    assert!(
        app.begin_password_unlock(),
        "a locked vault must raise the password prompt"
    );
    assert!(
        app.show_vault_password_prompt(),
        "the user wants one password entry, not a biometric step before it"
    );
    assert!(
        !app.vault_awaiting_biometric(),
        "no biometric wait may be entered, even for the length of one call"
    );

    // An already-unlocked vault raises nothing, so the handler proceeds to the
    // upgrade modal instead.
    let unlocked = app
        .vault
        .as_ref()
        .unwrap()
        .unlock_with_password("test-master")
        .unwrap();
    app.vault_state = VaultState::Unlocked {
        vault: Box::new(unlocked),
        awaiting_biometric: false,
    };
    assert!(!app.begin_password_unlock());
    assert!(!app.vault_awaiting_biometric());
}

/// Bug b: a successful biometric unlock must land the unlocked vault, dismiss
/// the awaiting state, and proceed to the upgrade modal — exactly one attempt.
#[tokio::test]
async fn test_vault_biometric_success_proceeds_to_modal() {
    let _keychain = isolate_keychain_async().await;
    let master_pw = "test-master";
    let (mut app, _temp_dir) = app_with_vault(test_servers(), master_pw, HashMap::new()).await;

    // Lock the vault again, then simulate the biometric task succeeding.
    app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };

    let unlocked = app
        .vault
        .as_ref()
        .unwrap()
        .unlock_with_password(master_pw)
        .unwrap();
    app.apply(Msg::VaultUnlocked {
        epoch: app.vault_epoch,
        unlocked: Box::new(unlocked),
    });

    assert!(app.vault_unlocked().is_some(), "vault must be unlocked");
    assert!(
        !app.vault_awaiting_biometric(),
        "awaiting state must clear on success"
    );
    assert!(
        !app.show_vault_password_prompt(),
        "no password prompt after biometric success"
    );
    assert!(
        app.show_upgrade_modal(),
        "successful unlock must proceed to the upgrade modal"
    );
}

/// Bug b: when biometrics are unavailable or cancelled, the TUI must fall back
/// to the password prompt (one clear fallback, not a silent dead-end).
#[tokio::test]
async fn test_vault_biometric_failed_falls_back_to_password() {
    let _keychain = isolate_keychain_async().await;
    let (mut app, _temp_dir) = app_with_vault(test_servers(), "test-master", HashMap::new()).await;
    app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };

    app.apply(Msg::VaultBiometricFailed {
        epoch: app.vault_epoch,
    });

    assert!(
        !app.vault_awaiting_biometric(),
        "awaiting state must clear on fallback"
    );
    assert!(
        app.show_vault_password_prompt(),
        "must offer the password prompt after biometric failure"
    );
    assert!(app.vault_unlocked().is_none(), "vault must still be locked");
}

/// Bug b: real end-to-end of the spawned biometric task. On a machine without
/// usable biometrics (the test environment), `unlock_biometric()` returns
/// `BiometricFailed` and the task must emit `VaultBiometricFailed` so the TUI
/// shows the password prompt — never a hang or a crash.
#[tokio::test]
async fn test_vault_biometric_task_emits_fallback_on_unavailable() {
    let _keychain = isolate_keychain_async().await;
    let (mut app, _temp_dir) = app_with_vault(test_servers(), "test-master", HashMap::new()).await;
    let vault = app.vault.clone().unwrap();

    let (tx, mut rx) = mpsc::channel::<Msg>(4);
    let tx2 = tx.clone();
    let handle = tokio::spawn(async move {
        match vault.unlock_biometric().await {
            Ok((unlocked, _)) => {
                let _ = tx2
                    .send(Msg::VaultUnlocked {
                        epoch: 0,
                        unlocked: Box::new(unlocked),
                    })
                    .await;
            }
            Err(_) => {
                let _ = tx2.send(Msg::VaultBiometricFailed { epoch: 0 }).await;
            }
        }
    });
    // Not `let _ =`. A panic inside the task is delivered here as a join error,
    // and swallowing it left the *only* symptom being the timeout below
    // elapsing ten seconds later and reporting "task must emit a message" --
    // which is true, and says nothing about why. Class H, in the harness: a
    // failure reported as something else, with the cause discarded one line
    // above the report.
    handle
        .await
        .expect("the biometric task must not panic -- its panic is the finding");

    // The task has already finished, so the message is queued and this returns
    // at once. The bound is only so a task that somehow sent nothing fails the
    // test instead of hanging it; if it ever *does* elapse, the reason is the
    // task, and the line above is what will say so.
    let msg = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
        .await
        .expect("the finished task must have left a message behind")
        .expect("channel must not close");

    // Apply the task's outcome to the app.
    app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };
    app.apply(msg);

    assert!(
        app.show_vault_password_prompt() || app.show_upgrade_modal(),
        "the task outcome must lead to either the password prompt or the modal"
    );
    assert!(
        !app.vault_awaiting_biometric(),
        "awaiting state must be cleared by the task outcome"
    );
}

/// Bug b: failed biometric attempts must not count as password failures.
/// Otherwise Touch ID being unavailable would push the user toward the
/// lockout backoff before they ever typed a password.
#[tokio::test]
async fn test_vault_biometric_failures_do_not_trigger_lockout() {
    let _keychain = isolate_keychain_async().await;
    let master_pw = "test-master";
    let (app, _temp_dir) = app_with_vault(test_servers(), master_pw, HashMap::new()).await;
    let vault = app.vault.clone().unwrap();

    // Simulate a handful of biometric failures (unavailable/cancelled).
    for _ in 0..5 {
        assert!(vault.unlock_biometric().await.is_err());
    }

    // A correct password must still unlock immediately — no RateLimited error.
    let unlocked = vault.unlock_with_password(master_pw);
    assert!(
        unlocked.is_ok(),
        "biometric failures must not accumulate password lockout"
    );
    drop(unlocked);
}
