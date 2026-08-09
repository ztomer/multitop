//! A filtered grid tells the agents how big its panes really are.
//!
//! Two numbers had to be the same number and were not. `ui::draw` splits the
//! screen by `filtered_indices()`; the render size handed to every agent was
//! derived from `panels.len()`. Filter four hosts down to one and the pane
//! became the whole screen while the frame drawn into it stayed a quarter of
//! it -- a small picture in a big box, with no error anywhere to explain it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use multitop::app::{App, Msg};
use multitop::config::Server;
use multitop::password_store;
use multitop::run::{handle_key, Tasks};
use multitop::ui::{agent_dims, regions, MIN_AGENT_COLS, MIN_AGENT_ROWS};
use ratatui::backend::TestBackend;
use ratatui::layout::{Rect, Size};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_stream::StreamExt as _;

const SIDE_MARGIN: u16 = 1;

static PORT_COUNTER: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(44000);

/// `.example` is reserved by RFC 2606 and no resolver answers it, so a monitor
/// task spawned by the real event loop cannot reach anything.
fn test_server(host: &str) -> Server {
    Server {
        host: format!("{host}.example"),
        port: PORT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        user: "admin".to_string(),
        upgrade_cmd: Some("true".to_string()),
    }
}

async fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

fn press(app: &mut App, code: KeyCode, tx: &mpsc::Sender<Msg>, tasks: &mut Tasks) {
    let (dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    std::mem::forget(dims_tx);
    handle_key(
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Press),
        app,
        (80, 24),
        Arc::new(dims_rx),
        tx,
        tasks,
    );
}

const fn size(w: u16, h: u16) -> Size {
    Size {
        width: w,
        height: h,
    }
}

// ------------------------------------------------------- the invariant itself

#[test]
fn the_render_size_is_the_smallest_pane_the_grid_hands_out() {
    // The class-level pin. `agent_dims` and `regions` were two independent
    // implementations of one grid; this asserts they cannot disagree, at every
    // pane count and terminal size that fits on a desk.
    for panels in 1..=12usize {
        for (w, h) in [(80, 24), (120, 40), (200, 60), (81, 25), (137, 41)] {
            let (cols, rows) = agent_dims(size(w, h), panels);
            let (panes, _) = regions(Rect::new(0, 0, w, h), panels);
            assert_eq!(panes.len(), panels, "the grid dropped a pane");

            for pane in &panes {
                assert!(
                    cols <= pane
                        .width
                        .saturating_sub(SIDE_MARGIN * 2)
                        .max(MIN_AGENT_COLS),
                    "{panels} panels at {w}x{h}: {cols} columns will not fit pane {pane:?}"
                );
                assert!(
                    rows <= pane.height.max(MIN_AGENT_ROWS),
                    "{panels} panels at {w}x{h}: {rows} rows will not fit pane {pane:?}"
                );
            }

            // And it is not needlessly small: it matches the tightest pane
            // rather than shrinking to some safe fraction of it.
            let tightest_w = panes.iter().map(|p| p.width).min().unwrap();
            let tightest_h = panes.iter().map(|p| p.height).min().unwrap();
            assert_eq!(
                cols,
                tightest_w
                    .saturating_sub(SIDE_MARGIN * 2)
                    .max(MIN_AGENT_COLS)
            );
            assert_eq!(rows, tightest_h.max(MIN_AGENT_ROWS));
        }
    }
}

#[test]
fn no_panes_at_all_still_yields_a_drawable_size() {
    // A filter that matches nothing is a real state, and a render size of zero
    // would divide the agent's column arithmetic by it.
    assert_eq!(
        agent_dims(size(120, 40), 0),
        (MIN_AGENT_COLS, MIN_AGENT_ROWS)
    );
}

#[test]
fn a_terminal_too_small_for_the_grid_clamps_rather_than_underflowing() {
    let (cols, rows) = agent_dims(size(1, 1), 8);
    assert_eq!((cols, rows), (MIN_AGENT_COLS, MIN_AGENT_ROWS));
}

// ------------------------------------------------------------ the pane count

#[tokio::test]
async fn the_visible_pane_count_follows_the_filter() {
    let _g = isolate().await;
    let mut app = App::new(vec![
        test_server("web-01"),
        test_server("web-02"),
        test_server("db-01"),
        test_server("cache-01"),
    ]);
    assert_eq!(app.visible_panes(), 4);
    assert_eq!(app.visible_panes(), app.filtered_indices().len());

    app.filter_query = "web".to_string();
    assert_eq!(app.visible_panes(), 2);

    app.filter_query = "db".to_string();
    assert_eq!(app.visible_panes(), 1);

    app.filter_query = "nothing matches this".to_string();
    assert_eq!(app.visible_panes(), 0);
}

#[tokio::test]
async fn filtering_four_hosts_to_one_gives_the_pane_the_whole_screen() {
    // The defect, stated as the number it produced. Four hosts is a 2x2 grid;
    // one host is the whole body. The render size has to follow.
    let _g = isolate().await;
    let mut app = App::new(vec![
        test_server("web-01"),
        test_server("web-02"),
        test_server("db-01"),
        test_server("cache-01"),
    ]);
    let term = size(160, 48);

    let grid = agent_dims(term, app.visible_panes());
    app.filter_query = "cache".to_string();
    let alone = agent_dims(term, app.visible_panes());

    assert!(
        alone.0 > grid.0 && alone.1 > grid.1,
        "the filtered pane was still rendered for the grid: {grid:?} then {alone:?}"
    );
    // Specifically: the whole body, less the side margins and the key bar.
    assert_eq!(alone, (160 - SIDE_MARGIN * 2, 48 - 1));
}

#[tokio::test]
async fn typing_a_filter_moves_the_size_key_by_key() {
    // The path the user actually walks: `/`, then letters. Every keystroke
    // re-splits the grid, so every keystroke can move the render size.
    let _g = isolate().await;
    let mut app = App::new(vec![
        test_server("alpha-1"),
        test_server("alpha-2"),
        test_server("beta-1"),
    ]);
    let (tx, _rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(3);
    let term = size(120, 40);

    let three = agent_dims(term, app.visible_panes());
    press(&mut app, KeyCode::Char('/'), &tx, &mut tasks);
    for c in "alpha".chars() {
        press(&mut app, KeyCode::Char(c), &tx, &mut tasks);
    }
    assert_eq!(app.visible_panes(), 2);
    let two = agent_dims(term, app.visible_panes());

    press(&mut app, KeyCode::Char('-'), &tx, &mut tasks);
    press(&mut app, KeyCode::Char('1'), &tx, &mut tasks);
    assert_eq!(app.visible_panes(), 1);
    let one = agent_dims(term, app.visible_panes());

    assert!(
        three.0 < two.0,
        "dropping from a 2x2 grid to a single column did not widen the pane: \
         {three:?} then {two:?}"
    );
    assert!(
        two.1 < one.1,
        "dropping from two rows to one did not make the pane taller: \
         {two:?} then {one:?}"
    );

    // Backspacing widens the query again and the size goes back with it.
    press(&mut app, KeyCode::Backspace, &tx, &mut tasks);
    press(&mut app, KeyCode::Backspace, &tx, &mut tasks);
    assert_eq!(app.visible_panes(), 2);
    assert_eq!(agent_dims(term, app.visible_panes()), two);
}

#[tokio::test]
async fn clearing_the_filter_puts_the_size_back_where_it_was() {
    let _g = isolate().await;
    let mut app = App::new(vec![
        test_server("web-01"),
        test_server("web-02"),
        test_server("db-01"),
    ]);
    let term = size(100, 30);
    let unfiltered = agent_dims(term, app.visible_panes());

    let (tx, _rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(3);
    press(&mut app, KeyCode::Char('/'), &tx, &mut tasks);
    // "db", not "d": the filter matches the user as well as the host, and
    // every one of these is `admin`.
    for c in "db".chars() {
        press(&mut app, KeyCode::Char(c), &tx, &mut tasks);
    }
    assert_eq!(app.visible_panes(), 1);
    assert_ne!(agent_dims(term, app.visible_panes()), unfiltered);

    press(&mut app, KeyCode::Esc, &tx, &mut tasks);
    assert_eq!(
        agent_dims(term, app.visible_panes()),
        unfiltered,
        "leaving the filter left every agent rendering for one pane"
    );
}

#[tokio::test]
async fn a_filter_matching_nothing_does_not_produce_a_zero_sized_render() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("web-01"), test_server("web-02")]);
    app.filter_query = "no such host".to_string();
    assert_eq!(app.visible_panes(), 0);

    let (cols, rows) = agent_dims(size(120, 40), app.visible_panes());
    assert_eq!((cols, rows), (MIN_AGENT_COLS, MIN_AGENT_ROWS));
}

// ------------------------------------------------- through the real event loop

/// Drive the loop over a script of keys and hand back what it published on the
/// dims channel.
///
/// This is the gap the defect came through: every test of this behaviour asked
/// `agent_dims` directly, and `agent_dims` was never the thing that was wrong --
/// the number handed to it was. So the loop is run for real and the channel is
/// read, which is what the agents read.
async fn dims_after(size: (u16, u16), servers: Vec<Server>, keys: &str) -> (u16, u16) {
    let dir = tempfile::tempdir().expect("tempdir");
    let (dims_tx, dims_rx) = watch::channel((0u16, 0u16));
    let press = |code| {
        Event::Key(KeyEvent::new_with_kind(
            code,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        ))
    };
    let mut events: Vec<Event> = Vec::new();
    events.extend(keys.chars().map(|c| press(KeyCode::Char(c))));
    if !keys.is_empty() {
        // Enter, not Esc: Enter keeps the query and leaves the editor, which is
        // how a filter is applied. Esc clears it, and the loop would then quit
        // with nothing filtered and publish the unfiltered size.
        events.push(press(KeyCode::Enter));
    }
    // `q` is query text while the filter editor is open, so it goes last.
    events.push(press(KeyCode::Char('q')));
    let mut stream = tokio_stream::iter(events.into_iter().map(Ok)).chain(tokio_stream::pending());

    let backend = TestBackend::new(size.0, size.1);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    tokio::time::timeout(
        Duration::from_secs(20),
        multitop::run::event_loop(
            &mut terminal,
            &mut stream,
            dims_tx,
            servers,
            dir.path().join("config.toml"),
            None,
        ),
    )
    .await
    .expect("the loop must quit on `q`");
    let published = *dims_rx.borrow();
    published
}

#[tokio::test]
async fn the_loop_publishes_the_filtered_size_to_every_agent() {
    // End to end, through the loop the program actually runs: four hosts filtered
    // to one must publish the whole-screen size, not the 2x2 grid size. Before
    // the fix this published the grid size and nothing noticed.
    let _g = isolate().await;
    let hosts = || {
        vec![
            test_server("web-01"),
            test_server("web-02"),
            test_server("db-01"),
            test_server("cache-01"),
        ]
    };

    let unfiltered = dims_after((160, 48), hosts(), "").await;
    let filtered = dims_after((160, 48), hosts(), "/cache").await;

    assert_eq!(
        unfiltered,
        agent_dims(size(160, 48), 4),
        "the loop did not publish the grid size with no filter"
    );
    assert_eq!(
        filtered,
        agent_dims(size(160, 48), 1),
        "the loop published {filtered:?} for a single visible pane"
    );
    assert!(filtered.0 > unfiltered.0 && filtered.1 > unfiltered.1);
}

#[tokio::test]
async fn a_filter_that_changes_no_pane_count_publishes_nothing_new() {
    // The reason there is no debounce timer: the size only moves when the size
    // actually moves. Typing further into a query that still matches the same
    // one host must not republish.
    let _g = isolate().await;
    let hosts = || vec![test_server("web-01"), test_server("db-01")];
    let short = dims_after((120, 40), hosts(), "/db").await;
    let long = dims_after((120, 40), hosts(), "/db-0").await;
    assert_eq!(short, long);
}
