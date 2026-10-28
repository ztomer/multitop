//! Tests for repaint behavior when rewinds and text arrive across separate I/O chunks.
//!
//! Tools like `docker compose` (`\x1b[NA\x1b[J`) and `sys_updater` / `update-local`
//! write their cursor rewinds and line clear codes in a separate write/flush from
//! the content lines of each tick. These tests ensure that bare rewinds do not emit
//! phantom blank lines, do not shift rows off-by-one, and properly blank stale rows
//! when a block shrinks.

use multitop::app::{App, Mode, Msg};
use multitop::config::Server;
use multitop::panel::UpgradeState;
use multitop::password_store;
use multitop::tasks::spawn_upgrade;
use multitop::tasks::{Paint, Painted, Painter};
use std::time::Duration;
use tokio::sync::mpsc;

fn local_server(cmd: &str) -> Server {
    Server {
        upgrade_cmd: Some(cmd.to_string()),
        ..crate::common::local_server("127.0.0.1")
    }
}

async fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

fn app_mid_upgrade() -> App {
    let mut app = App::new(vec![local_server("true")]);
    app.panels[0].upgrade_state = UpgradeState::STARTED;
    app.panels[0].mode = Mode::Upgrade;
    app
}

#[test]
fn bare_rewind_and_erase_in_separate_chunk_does_not_paint_or_corrupt_open_state() {
    let mut p = Painter::new();
    assert_eq!(p.feed_bytes(b"line 1\n").len(), 1);
    assert_eq!(p.feed_bytes(b"line 2\n").len(), 1);
    assert_eq!(p.feed_bytes(b"line 3\n").len(), 1);

    // Bare rewind and erase: \x1b[3A\x1b[J without content text or newline
    let rewound = p.feed_bytes(b"\x1b[3A\x1b[J");
    assert!(
        rewound.is_empty(),
        "bare rewind chunk must not emit a paint before content arrives, got: {rewound:?}"
    );

    // Followed by new content lines in subsequent chunks
    let c1 = p.feed_bytes(b"new 1\n");
    assert_eq!(
        c1,
        vec![Painted::Line(Paint {
            text: "new 1".to_string(),
            back: 3,
            erase_below: 3,
        })]
    );

    let c2 = p.feed_bytes(b"new 2\n");
    assert_eq!(
        c2,
        vec![Painted::Line(Paint {
            text: "new 2".to_string(),
            back: 2,
            erase_below: 0,
        })]
    );

    let c3 = p.feed_bytes(b"new 3\n");
    assert_eq!(
        c3,
        vec![Painted::Line(Paint {
            text: "new 3".to_string(),
            back: 1,
            erase_below: 0,
        })]
    );

    let finished = p.finish();
    assert_eq!(finished, [] as [Painted; 0]);
}

#[tokio::test]
async fn shrunken_block_blanks_stale_rows_below_write() {
    let _g = isolate().await;
    let mut app = app_mid_upgrade();
    let gen = app.panels[0].upgrade_gen;

    // Initial 5 rows
    for i in 1..=5 {
        app.apply(Msg::AuxLine {
            panel: 0,
            gen,
            line: format!("row {i}"),
        });
    }
    assert_eq!(app.panels[0].last_upgrade.len(), 5);

    // Block rewinds to top (back: 5) and redraws only 3 rows with erase_below: 5
    app.apply(Msg::AuxRepaint {
        panel: 0,
        gen,
        line: "shrunk 1".to_string(),
        back: 5,
        erase_below: 5,
    });
    app.apply(Msg::AuxRepaint {
        panel: 0,
        gen,
        line: "shrunk 2".to_string(),
        back: 4,
        erase_below: 0,
    });
    app.apply(Msg::AuxRepaint {
        panel: 0,
        gen,
        line: "shrunk 3".to_string(),
        back: 3,
        erase_below: 0,
    });

    let log: Vec<String> = app.panels[0].last_upgrade.iter().cloned().collect();
    assert_eq!(
        log,
        vec![
            "shrunk 1".to_string(),
            "shrunk 2".to_string(),
            "shrunk 3".to_string(),
            String::new(),
            String::new(),
        ]
    );
}

#[tokio::test]
async fn docker_compose_multi_tick_rewind_across_chunk_splits_leaves_no_duplicates() {
    let _g = isolate().await;
    // Simulates Docker Compose v2 (docker compose pull / up):
    // Outputs 3 container lines, rewinds with \033[3A\033[J in a separate write,
    // repeats for 3 ticks, and finishes.
    let script = concat!(
        "for i in 1 2 3; do ",
        r#"printf '⠿ Container c1 Step %s\n⠿ Container c2 Step %s\n⠿ Container c3 Step %s\n' "$i" "$i" "$i"; "#,
        "sleep 0.05; ",
        r#"if [ "$i" -lt 3 ]; then printf '\033[3A\033[J'; sleep 0.05; fi; "#,
        "done; ",
        "exit 0"
    );

    let (tx, mut rx) = mpsc::channel::<Msg>(512);
    let handle = spawn_upgrade(0, 1, local_server(script), None, (80, 24), tx);

    let mut app = app_mid_upgrade();
    app.panels[0].upgrade_gen = 1;
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

    let log: Vec<String> = app.panels[0].last_upgrade.iter().cloned().collect();
    let step1_count = log.iter().filter(|l| l.contains("Step 1")).count();
    let step2_count = log.iter().filter(|l| l.contains("Step 2")).count();
    let step3_count = log.iter().filter(|l| l.contains("Step 3")).count();

    assert_eq!(
        step1_count,
        0,
        "Step 1 was not completely overwritten, log:\n{}",
        log.join("\n")
    );
    assert_eq!(
        step2_count,
        0,
        "Step 2 was duplicated or not completely overwritten, log:\n{}",
        log.join("\n")
    );
    assert_eq!(
        step3_count,
        3,
        "Expected exactly 3 Step 3 lines (one per container), log:\n{}",
        log.join("\n")
    );
}

#[tokio::test]
async fn sys_updater_table_redraw_and_shrink_leaves_no_ghost_rows() {
    let _g = isolate().await;
    // Simulates sys_updater / update-local:
    // Frame 1: 4 packages updating
    // Frame 2: rewinds with \033[4A\033[J in separate write, redraws 4 packages done
    // Frame 3: rewinds with \033[4A\033[J in separate write, draws smaller 2-line summary table
    let script = concat!(
        r#"printf 'pkg-a: downloading\npkg-b: downloading\npkg-c: downloading\npkg-d: downloading\n'; "#,
        "sleep 0.05; ",
        r#"printf '\033[4A\033[J'; "#,
        "sleep 0.05; ",
        r#"printf 'pkg-a: done\npkg-b: done\npkg-c: done\npkg-d: done\n'; "#,
        "sleep 0.05; ",
        r#"printf '\033[4A\033[J'; "#,
        "sleep 0.05; ",
        r#"printf 'summary: 4 upgraded\nall clean\n'; "#,
        "exit 0"
    );

    let (tx, mut rx) = mpsc::channel::<Msg>(512);
    let handle = spawn_upgrade(0, 1, local_server(script), None, (80, 24), tx);

    let mut app = app_mid_upgrade();
    app.panels[0].upgrade_gen = 1;
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

    let log: Vec<String> = app.panels[0].last_upgrade.iter().cloned().collect();
    assert!(
        !log.iter().any(|l| l.contains("downloading")),
        "stale downloading rows remained in log:\n{}",
        log.join("\n")
    );
    assert!(
        !log.iter()
            .any(|l| l.contains("pkg-c: done") || l.contains("pkg-d: done")),
        "stale package rows remained below summary table:\n{}",
        log.join("\n")
    );
    assert!(
        log.iter().any(|l| l.contains("summary: 4 upgraded")),
        "summary table row missing from log:\n{}",
        log.join("\n")
    );
}

#[tokio::test]
async fn docker_compose_completed_lines_not_blanked_by_skipped_line_rewinds() {
    let _g = isolate().await;
    // Simulates Docker Compose pull where container c1 completes early,
    // and subsequent ticks step over container c1 with \n without redrawing it.
    let script = concat!(
        r#"printf '\033[?25l\033[0G\033[2K[+] pull 0/2\r\n\033[2K Image c1 Pulling 0.1s\r\n\033[2K Image c2 Pulling 0.1s\r\n\033[?25h'; "#,
        "sleep 0.05; ",
        // Frame 2: c1 completes to "Pulled"
        r#"printf '\033[?25l\033[3A\033[0G\r\n\033[2K Image c1 Pulled 0.2s\r\n\033[2K Image c2 Pulling 0.2s\r\n\033[?25h'; "#,
        "sleep 0.05; ",
        // Frame 3: c1 is now complete/unchanged, so Docker Compose skips it with \n
        r#"printf '\033[?25l\033[3A\033[0G\r\n\n\033[2K Image c2 Pulling 0.3s\r\n\033[?25h'; "#,
        "sleep 0.05; ",
        // Frame 4: c1 still unchanged, c2 completes
        r#"printf '\033[?25l\033[3A\033[0G\r\n\n\033[2K Image c2 Pulled 0.4s\r\n\033[?25h'; "#,
        "exit 0"
    );

    let (tx, mut rx) = mpsc::channel::<Msg>(512);
    let handle = spawn_upgrade(0, 1, local_server(script), None, (80, 24), tx);

    let mut app = app_mid_upgrade();
    app.panels[0].upgrade_gen = 1;
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

    let log: Vec<String> = app.panels[0].last_upgrade.iter().cloned().collect();
    assert!(
        log.iter().any(|l| l.contains("Image c1 Pulled 0.2s")),
        "completed container c1 was blanked out or disappeared:\n{}",
        log.join("\n")
    );
    assert!(
        log.iter().any(|l| l.contains("Image c2 Pulled 0.4s")),
        "completed container c2 missing from log:\n{}",
        log.join("\n")
    );
    assert!(
        !log.iter().any(String::is_empty),
        "log contains unexpected blank lines where completed container was wiped:\n{log:?}"
    );
}

const SYS_UPDATER_STREAM: &[u8] = include_bytes!("fixtures/sys-updater-local.bytes");

#[tokio::test]
async fn sys_updater_stream_does_not_duplicate_local_updates_title() {
    let _g = isolate().await;
    let mut p = Painter::new();
    let mut app = app_mid_upgrade();
    let gen = 1;
    app.panels[0].upgrade_gen = gen;

    for chunk in SYS_UPDATER_STREAM.chunks(512) {
        for painted in p.feed_bytes(chunk) {
            app.apply(multitop::tasks::painted_msg(
                0,
                gen,
                painted,
                std::convert::identity,
            ));
        }
    }
    for painted in p.finish() {
        app.apply(multitop::tasks::painted_msg(
            0,
            gen,
            painted,
            std::convert::identity,
        ));
    }

    let log: Vec<String> = app.panels[0].last_upgrade.iter().cloned().collect();
    let title_count = log.iter().filter(|l| l.contains("Local Updates")).count();
    assert_eq!(title_count, 1, "title duplicated! count: {title_count}");
}
