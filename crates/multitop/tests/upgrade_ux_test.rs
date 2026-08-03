//! The two-press `u` flow.
//!
//! Pressing `u` once switches into the Upgrade view and does nothing else;
//! pressing it again starts the run. These tests drive the real key handler,
//! because the property being protected is a *sequence* — testing the `App`
//! methods individually would not catch the sequence regressing.
//!
//! The rule this pins down: the behaviour of the first press must not depend on
//! whether an upgrade has ever run. It used to, which is how `u` could jump
//! straight to the confirm modal on a fresh start but show the pane once an
//! upgrade had happened.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::sync::{mpsc, watch};

use multitop::app::{App, Mode, Msg};
use multitop::config::Server;
use multitop::run::{handle_key, Tasks};
use multitop::state::HostUpdate;

/// Divert credentials to the in-memory store, and hold the process-global guard.
///
/// An integration binary is compiled without `cfg(test)`, so the mock store is
/// not in force unless it is asked for, and anything holding an `App` reaches
/// `password_store` several calls down. Without this these tests query the real
/// OS keychain: every rebuild changes the binary's code signature, so macOS
/// raises an access dialog and the suite stops until a human dismisses it.
#[allow(dead_code)]
fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test();
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

#[allow(dead_code)]
async fn isolate_keychain_async() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test_async().await;
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

fn server(host: &str, cmd: Option<&str>) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: cmd.map(str::to_string),
    }
}

struct Harness {
    app: App,
    servers: Vec<Server>,
    tasks: Tasks,
    tx: mpsc::Sender<Msg>,
    /// Messages the key handler emitted, so a test can assert that a press
    /// which should be inert really did not queue any work.
    rx: mpsc::Receiver<Msg>,
    dims_rx: Arc<watch::Receiver<(u16, u16)>>,
}

impl Harness {
    fn new(servers: Vec<Server>) -> Self {
        let app = App::new(servers.clone());
        let (tx, rx) = mpsc::channel::<Msg>(64);
        let (dims_tx, drx) = watch::channel((80u16, 24u16));
        // The receiver keeps working after the sender goes; nothing here resizes.
        drop(dims_tx);
        Self {
            tasks: Tasks::new(servers.len()),
            app,
            servers,
            tx,
            rx,
            dims_rx: Arc::new(drx),
        }
    }

    fn press(&mut self, c: char) {
        handle_key(
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: crossterm::event::KeyEventState::NONE,
            },
            &mut self.app,
            &self.servers,
            (80, 24),
            Arc::clone(&self.dims_rx),
            &self.tx,
            &mut self.tasks,
        );
    }

    /// Messages emitted so far, without blocking.
    fn emitted(&mut self) -> Vec<Msg> {
        let mut out = Vec::new();
        while let Ok(m) = self.rx.try_recv() {
            out.push(m);
        }
        out
    }

    fn pane_text(&self, panel: usize) -> String {
        strip_ansi(&self.app.panels[panel].view.join("\n"))
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c2 in chars.by_ref() {
                if c2 == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 1. First press switches to the pane, and starts nothing.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn first_press_switches_to_the_upgrade_pane() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    assert_eq!(h.app.panels[0].mode, Mode::Monitor);

    h.press('u');

    assert_eq!(h.app.panels[0].mode, Mode::Upgrade);
    assert!(
        !h.app.show_upgrade_modal(),
        "the first press must not open the confirm modal"
    );
    assert!(
        !h.app.upgrades_in_flight(),
        "the first press must not start an upgrade"
    );
    assert!(
        h.emitted().is_empty(),
        "the first press must not queue any work"
    );
}

/// The regression that motivated this work: the first press used to behave
/// differently depending on whether an upgrade had run before.
#[tokio::test]
async fn first_press_is_the_same_before_and_after_an_upgrade_has_run() {
    let _keychain = isolate_keychain_async().await;
    let mut fresh = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    fresh.press('u');
    let fresh_mode = fresh.app.panels[0].mode;
    let fresh_modal = fresh.app.show_upgrade_modal();

    let mut used = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    used.app.panels[0].upgrade_state = multitop::panel::UpgradeState::DONE;
    used.app.panels[0].last_upgrade = vec!["previous output".to_string()];
    used.press('u');

    assert_eq!(
        fresh_mode, used.app.panels[0].mode,
        "first press must land in the same view either way"
    );
    assert_eq!(
        fresh_modal,
        used.app.show_upgrade_modal(),
        "first press must never open the modal, upgraded before or not"
    );
}

#[tokio::test]
async fn first_press_reaches_the_pane_from_every_other_view() {
    let _keychain = isolate_keychain_async().await;
    for entry in ['d', 'f', 's'] {
        let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
        h.press(entry);
        h.press('u');
        assert_eq!(
            h.app.panels[0].mode,
            Mode::Upgrade,
            "u from '{entry}' must reach the Upgrade pane"
        );
        assert!(!h.app.show_upgrade_modal(), "from '{entry}'");
    }
}

// ---------------------------------------------------------------------------
// 2. The pane tells the user what they need to decide.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pane_shows_the_command_and_history_for_each_host() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![
        server("web-01", Some("apt update && apt upgrade -y")),
        server("db-02", Some("dnf upgrade -y")),
    ]);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    h.app.host_updates.insert(
        multitop::password_store::account(&h.servers[0]),
        HostUpdate {
            started_at: Some(now - 86_400 * 3 - 30),
            finished_at: Some(now - 86_400 * 3),
            success: true,
        },
    );

    h.press('u');

    // The host name itself comes from the panel banner that `ui::draw` writes
    // over view[0], not from the pane body — see `upgrade_view::header`. What
    // the body must get right is the per-host detail.
    let web = h.pane_text(0);
    assert!(web.contains("apt update && apt upgrade -y"), "{web}");
    assert!(web.contains("3 days ago"), "{web}");
    assert!(web.contains("ok"), "{web}");

    // Each pane shows its OWN host, not a shared summary.
    let db = h.pane_text(1);
    assert!(db.contains("dnf upgrade -y"), "{db}");
    assert!(db.contains("never"), "db-02 has no history: {db}");
    assert!(
        !db.contains("apt update"),
        "panes must not leak each other's commands: {db}"
    );
}

#[tokio::test]
async fn pane_explains_a_host_with_no_upgrade_cmd() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", None)]);
    h.press('u');

    let text = h.pane_text(0);
    assert!(text.contains("not configured"), "{text}");
    assert!(text.contains("host is skipped"), "{text}");
    assert!(
        text.contains("set upgrade_cmd in config.toml"),
        "must show how to fix it: {text}"
    );
}

#[tokio::test]
async fn pane_warns_about_an_interrupted_previous_run() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    h.app.host_updates.insert(
        multitop::password_store::account(&h.servers[0]),
        HostUpdate {
            started_at: Some(now - 3600),
            finished_at: None,
            success: false,
        },
    );

    h.press('u');

    let text = h.pane_text(0);
    assert!(text.contains("interrupted"), "{text}");
    assert!(text.contains("never finished"), "{text}");
}

// ---------------------------------------------------------------------------
// 3. Second press starts the run.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn second_press_opens_the_confirm_modal() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    h.press('u');
    assert!(!h.app.show_upgrade_modal());

    h.press('u');
    assert!(
        h.app.show_upgrade_modal(),
        "the second press must ask for confirmation"
    );
    assert!(
        !h.app.upgrades_in_flight(),
        "still nothing running until the modal is confirmed"
    );
}

#[tokio::test]
async fn confirming_after_two_presses_starts_the_upgrade() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    h.press('u');
    h.press('u');
    h.press('y');

    assert!(!h.app.show_upgrade_modal(), "modal closes on confirm");
    assert!(
        h.app.upgrades_in_flight(),
        "confirming must actually start the upgrade"
    );
    assert!(
        h.app.upgrade_started_at.is_some(),
        "the start time is recorded so an interrupted run can be detected"
    );
}

#[tokio::test]
async fn second_press_with_nothing_configured_does_not_open_a_modal() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", None), server("db-02", None)]);
    h.press('u');
    h.press('u');

    assert!(
        !h.app.show_upgrade_modal(),
        "a modal whose only outcome is skipping every host is not worth showing"
    );
    let text = h.pane_text(0);
    assert!(text.contains("nothing to run"), "{text}");
}

#[tokio::test]
async fn presses_are_ignored_while_an_upgrade_is_running() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    h.press('u');
    h.press('u');
    h.press('y');
    assert!(h.app.upgrades_in_flight());

    h.press('u');
    assert!(
        !h.app.show_upgrade_modal(),
        "u must not re-arm an upgrade that is already running"
    );
}

// ---------------------------------------------------------------------------
// 4. The flow is reversible and repeatable.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn s_leaves_the_pane_and_u_returns_to_it() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    h.press('u');
    assert_eq!(h.app.panels[0].mode, Mode::Upgrade);

    h.press('s');
    assert_eq!(h.app.panels[0].mode, Mode::Monitor);

    // Back in, and still only arming on the second press.
    h.press('u');
    assert_eq!(h.app.panels[0].mode, Mode::Upgrade);
    assert!(
        !h.app.show_upgrade_modal(),
        "re-entering the pane must not skip straight to the modal"
    );
}

// ---------------------------------------------------------------------------
// 5. Switching views mid-run. Reported from live use: after switching to stats
//    during an upgrade on one host, `u` would not come back, and the host that
//    finished while away lost its completion marker.
// ---------------------------------------------------------------------------

/// The upgrade generation the panel is currently running.
fn upgrade_gen(h: &Harness, panel: usize) -> u64 {
    h.app.panels[panel].upgrade_gen
}

fn start_upgrade(h: &mut Harness) {
    h.press('u');
    h.press('u');
    h.press('y');
    assert!(h.app.upgrades_in_flight(), "precondition: upgrade started");
}

#[tokio::test]
async fn can_return_to_the_pane_while_an_upgrade_is_running() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);

    h.press('s');
    assert_eq!(h.app.panels[0].mode, Mode::Monitor);
    assert!(
        h.app.upgrades_in_flight(),
        "leaving the view must not cancel the run"
    );

    h.press('u');
    assert_eq!(
        h.app.panels[0].mode,
        Mode::Upgrade,
        "u must return to the pane while the upgrade is still running"
    );
}

#[tokio::test]
async fn output_produced_while_away_is_shown_on_return() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);

    h.app.apply(Msg::AuxLine {
        panel: 0,
        gen: g,
        line: "while-watching".into(),
    });
    h.press('s');
    h.app.apply(Msg::AuxLine {
        panel: 0,
        gen: g,
        line: "while-away".into(),
    });

    // Upgrade output must not leak into the stats view.
    assert!(
        !h.pane_text(0).contains("while-away"),
        "stats view must not collect upgrade output: {}",
        h.pane_text(0)
    );

    h.press('u');
    let text = h.pane_text(0);
    assert!(text.contains("while-watching"), "{text}");
    assert!(text.contains("while-away"), "{text}");
}

#[tokio::test]
async fn output_keeps_streaming_after_returning_to_the_pane() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);

    h.press('s');
    h.press('u');
    h.app.apply(Msg::AuxLine {
        panel: 0,
        gen: g,
        line: "after-return".into(),
    });

    assert!(
        h.pane_text(0).contains("after-return"),
        "the pane must keep updating after switching back: {}",
        h.pane_text(0)
    );
}

/// The reported symptom: the host that finished while the user was on the
/// stats view showed no completion marker when they came back.
#[tokio::test]
async fn completion_marker_survives_being_away_when_it_arrives() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);

    h.press('s');
    h.app.apply(Msg::AuxDone {
        panel: 0,
        gen: g,
        note: Some("-done".into()),
        success: true,
    });

    assert!(!h.app.upgrades_in_flight(), "the run completed while away");
    assert!(
        !h.pane_text(0).contains("-done"),
        "the marker must not be dumped into the stats view"
    );

    h.press('u');
    let text = h.pane_text(0);
    assert!(
        text.contains("-done"),
        "completion marker must be there on return: {text}"
    );
    assert!(text.contains("ok"), "and the status must read ok: {text}");
}

#[tokio::test]
async fn returning_after_completion_shows_the_finished_state() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);
    h.app.apply(Msg::AuxDone {
        panel: 0,
        gen: g,
        note: Some("-done".into()),
        success: true,
    });

    let text = h.pane_text(0);
    assert!(
        !text.contains("do not quit"),
        "a finished run must stop saying it is running: {text}"
    );
}

// ---------------------------------------------------------------------------
// 6. Reported from a live four-host run: panels showed nothing but
//    "sudo ready", a failing command was blamed on the network, and the status
//    block vanished the moment output arrived.
// ---------------------------------------------------------------------------

/// `Msg::Status` used to assign `view = vec![text]`, throwing away the status
/// header and every line of output collected so far.
#[tokio::test]
async fn a_status_note_does_not_wipe_the_upgrade_pane() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);

    h.app.apply(Msg::AuxLine {
        panel: 0,
        gen: g,
        line: "Reading package lists...".into(),
    });
    h.app.apply(Msg::Status {
        panel: 0,
        gen: g,
        text: "sudo ready - already authorized".into(),
    });

    let text = h.pane_text(0);
    assert!(
        text.contains("apt upgrade"),
        "the status header must survive a status note: {text}"
    );
    assert!(
        text.contains("Reading package lists"),
        "output collected so far must survive a status note: {text}"
    );
    assert!(
        text.contains("sudo ready"),
        "and the note itself belongs in the log: {text}"
    );
}

/// The note must also survive being away, like any other upgrade output.
#[tokio::test]
async fn a_status_note_arriving_while_away_is_kept() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);

    h.app.apply(Msg::Status {
        panel: 0,
        gen: g,
        text: "sudo ready - already authorized".into(),
    });
    h.press('s');
    assert!(
        !h.pane_text(0).contains("sudo ready"),
        "must not be dumped into the stats view"
    );
    h.press('u');
    assert!(h.pane_text(0).contains("sudo ready"), "{}", h.pane_text(0));
}

/// A non-zero exit is a failed command, not a lost connection. Reporting it as
/// "disconnected" pointed at the network for a host the stats view was happily
/// streaming from at that moment.
#[tokio::test]
async fn a_failing_command_is_not_reported_as_a_disconnect() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("./update_sys.sh"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);

    h.app.apply(Msg::AuxDone {
        panel: 0,
        gen: g,
        note: Some("\u{26A0} upgrade command exited 2 - host reachable, command failed".into()),
        success: false,
    });

    let text = h.pane_text(0);
    assert!(text.contains("exited 2"), "must give the exit code: {text}");
    assert!(
        text.contains("host reachable"),
        "must not blame the connection: {text}"
    );
    assert!(
        text.contains("last run failed"),
        "and the badge must say failed: {text}"
    );
}

/// The status block is the point of the pane, so it must not scroll away the
/// moment output starts arriving.
#[tokio::test]
async fn the_status_block_stays_pinned_under_heavy_output() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);
    for i in 0..200 {
        h.app.apply(Msg::AuxLine {
            panel: 0,
            gen: g,
            line: format!("line {i}"),
        });
    }

    // What the renderer would actually show in a 20-row panel.
    let (shown, _) = multitop::ui::visible(
        &h.app.panels[0].view,
        20,
        h.app.panels[0].pinned_lines.max(1),
        0,
        0,
    );
    let text = strip_ansi(&shown.join("\n"));
    assert!(
        text.contains("Command"),
        "the command must still be visible under 200 lines of output: {text}"
    );
    assert!(
        text.contains("line 199"),
        "and the newest output must still be the tail: {text}"
    );
}

/// A panel whose upgrade never reports back stays "running" for the rest of the
/// session and blocks every later upgrade, because `upgrades_in_flight()` never
/// clears. That is what a failed SSH spawn used to do: it sent a status line and
/// returned, with no `AuxDone`.
#[tokio::test]
async fn a_panel_that_cannot_start_still_reaches_a_terminal_state() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);

    // What the task now emits when ssh::spawn_command fails.
    h.app.apply(Msg::AuxLine {
        panel: 0,
        gen: g,
        line: "ssh: could not resolve hostname".into(),
    });
    h.app.apply(Msg::AuxDone {
        panel: 0,
        gen: g,
        note: Some("\u{26A0} could not start the upgrade over SSH".into()),
        success: false,
    });

    assert!(
        !h.app.upgrades_in_flight(),
        "a panel that could not start must not block every later upgrade"
    );
    let text = h.pane_text(0);
    assert!(
        !text.contains("do not quit"),
        "and it must stop claiming to be running: {text}"
    );
    assert!(text.contains("could not start"), "{text}");

    // The user can immediately try again.
    h.press('u');
    assert!(
        h.app.show_upgrade_modal(),
        "u must be able to arm another attempt"
    );
}

/// `AuxBegin` arrives immediately after every upgrade starts. It used to
/// replace the whole view, so the status header was destroyed on every single
/// run before a byte of output appeared -- leaving a bare "Upgrade on <host>"
/// line that the panel banner then overwrote, which is why panels showed
/// output with no header at all.
#[tokio::test]
async fn the_pane_survives_the_aux_begin_that_every_run_sends() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);

    h.app.apply(Msg::AuxBegin {
        panel: 0,
        gen: g,
        header: Some("Upgrade on web-01".into()),
    });

    let text = h.pane_text(0);
    assert!(
        text.contains("apt upgrade"),
        "the status header must survive AuxBegin: {text}"
    );
    assert!(
        text.contains("running"),
        "and still show the running state: {text}"
    );
}

/// Reported from live use: the pane said "will prompt" for a host whose
/// password had been saved. Passwords load lazily, so a panel that has not run
/// an upgrade yet holds nothing in memory, and the pane read that emptiness as
/// "no password" instead of asking the store.
#[tokio::test]
async fn the_pane_reports_a_saved_password_rather_than_promising_a_prompt() {
    let _guard = multitop::password_store::lock_for_test_async().await;
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();

    let s = server("web-01", Some("apt upgrade"));
    multitop::password_store::save(&s, "stored-secret").unwrap();

    let mut h = Harness::new(vec![s]);
    // Nothing loaded yet: exactly the state a fresh session is in.
    assert!(h.app.panels[0].sudo_password.is_none());

    h.press('u');

    let text = h.pane_text(0);
    assert!(
        text.contains("password stored"),
        "a saved password must be reported as stored: {text}"
    );
    assert!(
        !text.contains("will prompt"),
        "and must not threaten a prompt that will not happen: {text}"
    );
}
