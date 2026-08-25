use super::*;

/// Walk from the row editor to a created vault, the way a user does.
///
/// Returns with the vault made and every message delivered.
async fn create_a_vault(h: &mut Harness, master: &str) -> usize {
    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Enter);
    for _ in 0..4 {
        h.press(KeyCode::Tab);
    }
    h.type_str("host-secret");
    h.press(KeyCode::Enter);
    assert!(h.app.vault_creating(), "the first password offers a vault");
    h.type_str(master);
    h.press(KeyCode::Enter);
    h.pump(std::time::Duration::from_secs(10)).await
}

/// Answering the vault offer must leave the user where they were.
///
/// Reported: "when setting the vault password, stay on the settings pane, do
/// not switch back to the stats panel." It did switch, because the renderer
/// drew either the configuration panel or a modal and never both, so the offer
/// could only be shown by closing the panel first.
#[tokio::test]
async fn creating_a_vault_from_server_settings_stays_in_server_settings() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Enter);
    for _ in 0..4 {
        h.press(KeyCode::Tab);
    }
    h.type_str("host-secret");
    h.press(KeyCode::Enter);

    assert!(h.app.vault_creating(), "the offer must appear");
    assert!(
        h.app.password_manager.is_some(),
        "the offer must not close Server Settings"
    );
    assert!(
        h.screen().contains("Create Vault"),
        "and the prompt must be drawn over the panel, got:\n{}",
        h.screen()
    );

    // Declining is the same story: back to the list, not to the stats screen.
    h.press(KeyCode::Esc);
    assert!(!h.app.vault_creating());
    assert!(
        h.app.password_manager.is_some(),
        "Esc returns to the settings list"
    );
    assert!(h.screen().contains("Settings"), "got:\n{}", h.screen());
}

/// The master password is taken once, however many times Enter is pressed.
///
/// Reported: "I had to enter the vault password three times when creating it."
/// Enter handed the password to Argon2id and left the prompt on screen with an
/// empty field, so it read as not having taken -- and every re-submission
/// initialised the vault again. The later attempts failed (a vault existed by
/// then) and their failures, carrying the same epoch as the first attempt's
/// success, put the creation prompt back up over a working vault.
#[tokio::test]
async fn a_second_enter_cannot_start_a_second_vault_creation() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Enter);
    for _ in 0..4 {
        h.press(KeyCode::Tab);
    }
    h.type_str("host-secret");
    h.press(KeyCode::Enter);
    assert!(h.app.vault_creating());

    h.type_str("master-once");
    h.press(KeyCode::Enter);
    assert!(
        h.app.vault_create_in_flight(),
        "the prompt must report that it took the password"
    );
    let screen = h.screen();
    assert!(
        screen.contains("Creating Vault"),
        "and say so on screen, got:\n{screen}"
    );

    // What a user does when a field looks empty: type it again. Twice.
    h.type_str("master-once");
    h.press(KeyCode::Enter);
    h.type_str("master-once");
    h.press(KeyCode::Enter);

    let delivered = h.pump(std::time::Duration::from_secs(10)).await;
    assert_eq!(
        delivered, 1,
        "one Enter, one attempt -- extra presses must not initialise the vault again"
    );
    assert!(
        !h.app.vault_creating(),
        "the prompt must be gone once the vault exists, not back with an error: {:?}",
        h.app.vault_create_error()
    );
    assert!(h.app.vault.is_some(), "and the vault must exist");
    assert!(
        h.app.password_manager.is_some(),
        "still in Server Settings afterwards"
    );
    assert!(
        h.notice().contains("Vault created"),
        "with the outcome said where the user is looking, got: {:?}",
        h.notice()
    );
}

/// The password that started all this must be in the vault it created.
#[tokio::test]
async fn the_password_that_offered_the_vault_is_stored_in_it() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);

    assert_eq!(create_a_vault(&mut h, "master-once").await, 1);
    assert!(h.app.vault.is_some());
    assert_eq!(
        h.app.panels[0].sudo_password.as_deref(),
        Some("host-secret")
    );
}
