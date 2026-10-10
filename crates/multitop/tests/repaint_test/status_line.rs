//! The tool's pinned status line, as the panel holds and draws it.
//!
//! apt draws its progress bar on a row outside the scrolling log; the painter
//! reports it as `Msg::AuxStatus`, and the pane pins it to its bottom row.

use multitop::app::{App, Msg};
use multitop::config::Server;
use multitop::panel::UpgradeState;
use multitop::password_store;

fn local_server(cmd: &str) -> Server {
    Server {
        upgrade_cmd: Some(cmd.to_string()),
        ..crate::common::local_server("127.0.0.1")
    }
}

/// The mock credential store is process-global; see `applying.rs`.
async fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

fn app_mid_upgrade() -> App {
    let mut app = App::new(vec![local_server("true")]);
    app.panels[0].upgrade_state = UpgradeState::STARTED;
    app.panels[0].mode = multitop::app::Mode::Upgrade;
    app
}

/// apt's bar, as the panel draws it: the bottom row of the pane, whatever the
/// log holds, and gone once a new run starts -- keyed by the run that drew it.
#[tokio::test]
async fn the_status_line_is_pinned_to_the_bottom_row_of_the_pane() {
    let _g = isolate().await;
    let mut app = app_mid_upgrade();
    let gen = app.panels[0].upgrade_gen;
    for line in ["one", "two"] {
        app.apply(Msg::AuxLine {
            panel: 0,
            gen,
            line: line.to_string(),
        });
    }
    app.apply(Msg::AuxStatus {
        panel: 0,
        gen,
        status: Some("Progress: [ 40%]".to_string()),
    });
    let (lines, _) = multitop::ui::pane_lines(&app, 0, 12, 40, 0);
    assert_eq!(
        lines.len(),
        12,
        "the pane is filled to its height: {lines:?}"
    );
    assert_eq!(
        multitop_agent::color::strip_ansi(&lines[11]).trim_end(),
        "Progress: [ 40%]"
    );
    assert!(
        lines[..11].iter().any(|l| l.contains("two")),
        "the log is still drawn above it: {lines:?}"
    );

    // Scrolled back, the bar stays put; it is not part of the log.
    let (scrolled, _) = multitop::ui::pane_lines(&app, 0, 12, 40, 5);
    assert_eq!(
        multitop_agent::color::strip_ansi(&scrolled[11]).trim_end(),
        "Progress: [ 40%]"
    );

    // A status for another run is not this run's.
    assert!(!app.apply(Msg::AuxStatus {
        panel: 0,
        gen: gen.wrapping_add(9),
        status: Some("someone else's".to_string()),
    }));

    // A new run: the old bar is not inherited, however the run started.
    app.panels[0].upgrade_gen = gen.wrapping_add(1);
    let (fresh, _) = multitop::ui::pane_lines(&app, 0, 12, 40, 0);
    assert!(
        !fresh.iter().any(|l| l.contains("Progress")),
        "a stale run's bar was drawn: {fresh:?}"
    );

    // Taken down: no row reserved for it.
    app.panels[0].upgrade_gen = gen;
    app.apply(Msg::AuxStatus {
        panel: 0,
        gen,
        status: None,
    });
    let (down, _) = multitop::ui::pane_lines(&app, 0, 12, 40, 0);
    assert!(!down.iter().any(|l| l.contains("Progress")), "{down:?}");
}

/// The same pinned row for an ordinary view's run (journal, tail, a custom
/// command): the exec runner shares the painter, so it shares the status too,
/// keyed by the view's `gen`.
#[tokio::test]
async fn an_ordinary_views_run_pins_its_status_too() {
    let _g = isolate().await;
    let mut app = App::new(vec![local_server("true")]);
    let gen = app.panels[0].gen;
    assert!(app.apply(Msg::AuxStatus {
        panel: 0,
        gen,
        status: Some("Progress: [ 7%]".to_string()),
    }));
    let (lines, _) = multitop::ui::pane_lines(&app, 0, 8, 40, 0);
    assert_eq!(
        multitop_agent::color::strip_ansi(&lines[7]).trim_end(),
        "Progress: [ 7%]"
    );

    // The view moved on: a new gen, and the old run's bar is not drawn.
    app.panels[0].gen = gen.wrapping_add(1);
    let (moved, _) = multitop::ui::pane_lines(&app, 0, 8, 40, 0);
    assert!(!moved.iter().any(|l| l.contains("Progress")), "{moved:?}");
}
