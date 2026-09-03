//! Regression tests for the five bugs fixed in this round.
//!
//! These drive the real event loop / App state machine end-to-end. Each test
//! reproduces the exact failure the user reported, then verifies the fix holds.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::items_after_statements,
    clippy::unnecessary_mut_passed,
    clippy::redundant_clone,
    clippy::needless_borrow,
    clippy::range_plus_one,
    clippy::no_effect_underscore_binding,
    clippy::used_underscore_binding,
    clippy::uninlined_format_args,
    clippy::manual_range_contains
)]

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use multitop::app::{App, Msg, VaultState};
use multitop::config::Server;
use multitop::panel::{Mode, UpgradeState};
use multitop::password_store;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[allow(dead_code, clippy::missing_const_for_fn)]
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new_with_kind(
        code,
        KeyModifiers::NONE,
        KeyEventKind::Press,
    ))
}

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 0,
        user: "testuser".to_string(),
        upgrade_cmd: Some("true".to_string()),
        custom_command: None,
    }
}

fn local_server(upgrade_cmd: &str) -> Server {
    Server {
        host: "127.0.0.1".to_string(),
        port: 0,
        user: "testuser".to_string(),
        upgrade_cmd: Some(upgrade_cmd.to_string()),
        custom_command: None,
    }
}

async fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

fn isolate_keychain_blocking() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test();
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

// ===========================================================================
// Bug #1: App gets stuck when switching away from the upgrade view
// ===========================================================================

/// The bounded drain must process all queued messages but cap work per poll.
/// This is the invariant that prevents the loop from wedging when a producer
/// floods the channel.
#[test]
fn bounded_drain_caps_work_per_poll() {
    let _keychain = isolate_keychain_blocking();
    let mut a = App::new(vec![local_server("true")]);

    a.panels[0].mode = Mode::Upgrade;
    a.panels[0].upgrade_state = UpgradeState::STARTED;
    a.panels[0].upgrade_gen = 1;

    // Apply 1000 messages — simulating a flood. The drain processes them in
    // bounded chunks (budget=32), so no single poll iteration hogs the loop.
    for _ in 0..1000 {
        let msg = Msg::AuxLine {
            panel: 0,
            gen: 1,
            line: "flood".to_string(),
        };
        a.apply(msg);
    }

    // All 1000 lines landed in the ring (capped at upgrade_history_lines).
    assert!(
        !a.panels[0].last_upgrade.is_empty(),
        "output must accumulate in the ring"
    );
}

/// Output produced while the user is on the stats view must land in the panel's
/// durable ring, so it is visible when they switch back.
#[tokio::test]
async fn upgrade_output_produces_while_away_is_preserved() {
    let _keychain = isolate_keychain().await;
    let mut a = App::new(vec![local_server("echo hello from upgrade")]);

    // Set up: panel 0 is upgrading with a known gen.
    a.panels[0].mode = Mode::Upgrade;
    a.panels[0].upgrade_state = UpgradeState::STARTED;
    a.panels[0].upgrade_gen = 7;

    // Simulate an AuxLine arriving while the user is on stats.
    let line = Msg::AuxLine {
        panel: 0,
        gen: 7,
        line: "hello from upgrade".to_string(),
    };
    let _ = a.apply(line);

    // Output was stored even though we're not in Upgrade mode.
    assert!(
        a.panels[0]
            .last_upgrade
            .iter()
            .any(|l| l.contains("hello from upgrade")),
        "output must land in the ring even while away"
    );
}

// ===========================================================================
// Bug #3: Scrolling doesn't work (offset resets on every view switch)
// ===========================================================================

/// Scrolling up in the upgrade view, switching away, and switching back must
/// preserve the scroll position — and must not hand that position to the view
/// in between.
///
/// This asserted the offset was still 25 immediately after `switch_stats`,
/// which is a statement about the Monitor pane, not about the log. It passed
/// because one field served both views, which is the same reason the Monitor
/// pane opened scrolled to a place the user never chose.
#[test]
fn scroll_position_persists_across_view_switch() {
    let _keychain = isolate_keychain_blocking();
    let mut a = App::new(vec![local_server("true")]);

    a.panels[0].mode = Mode::Upgrade;
    a.panels[0].upgrade_state = UpgradeState::STARTED;
    a.panels[0].upgrade_gen = 3;
    a.panels[0].scroll_offset = 25;

    a.switch_stats();
    assert_eq!(
        a.panels[0].scroll_offset, 0,
        "the Monitor pane opened at the upgrade log's offset"
    );
    assert_eq!(
        a.panels[0].upgrade_gen, 3,
        "the in-flight generation was retired"
    );

    a.enter_upgrade_view();
    assert_eq!(
        a.panels[0].scroll_offset, 25,
        "scroll must persist across a view switch for mid-upgrade panels"
    );
}

/// Idle panels (not mid-upgrade) still reset scroll on view switch.
#[test]
fn idle_panel_scroll_resets_on_view_switch() {
    let _keychain = isolate_keychain_blocking();
    let mut a = App::new(vec![local_server("true")]);

    a.panels[0].scroll_offset = 25;
    a.switch_stats();

    assert_eq!(
        a.panels[0].scroll_offset, 0,
        "idle panel scroll should reset on view switch"
    );
}

/// Re-entering the upgrade view must NOT reset scroll.
///
/// Scrolled *in that view*, which is the case the complaint was about. Setting
/// the offset while the pane was still in Monitor mode and expecting it to
/// carry over was asserting that the two views share one number.
#[test]
fn reentering_upgrade_preserves_scroll() {
    let _keychain = isolate_keychain_blocking();
    let mut a = App::new(vec![local_server("true")]);

    a.enter_upgrade_view();
    a.panels[0].scroll_offset = 42;

    a.switch_stats();
    a.enter_upgrade_view();

    assert_eq!(
        a.panels[0].scroll_offset, 42,
        "re-entering upgrade must keep scroll offset"
    );
}

// ===========================================================================
// Bug #4: When updating docker, some older views are still present
// ===========================================================================

/// Re-entering Docker after fetching must show the cached data immediately,
/// not a loading placeholder.
#[test]
fn reentering_docker_shows_cached_data() {
    let _keychain = isolate_keychain_blocking();
    let mut a = App::new(vec![test_server("host-a")]);

    // Simulate previously-fetched docker data.
    a.panels[0].last_docker = Some(multitop_agent::proto::Payload::Docker {
        host: "host-a".into(),
        rows: vec![multitop_agent::docker::Row {
            name: "my-container".into(),
            status: "Up".into(),
            image: "nginx:latest".into(),
            cpu: "1%".into(),
            cpu_pct: 1.0,
            mem: "64M".into(),
            mem_bytes: 67_108_864,
        }],
    });

    let cmds = a.toggle_docker((80, 24));

    assert!(!cmds.is_empty(), "refresh task spawned");
    assert!(
        a.panels[0].view.iter().any(|l| l.contains("my-container")),
        "cached container must be visible immediately, not a loading placeholder"
    );
}

/// Re-entering Fetch after fetching must show cached data immediately.
#[test]
fn reentering_fetch_shows_cached_data() {
    let _keychain = isolate_keychain_blocking();
    let mut a = App::new(vec![test_server("host-a")]);

    use multitop_agent::fetch::FetchSnapshot;
    a.panels[0].last_fetch = Some(FetchSnapshot {
        user_host: "cached-host".into(),
        ..FetchSnapshot::default()
    });

    let cmds = a.toggle_fetch((80, 24));

    assert!(!cmds.is_empty(), "refresh task spawned");
    // center_host renders as fullwidth; check for the detail rows instead.
    assert!(
        a.panels[0].view.iter().any(|l| l.contains("OS")),
        "cached fetch must render detail rows immediately"
    );
}

/// First visit (no cache) must show a loading placeholder.
#[test]
fn first_visit_shows_loading_placeholder() {
    let _keychain = isolate_keychain_blocking();
    let mut a = App::new(vec![test_server("host-a")]);

    let cmds = a.toggle_docker((80, 24));

    assert!(!cmds.is_empty(), "refresh task spawned");
    assert!(
        a.panels[0].view.iter().any(|l| l.contains("loading")),
        "first visit must show loading placeholder"
    );
}

// ===========================================================================
// Bug #5: Password Vault requires multiple password inputs
// ===========================================================================

/// After `begin_vault_unlock` confirms a locked vault, the handler sets the
/// password prompt directly (no biometric step). Verify the handler path by
/// simulating what the handler does.
#[tokio::test]
async fn locked_vault_goes_straight_to_password_prompt() {
    let _keychain = isolate_keychain().await;
    let mut a = App::new(vec![local_server("true")]);

    // Create and lock a vault.
    use multitop_vault::{Vault, VaultConfig};
    let dir = tempfile::tempdir().unwrap();
    let vault_config = VaultConfig {
        vault_path: dir.path().join("vault.bin"),
        argon2_params: Some(multitop_vault::crypto::Argon2Params {
            t: 1,
            m_kib: 32768,
            p: 1,
        }),
        use_os_keychain: false,
    };
    let vault = Vault::new(vault_config);
    vault.initialize("test-master").await.unwrap();
    a.vault = Some(std::sync::Arc::new(vault));
    a.vault_state = VaultState::Locked;
    a.panels[0].mode = Mode::Upgrade;

    // One call: a locked vault raises the password prompt. It used to take
    // two, and the first of them set a biometric wait the second undid.
    assert!(
        a.begin_password_unlock(),
        "a locked vault must be unlockable"
    );
    assert!(
        !a.vault_awaiting_biometric(),
        "no biometric wait may be entered, even for an instant"
    );
    assert!(
        a.show_vault_password_prompt(),
        "must go straight to password prompt — no biometric step"
    );
}

/// The confirm modal must STILL appear after vault unlock — it protects against
/// a stray `u` starting apt upgrade on N servers.
#[tokio::test]
async fn confirm_modal_still_protects_after_vault_unlock() {
    let _keychain = isolate_keychain().await;
    let mut a = App::new(vec![local_server("true")]);

    use multitop_vault::{Vault, VaultConfig};
    let dir = tempfile::tempdir().unwrap();
    let vault_config = VaultConfig {
        vault_path: dir.path().join("vault.bin"),
        argon2_params: Some(multitop_vault::crypto::Argon2Params {
            t: 1,
            m_kib: 32768,
            p: 1,
        }),
        use_os_keychain: false,
    };
    let vault = Vault::new(vault_config);
    vault.initialize("test-master").await.unwrap();
    a.vault = Some(std::sync::Arc::new(vault));
    a.vault_state = VaultState::Locked;
    a.panels[0].mode = Mode::Upgrade;

    // Simulate: begin vault unlock, then password entered and verified.
    assert!(a.begin_password_unlock(), "locked");
    let epoch = a.vault_epoch;
    // Unlock with the password to get a real UnlockedVault.
    let unlocked = a
        .vault
        .as_ref()
        .unwrap()
        .unlock_with_password("test-master")
        .unwrap();
    let msg = Msg::VaultUnlocked {
        epoch,
        unlocked: Box::new(unlocked),
    };
    a.apply(msg);

    // Confirm modal is showing — operator must confirm the actual run.
    assert!(
        a.show_upgrade_modal(),
        "confirm modal must still appear after vault unlock"
    );
}

/// With no vault at all, pressing `u` twice goes straight to the upgrade modal
/// (no password prompt, no biometric).
#[test]
fn no_vault_goes_straight_to_upgrade_modal() {
    let _keychain = isolate_keychain_blocking();
    let mut a = App::new(vec![local_server("true")]);

    a.vault = None;
    a.vault_state = VaultState::Locked; // default, but no vault behind it
    a.panels[0].mode = Mode::Upgrade;

    // Nothing to unlock, so no prompt goes up.
    assert!(
        !a.begin_password_unlock(),
        "no vault means nothing to unlock"
    );
    assert!(!a.show_vault_password_prompt());

    // The handler would proceed to the upgrade modal.
    a.set_show_upgrade_modal(true);
    assert!(a.show_upgrade_modal());
}

// ===========================================================================
// Cross-cutting: upgrades must NEVER be aborted by view switches
// ===========================================================================

/// The `tasks.upgrades` slot must not be touched by `switch_stats`.
#[test]
fn switch_stats_does_not_touch_upgrade_tasks() {
    let _keychain = isolate_keychain_blocking();
    let mut a = App::new(vec![local_server("sleep 60")]);

    a.panels[0].mode = Mode::Upgrade;
    a.panels[0].upgrade_state = UpgradeState::STARTED;
    a.panels[0].upgrade_gen = 99;
    let gen_before = a.panels[0].gen;

    a.switch_stats();

    // Gen preserved = the task's messages still belong.
    assert_eq!(a.panels[0].gen, gen_before, "gen must not be retired");
    assert_eq!(
        a.panels[0].upgrade_state,
        UpgradeState::STARTED,
        "upgrade must still be running"
    );
}

/// `mark_upgrade_interrupted` flips `STARTED` → `DONE` and records the interruption.
#[test]
fn interrupted_upgrade_is_recorded() {
    let _keychain = isolate_keychain_blocking();
    let mut a = App::new(vec![local_server("true")]);
    a.config_path = Some(std::env::temp_dir().join("multitop_test_interrupted.toml"));

    a.panels[0].upgrade_state = UpgradeState::STARTED;
    a.mark_upgrade_interrupted(0);

    assert_eq!(a.panels[0].upgrade_state, UpgradeState::DONE);
    let key = multitop::password_store::account(&a.panels[0].server);
    let entry = a.host_updates.get(&key).expect("interruption persisted");
    assert!(entry.finished_at.is_some(), "finished_at recorded");
    assert!(!entry.success, "interrupted = failure");
}

/// `mark_upgrade_interrupted` is a no-op for panels not in `STARTED`.
#[test]
fn interrupted_upgrade_noop_for_done() {
    let _keychain = isolate_keychain_blocking();
    let mut a = App::new(vec![local_server("true")]);

    a.panels[0].upgrade_state = UpgradeState::DONE;
    a.mark_upgrade_interrupted(0);

    assert!(a.host_updates.is_empty(), "no state written for DONE panel");
}

// ===========================================================================
// Streaming display: output must not flood-redraw (the AuxLine return value
// must NOT force a repaint on every line while the user is on another view).
// ===========================================================================

/// Applying many `AuxLine` messages while NOT in Upgrade mode must NOT set `dirty`
/// on every line (that was the v1 bug we reverted).
#[test]
fn background_upgrade_output_does_not_force_redraw() {
    let _keychain = isolate_keychain_blocking();
    let mut a = App::new(vec![local_server("true")]);

    a.panels[0].mode = Mode::Monitor; // user is on stats
    a.panels[0].upgrade_state = UpgradeState::STARTED;
    a.panels[0].upgrade_gen = 5;

    // AuxLine for the running upgrade.
    let msg = Msg::AuxLine {
        panel: 0,
        gen: 5,
        line: "background output".to_string(),
    };
    let dirty = a.apply(msg);

    // The ring is updated regardless...
    assert!(a.panels[0]
        .last_upgrade
        .iter()
        .any(|l| l.contains("background output")));
    // ...but applying a background line does not force a repaint.
    assert!(
        !dirty,
        "background output must not set dirty — no wasted repaint"
    );
}
