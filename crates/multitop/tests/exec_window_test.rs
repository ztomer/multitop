//! The window an upgrade's child is told it has.
//!
//! # The defect
//!
//! `ExecFrame::Request` carries `cols` and `rows`, and their own documentation
//! says why: *"A pty with a plausible size is what stops `apt` deciding it has
//! 80 columns on a 200-column panel."* Every remote channel was handed the
//! live pane size -- Monitor, Docker, Fetch, Ops -- and the two exec callers
//! that draw into that same pane passed a literal:
//!
//! ```text
//! cols: 80,
//! rows: 24,
//! ```
//!
//! So the field existed to carry a number, was filled in with the one number it
//! exists to prevent, and nothing failed. Measured on a live host over a
//! multiplexed SSH connection, on a 199-column terminal, the upgrade log's first
//! line was `24 80` -- `stty size`, asked inside the run, answering with a
//! window nobody was reading. `apt` laid its progress bar and its "After this
//! operation" table out for eighty columns, and the pane drew the result into
//! two hundred.
//!
//! The test below asks the question the way the defect was found: from inside
//! the run, with `stty size`, against a width nobody would have typed.
//!
//! Why it slipped past every other suite: the ones that check the window check
//! that the *mechanism* works. `tests/test_exec_live.py::test_06_the_window_size
//! _reaches_the_command` passes `cols=203, rows=51` through the wire and asserts
//! `51 203` comes back -- correctly, because it is the one that supplies the
//! number. Nothing asked whether the product supplies one.

#![cfg(test)]

mod common;

use std::time::Duration;

use multitop::app::{App, Mode, Msg};
use multitop::panel::UpgradeState;
use multitop::password_store;
use multitop::tasks::spawn_upgrade;
use tokio::sync::mpsc;

/// The window this test asks for. Deliberately neither 80 nor a round number
/// a constant might have been rounded to: a width only a pane could have
/// produced.
const PANE: (u16, u16) = (203, 51);

fn server() -> multitop::config::Server {
    multitop::config::Server {
        upgrade_cmd: Some("stty size".to_string()),
        ..common::local_server("127.0.0.1")
    }
}

/// The mock credential store is process-global, so every test that reaches it
/// holds the same guard for its whole body.
async fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

/// Run one upgrade to completion and return what its panel's log holds.
async fn run_upgrade_log(dims: (u16, u16), command: &str) -> Vec<String> {
    let mut app = App::new(vec![multitop::config::Server {
        upgrade_cmd: Some(command.to_string()),
        ..server()
    }]);
    app.panels[0].upgrade_state = UpgradeState::STARTED;
    app.panels[0].mode = Mode::Upgrade;
    let gen = app.panels[0].upgrade_gen;

    let (tx, mut rx) = mpsc::channel::<Msg>(512);
    let handle = spawn_upgrade(0, gen, server(), None, dims, tx);
    let collect = async {
        while let Some(msg) = rx.recv().await {
            let done = matches!(msg, Msg::AuxDone { .. });
            app.apply(msg);
            if done {
                break;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(60), collect)
        .await
        .expect("the run must finish");
    handle.abort();
    app.panels[0].last_upgrade.iter().cloned().collect()
}

/// The property, asked of the product's own path rather than of the wire.
///
/// A 200-column pane told the child it had 80, and every assertion in the suite
/// passed, because every one of them was about the mechanism.
#[tokio::test]
async fn the_child_is_told_the_size_of_the_pane_its_output_is_drawn_into() {
    let _g = isolate().await;
    let log = run_upgrade_log(PANE, "stty size").await.join("\n");
    assert!(
        log.contains("51 203"),
        "the upgrade's pty did not get the pane's window; the log said:\n{log}\n\
         (`stty size` prints `rows cols`, so a pane of {PANE:?} reads as `51 203`)"
    );
}

/// The same question at a width the constant could not have been mistaken for,
/// in the other direction. A pane narrower than 80 is the case where the old
/// behaviour was not merely ugly: `apt` laid out a table for 80 columns and the
/// pane clipped it, so the right-hand column of every row was simply gone.
#[tokio::test]
async fn a_narrow_pane_is_not_given_eighty_columns() {
    let _g = isolate().await;
    let log = run_upgrade_log((47, 11), "stty size").await.join("\n");
    assert!(
        log.contains("11 47"),
        "a 47-column pane was told something else:\n{log}"
    );
}

/// The width also has to reach the tool that decides its own layout, not just
/// `stty`. `apt` reads `COLUMNS` off the terminal, and a progress bar is what
/// an operator watches for the length of an upgrade -- so the bar has to be
/// built for the pane it is drawn in.
#[tokio::test]
async fn a_tool_that_lays_itself_out_is_told_the_panes_width() {
    let _g = isolate().await;
    // `tput cols` is the shell's own answer for the terminal width, which is
    // what a progress bar measures itself against. No `tput` on a bare host is
    // a skip with a stated reason, never a pass.
    let log = run_upgrade_log(PANE, "tput cols 2>/dev/null || echo NO_TPUT")
        .await
        .join("\n");
    if log.contains("NO_TPUT") {
        eprintln!("skipping: no tput on this host, so the width cannot be asked of it");
        return;
    }
    assert!(
        log.contains("203"),
        "the shell reported a width that is not the pane's:\n{log}"
    );
}

/// One width, one answer, whatever the pane happens to be: the number is the
/// pane's and the pane's alone. Two different widths producing two different
/// answers is the whole proof that the value is threaded rather than typed.
#[tokio::test]
async fn two_panes_two_answers() {
    let _g = isolate().await;
    let wide = run_upgrade_log((203, 51), "stty size").await.join("\n");
    let narrow = run_upgrade_log((61, 9), "stty size").await.join("\n");
    assert!(wide.contains("51 203"), "{wide}");
    assert!(narrow.contains("9 61"), "{narrow}");
}
