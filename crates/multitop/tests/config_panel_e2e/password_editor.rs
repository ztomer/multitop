use super::*;

/// The reported flow, in the shape the panel has now.
#[tokio::test]
async fn a_password_typed_in_the_row_editor_reaches_the_store() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a", "host-b"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Down);
    h.press(KeyCode::Enter);
    assert!(
        h.screen().contains("Editing server"),
        "Enter must open the row editor, got:\n{}",
        h.screen()
    );

    for _ in 0..4 {
        h.press(KeyCode::Tab);
    }
    h.type_str("just-for-b");
    h.press(KeyCode::Enter);

    assert_eq!(
        password_store::load(&server("host-b")).unwrap().as_deref(),
        Some("just-for-b"),
        "it is this host's password"
    );
    assert_eq!(
        password_store::load(&server("host-a")).unwrap(),
        None,
        "and only this host's -- setting one password must not mark others as \
         configured, which is what the shared fallback used to do"
    );
}

/// Adding a server carries its password in the same editor. There is nowhere
/// else to put one.
#[tokio::test]
async fn a_new_server_gets_its_password_in_the_same_editor() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Char('a'));
    h.type_str("host-new");
    h.press(KeyCode::Tab);
    h.type_str("admin");
    // Port is pre-filled with the default; typing into it would append.
    h.press(KeyCode::Tab);
    h.press(KeyCode::Tab);
    h.type_str("ls -l");
    h.press(KeyCode::Tab);
    h.type_str("new-secret");
    h.press(KeyCode::Enter);

    assert_eq!(h.app.panels.len(), 2, "the server must be added");
    assert_eq!(
        password_store::load(&server("host-new"))
            .unwrap()
            .as_deref(),
        Some("new-secret")
    );
}

/// Clearing the field is how a password is taken back, now that the separate
/// Passwords list is gone.
#[tokio::test]
async fn clearing_the_password_field_removes_the_stored_password() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);
    password_store::save(&server("host-a"), "old-secret").unwrap();
    h.app.panels[0].sudo_password = Some("old-secret".to_string());
    h.app.panels[0].password_saved = true;

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Enter);
    for _ in 0..4 {
        h.press(KeyCode::Tab);
    }
    for _ in 0.."old-secret".len() {
        h.press(KeyCode::Backspace);
    }
    h.press(KeyCode::Enter);

    assert_eq!(
        password_store::load(&server("host-a")).unwrap(),
        None,
        "an emptied field must not leave the old password behind"
    );
}

/// Non-ASCII input must survive the round trip through the mask and the buffer.
#[tokio::test]
async fn a_password_with_wide_and_multibyte_characters_renders() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Enter);
    for _ in 0..4 {
        h.press(KeyCode::Tab);
    }
    h.type_str("pä55wörd-\u{4f60}\u{597d}");
    h.press(KeyCode::Backspace);
    h.press(KeyCode::Enter);

    assert_eq!(
        password_store::load(&server("host-a")).unwrap().as_deref(),
        Some("pä55wörd-\u{4f60}")
    );
}

/// The vault offer follows the first stored password, and is part of the flow.
#[tokio::test]
async fn the_vault_creation_offer_renders_and_can_be_declined() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Enter);
    for _ in 0..4 {
        h.press(KeyCode::Tab);
    }
    h.type_str("first-password");
    h.press(KeyCode::Enter);

    assert!(
        h.app.vault_creating(),
        "saving the first password must offer a vault"
    );
    h.type_str("master");
    h.draw();
    h.press(KeyCode::Esc);
    assert!(!h.app.vault_creating(), "Esc must decline the offer");
}
