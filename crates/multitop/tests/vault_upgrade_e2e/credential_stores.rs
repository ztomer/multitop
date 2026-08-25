use super::*;

/// The first `u` -- the read-only screen that says what an upgrade *would* do --
/// must not reach the OS credential store when a vault exists.
///
/// Reported: "Upgrade - I had to enter the vault password twice - one to unlock
/// the vault, and one when initiating updates." Entering the view asked the
/// credential store about every host so it could report "password stored". On
/// macOS that is a system credential dialog per host, raised *before* the vault
/// is ever unlocked, because the binary's code signature changes on every
/// rebuild and the keychain grant never sticks. Two prompts for one upgrade.
///
/// Once a vault exists it holds everything the credential store holds -- saving
/// a password writes to both -- so the store has nothing to add and is not
/// asked. The proxy for "was it asked" is whether the panel came back holding a
/// password that only the store had.
#[tokio::test]
async fn entering_the_upgrade_view_does_not_read_the_credential_store_when_a_vault_exists() {
    let _keychain = isolate_keychain_async().await;
    let (mut app, _temp_dir) = app_with_vault(test_servers(), "test-master", HashMap::new()).await;
    // Only the credential store knows this one.
    password_store::save(&app.panels[0].server, "only-in-the-keychain").unwrap();
    // A fresh start: the vault exists and is locked.
    app.vault_state = VaultState::Locked;

    app.enter_upgrade_view();

    assert_eq!(
        app.panels[0].sudo_password, None,
        "the credential store must not be read behind a locked vault"
    );
    // Through the renderer, not `panels[0].view`: the Upgrade pane is composed
    // from the status header and the `last_upgrade` ring, and `view` is the
    // buffer the *other* modes use.
    let pane = multitop::ui::pane_lines(&app, 0, usize::MAX, 0, 0)
        .0
        .join("\n");
    assert!(
        pane.contains("unlocks on run"),
        "and the pane must say the vault will be asked, not guess: \n{pane}"
    );
    assert!(
        !pane.contains("will prompt"),
        "a locked vault is not a missing password: \n{pane}"
    );
}

/// Without a vault the credential store is the only store there is, so the same
/// screen must still read it -- that is what makes "password stored" true.
#[tokio::test]
async fn entering_the_upgrade_view_still_reads_the_credential_store_without_a_vault() {
    let _keychain = isolate_keychain_async().await;
    let mut app = App::new(test_servers());
    password_store::save(&app.panels[0].server, "keychain-only").unwrap();

    app.enter_upgrade_view();

    assert_eq!(
        app.panels[0].sudo_password.as_deref(),
        Some("keychain-only"),
        "with no vault, the credential store is the source of truth"
    );
    assert!(multitop::ui::pane_lines(&app, 0, usize::MAX, 0, 0)
        .0
        .join("\n")
        .contains("password stored"));
}

/// Deleting a password must delete it from **both** stores.
///
/// A credential lives in the OS credential store and in the vault. `Save`
/// wrote to both; `Delete` wrote to the credential store only -- and the vault
/// is the one that is read *first*, so emptying a host's password field removed
/// the keychain entry, left the vault entry standing, reported "this host now
/// has none", and the password came straight back on the next load. The one
/// asymmetric operation over a two-store credential was the whole defect.
#[tokio::test]
async fn deleting_a_password_removes_it_from_the_vault_too() {
    let _keychain = isolate_keychain_async().await;
    let (mut app, _temp_dir) = app_with_vault(test_servers(), "test-master", HashMap::new()).await;
    let key = password_store::account(&app.panels[0].server);

    // Saved the way the app saves it: both stores.
    password_store::save(&app.panels[0].server, "the-secret").unwrap();
    app.vault_unlocked_mut()
        .unwrap()
        .set_password(key.clone(), &SecretString::from("the-secret".to_string()))
        .unwrap();
    app.panels[0].sudo_password = Some("the-secret".to_string());
    app.panels[0].password_saved = true;

    let (tx, _rx) = mpsc::channel::<multitop::app::Msg>(16);
    let mut tasks = multitop::run::Tasks::new(app.panels.len());
    multitop::password_actions::apply(
        multitop::passwords::PasswordAction::Delete { panel: 0 },
        &mut app,
        &tx,
        &mut tasks,
    );

    assert_eq!(
        password_store::load(&app.panels[0].server),
        Ok(None),
        "the credential store entry must be gone"
    );
    assert!(
        app.vault_unlocked_mut()
            .unwrap()
            .get_password(&key)
            .is_none(),
        "and so must the vault's, or the password comes back on the next load"
    );

    // The proof that matters: the app must not be able to find it again.
    app.enter_upgrade_view();
    assert_eq!(
        app.panels[0].sudo_password, None,
        "a deleted password must not return from the store that was skipped"
    );
}
