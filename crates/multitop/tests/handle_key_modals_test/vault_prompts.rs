use super::*;

#[tokio::test]
async fn the_creation_prompt_takes_a_password_and_can_be_corrected() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    assert!(app.begin_vault_creation());
    let mut k = Keys::new(1);

    k.type_str(&mut app, "hunter2");
    assert_eq!(app.vault_password_input(), "hunter2");
    k.press(&mut app, KeyCode::Backspace);
    assert_eq!(app.vault_password_input(), "hunter");
    // Keys the prompt has no use for leave what was typed alone.
    k.press(&mut app, KeyCode::Up);
    k.press(&mut app, KeyCode::Tab);
    assert_eq!(app.vault_password_input(), "hunter");
}

#[tokio::test]
async fn an_empty_master_password_is_refused_rather_than_accepted() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    assert!(app.begin_vault_creation());
    let mut k = Keys::new(1);

    k.press(&mut app, KeyCode::Enter);
    assert!(
        app.vault_creating(),
        "the prompt was dismissed on an empty password"
    );
    assert!(
        !app.vault_create_in_flight(),
        "an empty password started a create"
    );
    assert!(app.vault_create_error().is_some(), "no reason was given");
    assert!(k.rx.try_recv().is_err(), "an empty password spawned work");
}

#[tokio::test]
async fn a_creation_with_nowhere_to_put_the_vault_says_so() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    // A config path is what names the directory, so force the state directly:
    // `begin_vault_creation` refuses without one.
    app.config_path = Some(std::path::PathBuf::from("config.toml"));
    assert!(app.begin_vault_creation());
    app.config_path = Some(std::path::PathBuf::new());

    let mut k = Keys::new(1);
    k.type_str(&mut app, "hunter2");
    k.press(&mut app, KeyCode::Enter);

    assert!(app.vault_create_error().is_some(), "the failure was silent");
    assert!(!app.vault_create_in_flight());
}

#[tokio::test]
async fn escape_declines_the_vault_offer_without_losing_the_password() {
    // Declining leaves the password in the OS credential store, which still
    // works; only the encrypted vault is skipped.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    assert!(app.begin_vault_creation());
    let mut k = Keys::new(1);

    k.type_str(&mut app, "typed");
    k.press(&mut app, KeyCode::Esc);
    assert!(!app.vault_creating());
    assert!(
        app.vault_password_input().is_empty(),
        "the typed password was kept"
    );
}

#[tokio::test]
async fn while_a_vault_is_being_created_only_escape_still_means_anything() {
    // Argon2id is running and there is nothing to type into, but the user must
    // never be trapped waiting for it.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    assert!(app.begin_vault_creation());
    app.vault_password_input_mut().push_str("hunter2");
    assert!(app.begin_vault_create_attempt().is_some());
    assert!(app.vault_create_in_flight());

    let mut k = Keys::new(1);
    k.type_str(&mut app, "more");
    assert!(
        app.vault_password_input().is_empty(),
        "keys reached the field"
    );
    assert!(
        app.vault_create_in_flight(),
        "a stray key cancelled the create"
    );

    k.press(&mut app, KeyCode::Esc);
    assert!(!app.vault_creating(), "Esc did not give up on waiting");
}

// -------------------------------------------------------- the password prompt

#[tokio::test]
async fn the_unlock_prompt_takes_a_password_and_can_be_corrected() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    app.set_show_vault_password_prompt(true);
    let mut k = Keys::new(1);

    k.type_str(&mut app, "secret");
    assert_eq!(app.vault_password_input(), "secret");
    k.press(&mut app, KeyCode::Backspace);
    assert_eq!(app.vault_password_input(), "secre");
    k.press(&mut app, KeyCode::Down);
    assert_eq!(app.vault_password_input(), "secre");
}

#[tokio::test]
async fn an_empty_unlock_attempt_does_not_start_an_unwrap() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    app.set_show_vault_password_prompt(true);
    let mut k = Keys::new(1);

    k.press(&mut app, KeyCode::Enter);
    assert!(
        app.show_vault_password_prompt(),
        "an empty password left the prompt"
    );
    assert!(
        k.rx.try_recv().is_err(),
        "an empty password spawned an unwrap"
    );
}

#[tokio::test]
async fn escape_leaves_the_unlock_prompt_and_clears_what_was_typed() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    app.set_show_vault_password_prompt(true);
    app.set_vault_password_error(Some("wrong password".into()));
    let mut k = Keys::new(1);

    k.type_str(&mut app, "secret");
    k.press(&mut app, KeyCode::Esc);

    assert!(!app.show_vault_password_prompt());
    assert!(
        app.vault_password_input().is_empty(),
        "the password was kept"
    );
    assert!(
        app.vault_password_error().is_none(),
        "the stale error survived into the next prompt"
    );
}

// ------------------------------------------------------------ the async prompts

#[tokio::test]
async fn a_hung_biometric_prompt_can_still_be_escaped() {
    // The outcome normally arrives as a message; if that task dies or hangs,
    // every key including quit was swallowed and the app could only be killed.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();

    for code in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('Q')] {
        let mut app = app_with_config(&dir, &["alpha"]);
        // Set directly: reaching this state through `begin_vault_unlock`
        // needs a real vault, and unwrapping one costs an Argon2id pass.
        app.vault_state = VaultState::Unlocking {
            awaiting_biometric: true,
        };
        assert!(app.vault_awaiting_biometric());
        let mut k = Keys::new(1);

        k.press(&mut app, code);
        assert!(
            !app.vault_awaiting_biometric(),
            "{code:?} did not get the user out"
        );
    }

    // Anything else is swallowed while the prompt is up.
    let mut app = app_with_config(&dir, &["alpha"]);
    app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };
    let mut k = Keys::new(1);
    k.press(&mut app, KeyCode::Char('u'));
    assert!(app.vault_awaiting_biometric());
}

#[tokio::test]
async fn a_password_being_verified_can_still_be_escaped() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    app.set_vault_unlocking();
    assert!(app.vault_verifying());
    let mut k = Keys::new(1);

    // Stray keys are swallowed; Esc is not.
    k.press(&mut app, KeyCode::Char('x'));
    assert!(app.vault_verifying());
    k.press(&mut app, KeyCode::Esc);
    assert!(!app.vault_verifying(), "Esc did not cancel the verify");
}
