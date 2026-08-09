//! Output that repaints several lines in place.
//!
//! `apt` and `curl` rewrite one line with carriage returns, and that has been
//! handled for a while. `docker compose pull` prints a block, moves the cursor
//! up over it with `ESC[nA`, and prints the block again — several times a
//! second. Treated as new lines, a pull of five layers buries the whole run in
//! its own progress display.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use multitop::app::{App, Msg};
use multitop::config::Server;
use multitop::panel::{RingLines, UpgradeState};
use multitop::password_store;
use multitop::tasks::{spawn_upgrade, Painter};
use tokio::sync::mpsc;

fn local_server(cmd: &str) -> Server {
    Server {
        host: "localhost".to_string(),
        port: 0,
        user: String::new(),
        upgrade_cmd: Some(cmd.to_string()),
    }
}

async fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

// ----------------------------------------------------------- the screen model

#[test]
fn ordinary_lines_simply_follow_one_another() {
    let mut p = Painter::new();
    for text in ["first", "second", "third"] {
        let paint = p.feed(text).expect("a line with text is shown");
        assert_eq!(paint.text, text);
        assert_eq!(paint.back, 0, "{text} was placed as a repaint");
        assert_eq!(paint.erase_below, 0);
    }
}

#[test]
fn a_cursor_up_places_the_next_line_over_one_already_shown() {
    let mut p = Painter::new();
    p.feed("layer one: pulling").unwrap();
    p.feed("layer two: pulling").unwrap();

    // Up two, then the block again — which is what a repainting tool does.
    let first = p.feed("\u{1b}[2Alayer one: extracting").unwrap();
    assert_eq!(first.text, "layer one: extracting");
    assert_eq!(first.back, 2, "the repaint did not land on the first line");

    let second = p.feed("layer two: extracting").unwrap();
    assert_eq!(second.text, "layer two: extracting");
    assert_eq!(second.back, 1, "the block was not walked down in order");

    // And the tick after that lands in the same two places.
    assert_eq!(p.feed("\u{1b}[2Alayer one: done").unwrap().back, 2);
    assert_eq!(p.feed("layer two: done").unwrap().back, 1);
}

#[test]
fn a_bare_cursor_up_carries_to_the_next_line_that_has_text() {
    // Tools split the movement and the text across writes; the reader hands
    // them over as separate lines and the movement must not be lost.
    let mut p = Painter::new();
    p.feed("one").unwrap();
    p.feed("two").unwrap();

    assert!(
        p.feed("\u{1b}[2A").is_none(),
        "movement alone shows nothing"
    );
    assert_eq!(p.feed("rewritten").unwrap().back, 2);
}

#[test]
fn a_missing_count_means_one_row() {
    let mut p = Painter::new();
    p.feed("one").unwrap();
    assert_eq!(p.feed("\u{1b}[Aover it").unwrap().back, 1);
}

#[test]
fn moving_back_down_returns_to_the_end() {
    let mut p = Painter::new();
    p.feed("one").unwrap();
    p.feed("two").unwrap();
    p.feed("three").unwrap();

    assert!(p.feed("\u{1b}[3A").is_none());
    // Straight back down: the next line appends rather than overwriting.
    assert_eq!(p.feed("\u{1b}[3Bappended").unwrap().back, 0);
}

#[test]
fn moving_up_past_the_top_stops_at_the_top() {
    // A tool that has been running longer than the log is a real case once the
    // ring has rolled; the count must not wrap.
    let mut p = Painter::new();
    p.feed("one").unwrap();
    let paint = p.feed("\u{1b}[99Aover the top").unwrap();
    assert_eq!(paint.back, 99, "the count was mangled on the way through");
}

#[test]
fn erasing_downward_is_reported_so_the_stale_rows_can_be_blanked() {
    let mut p = Painter::new();
    p.feed("one").unwrap();
    p.feed("two").unwrap();
    let paint = p.feed("\u{1b}[2A\u{1b}[Jshorter block").unwrap();
    assert_eq!(paint.back, 2);
    assert_eq!(paint.erase_below, 1, "the erase was dropped");
}

#[test]
fn erasing_a_line_needs_no_reporting_because_the_write_replaces_it() {
    let mut p = Painter::new();
    p.feed("one").unwrap();
    let paint = p.feed("\u{1b}[A\u{1b}[2Kfresh").unwrap();
    assert_eq!(paint.text, "fresh");
    assert_eq!(paint.back, 1);
    assert_eq!(paint.erase_below, 0);
}

#[test]
fn carriage_returns_still_collapse_within_the_line_being_painted() {
    // The two mechanisms compose: a tool may move the cursor up *and* rewrite
    // the line it lands on with carriage returns.
    let mut p = Painter::new();
    p.feed("one").unwrap();
    let paint = p.feed("\u{1b}[A10%\r55%\r100%").unwrap();
    assert_eq!(paint.text, "100%", "only the state it ended on is shown");
    assert_eq!(paint.back, 1);
}

#[test]
fn a_sequence_this_model_does_not_know_is_left_in_the_text() {
    // Colour is not cursor movement: `ansi.rs` renders it, so it must survive.
    let mut p = Painter::new();
    let paint = p.feed("\u{1b}[31mred text").unwrap();
    assert_eq!(paint.text, "\u{1b}[31mred text");
    assert_eq!(paint.back, 0);
}

// ------------------------------------------------------------------- the ring

#[test]
fn the_ring_can_overwrite_a_line_by_how_far_back_it_is() {
    let mut ring = RingLines::new(8);
    for text in ["a", "b", "c"] {
        ring.push(text.to_string());
    }

    ring.overwrite_from_end(0, "c2");
    ring.overwrite_from_end(2, "a2");
    assert_eq!(
        ring.iter().cloned().collect::<Vec<_>>(),
        vec!["a2".to_string(), "b".to_string(), "c2".to_string()]
    );

    // Past the oldest line is a no-op, not a panic and not a wrap onto some
    // other row.
    ring.overwrite_from_end(3, "nope");
    ring.overwrite_from_end(usize::MAX, "nope");
    assert!(!ring.iter().any(|l| l == "nope"));
}

#[test]
fn overwriting_still_addresses_the_newest_after_the_ring_has_rolled() {
    // Once full, the oldest slot is reused in place and the physical order no
    // longer matches the logical one. Addressing by "back from newest" has to
    // survive that.
    let mut ring = RingLines::new(3);
    for text in ["a", "b", "c", "d", "e"] {
        ring.push(text.to_string());
    }
    assert_eq!(
        ring.iter().cloned().collect::<Vec<_>>(),
        vec!["c".to_string(), "d".to_string(), "e".to_string()]
    );

    ring.overwrite_from_end(0, "e2");
    ring.overwrite_from_end(2, "c2");
    assert_eq!(
        ring.iter().cloned().collect::<Vec<_>>(),
        vec!["c2".to_string(), "d".to_string(), "e2".to_string()]
    );
}

// --------------------------------------------------------------- through the app

fn app_mid_upgrade() -> App {
    let mut app = App::new(vec![local_server("true")]);
    app.panels[0].upgrade_state = UpgradeState::STARTED;
    app.panels[0].mode = multitop::app::Mode::Upgrade;
    app
}

#[tokio::test]
async fn a_repaint_replaces_the_line_it_addresses() {
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
    app.apply(Msg::AuxRepaint {
        panel: 0,
        gen,
        line: "one again".to_string(),
        back: 2,
        erase_below: 0,
    });

    assert_eq!(
        app.panels[0]
            .last_upgrade
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["one again".to_string(), "two".to_string()],
        "the repaint appended instead of replacing"
    );
}

#[tokio::test]
async fn a_repaint_that_erases_below_blanks_the_rows_it_covered() {
    let _g = isolate().await;
    let mut app = app_mid_upgrade();
    let gen = app.panels[0].upgrade_gen;

    for line in ["one", "two", "three"] {
        app.apply(Msg::AuxLine {
            panel: 0,
            gen,
            line: line.to_string(),
        });
    }
    // The block shrank: rewrite the top and erase what was under it.
    app.apply(Msg::AuxRepaint {
        panel: 0,
        gen,
        line: "only line".to_string(),
        back: 3,
        erase_below: 2,
    });

    let log: Vec<String> = app.panels[0].last_upgrade.iter().cloned().collect();
    assert_eq!(log[0], "only line");
    assert_eq!(log[1], "", "a row from the previous frame was left showing");
    assert_eq!(log[2], "");
}

#[tokio::test]
async fn a_repaint_reaching_past_the_log_appends_rather_than_dropping_output() {
    // The ring has rolled and the block the tool is addressing is gone. Losing
    // the line silently would be the worse answer.
    let _g = isolate().await;
    let mut app = app_mid_upgrade();
    let gen = app.panels[0].upgrade_gen;
    app.apply(Msg::AuxLine {
        panel: 0,
        gen,
        line: "only".to_string(),
    });

    app.apply(Msg::AuxRepaint {
        panel: 0,
        gen,
        line: "way back".to_string(),
        back: 50,
        erase_below: 0,
    });

    let log: Vec<String> = app.panels[0].last_upgrade.iter().cloned().collect();
    assert_eq!(log, vec!["only".to_string(), "way back".to_string()]);
}

#[tokio::test]
async fn a_repaint_for_a_run_that_is_not_this_panels_is_dropped() {
    let _g = isolate().await;
    let mut app = app_mid_upgrade();
    let gen = app.panels[0].upgrade_gen;
    app.apply(Msg::AuxLine {
        panel: 0,
        gen,
        line: "mine".to_string(),
    });

    assert!(!app.apply(Msg::AuxRepaint {
        panel: 0,
        gen: gen.wrapping_add(9),
        line: "someone else's".to_string(),
        back: 0,
        erase_below: 0,
    }));
    assert!(!app.apply(Msg::AuxRepaint {
        panel: 99,
        gen,
        line: "no such panel".to_string(),
        back: 0,
        erase_below: 0,
    }));
    assert_eq!(app.panels[0].last_upgrade.last().unwrap(), "mine");
}

// ------------------------------------------------------------------ end to end

#[tokio::test]
async fn a_block_repainted_ten_times_stays_one_block_in_the_log() {
    // The complaint, in the shape it arrives: a two-line block, rewritten with
    // a cursor-up between each pass. Ten passes used to be twenty lines.
    let _g = isolate().await;
    // Ten passes over a two-line block: print both lines, then move back up
    // over them. No inner quotes, so the whole thing survives `sh_quote`.
    let script = concat!(
        "i=1; while [ $i -le 10 ]; do ",
        r"printf 'layer one: step %s\n' $i; ",
        r"printf 'layer two: step %s\n' $i; ",
        r"printf '\033[2A'; ",
        "i=$((i+1)); done; ",
        r"printf '\033[2B'; exit 0"
    );

    let (tx, mut rx) = mpsc::channel::<Msg>(512);
    let handle = spawn_upgrade(0, 1, local_server(script), None, tx);

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
    let steps = log.iter().filter(|l| l.contains("layer one: step")).count();
    assert_eq!(
        steps,
        1,
        "ten repaints of one block left {steps} copies of it:\n{}",
        log.join("\n")
    );
    assert!(
        log.iter().any(|l| l.contains("layer one: step 10")),
        "the log kept an earlier frame instead of the last one:\n{}",
        log.join("\n")
    );
}

#[tokio::test]
async fn output_that_never_moves_the_cursor_is_untouched_by_any_of_this() {
    // The common case has to stay exactly as it was.
    let _g = isolate().await;
    let (tx, mut rx) = mpsc::channel::<Msg>(256);
    let handle = spawn_upgrade(
        0,
        1,
        local_server("printf 'alpha\\nbeta\\ngamma\\n'; exit 0"),
        None,
        tx,
    );

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

    let log = app.panels[0]
        .last_upgrade
        .iter()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    for expected in ["alpha", "beta", "gamma"] {
        assert!(log.contains(expected), "{expected} missing from:\n{log}");
    }
}
