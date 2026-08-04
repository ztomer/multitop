//! Filtering the panel grid with `/`.
//!
//! The scaffolding for this shipped a long time ago and was never reachable:
//! `filter_query`, `filtered_indices` and `AppMode::Filtering` all existed, no
//! key produced them, and `ui.rs` ignored them. So these drive it the way the
//! feature is actually used -- real key presses through `run::handle_key`, and
//! a real `ui::draw` into a `TestBackend` -- rather than calling the `App`
//! methods that were already "tested" while the feature did not exist.
//!
//! The properties that matter are the ones a half-built filter gets wrong:
//! panels really disappear, the selection cannot be left on a hidden host, a
//! query that matches nothing says so instead of showing a blank screen, and
//! the user is always told a filter is in force.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tokio::sync::{mpsc, watch};

use multitop::app::{App, Msg};
use multitop::config::Server;
use multitop::run::{handle_key, Tasks};

/// Divert credentials to the in-memory store, and hold the process-global guard.
///
/// Driving an `App` reaches `password_store` several calls down, and an
/// integration binary is compiled without `cfg(test)`, so the mock is not in
/// force unless it is asked for. Without this these tests query the real OS
/// keychain: every rebuild changes the binary's code signature, so macOS raises
/// an access dialog and the suite stops until a human dismisses it -- and a test
/// can read, overwrite or delete credentials the user depends on.
#[allow(dead_code)]
fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test();
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

/// `isolate_keychain` for `#[tokio::test]` bodies, which must not block the
/// runtime thread to take the guard.
#[allow(dead_code)]
async fn isolate_keychain_async() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test_async().await;
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

fn server(host: &str, user: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: user.to_string(),
        upgrade_cmd: None,
    }
}

struct Harness {
    app: App,
    servers: Vec<Server>,
    tasks: Tasks,
    tx: mpsc::Sender<Msg>,
    dims_rx: Arc<watch::Receiver<(u16, u16)>>,
    terminal: Terminal<TestBackend>,
}

impl Harness {
    fn new() -> Self {
        let servers = vec![
            server("web-01", "admin"),
            server("db-01", "postgres"),
            server("web-02", "admin"),
            server("cache-01", "redis"),
        ];
        let mut app = App::new(servers.clone());
        // A panel with an empty `view` renders nothing at all -- the host banner
        // is written over line 0, so there has to be a line 0. This is the
        // "waiting for data..." state a real panel is in before its first frame.
        for panel in &mut app.panels {
            panel.show_last_frame();
        }
        let (tx, _rx) = mpsc::channel::<Msg>(64);
        let (dims_tx, drx) = watch::channel((120u16, 40u16));
        drop(dims_tx);
        Self {
            tasks: Tasks::new(servers.len()),
            terminal: Terminal::new(TestBackend::new(120, 40)).unwrap(),
            app,
            servers,
            tx,
            dims_rx: Arc::new(drx),
        }
    }

    fn press(&mut self, code: KeyCode) {
        handle_key(
            KeyEvent {
                code,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
            &mut self.app,
            &self.servers,
            (120, 40),
            Arc::clone(&self.dims_rx),
            &self.tx,
            &mut self.tasks,
        );
        self.draw();
    }

    fn type_str(&mut self, text: &str) {
        for c in text.chars() {
            self.press(KeyCode::Char(c));
        }
    }

    fn draw(&mut self) {
        // Force a full redraw. ratatui diffs against the previous frame, and a
        // fullwidth glyph covers two columns physically while only writing the
        // first cell -- so in a `TestBackend` buffer the second cell keeps
        // whatever the last frame put there, and reading the buffer back gives
        // `a-d-m-i-n` instead of `admin`. A real terminal never shows this: the
        // glyph itself covers both columns.
        self.terminal.clear().unwrap();
        self.terminal
            .draw(|f| multitop::ui::draw(f, &mut self.app))
            .unwrap();
    }

    /// The rendered screen, with the fullwidth host banner folded back to ASCII
    /// so a test can look for "web-01" rather than the decorative form.
    fn screen(&self) -> String {
        let raw: String = self
            .terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<Vec<_>>()
            .chunks(120)
            .map(<[&str]>::concat)
            .collect::<Vec<_>>()
            .join("\n");
        // A fullwidth glyph occupies two terminal cells, and the backend fills
        // the second with a space. Folding the glyph back to ASCII without
        // dropping that pad yields "w e b - 0 1", which matches nothing.
        let mut out = String::with_capacity(raw.len());
        let mut skip_pad = false;
        for c in raw.chars() {
            let n = c as u32;
            // Fullwidth forms sit at U+FF01..=U+FF5E, one-for-one with ASCII.
            if (0xFF01..=0xFF5E).contains(&n) {
                out.push(char::from_u32(n - 0xFEE0).unwrap_or(c));
                skip_pad = true;
            } else if skip_pad && c == ' ' {
                skip_pad = false;
            } else {
                skip_pad = false;
                out.push(c);
            }
        }
        out
    }

    fn shows(&self, host: &str) -> bool {
        self.screen().contains(host)
    }
}

#[tokio::test]
async fn slash_narrows_the_grid_to_matching_hosts() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new();
    h.draw();
    assert!(
        h.shows("web-01") && h.shows("db-01"),
        "all four to start:\n{}",
        h.screen()
    );

    h.press(KeyCode::Char('/'));
    h.type_str("web");

    assert!(h.shows("web-01"), "a match must stay: {}", h.screen());
    assert!(h.shows("web-02"), "both matches must stay");
    assert!(
        !h.shows("db-01") && !h.shows("cache-01"),
        "non-matching hosts must be gone, not merely dimmed:\n{}",
        h.screen()
    );
}

/// The user must be able to type a host name that collides with a binding.
#[tokio::test]
async fn keys_that_are_bindings_elsewhere_are_ordinary_text_while_typing() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new();
    h.press(KeyCode::Char('/'));
    // 'd' is Docker, 'u' is Upgrade, 'q' quits, 'e' opens Settings.
    h.type_str("db-01");

    assert_eq!(h.app.filter_query, "db-01");
    assert!(
        h.app.password_manager.is_none(),
        "typing 'e' must not have opened Settings"
    );
    assert!(!h.app.should_quit(), "typing 'q' must not have quit");
}

#[tokio::test]
async fn enter_keeps_the_filter_and_esc_clears_it() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new();
    h.press(KeyCode::Char('/'));
    h.type_str("web");
    h.press(KeyCode::Enter);

    assert!(!h.app.is_filtering(), "Enter leaves the prompt");
    assert_eq!(h.app.filter_query, "web", "but keeps what was typed");
    assert!(!h.shows("db-01"), "the filter is still in force");

    h.press(KeyCode::Esc);
    assert_eq!(h.app.filter_query, "", "Esc clears an applied filter");
    assert!(h.shows("db-01"), "and every host comes back");
    assert!(!h.app.should_quit(), "that Esc must not also quit the app");
}

#[tokio::test]
async fn esc_while_typing_abandons_the_query() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new();
    h.press(KeyCode::Char('/'));
    h.type_str("web");
    h.press(KeyCode::Esc);

    assert!(!h.app.is_filtering());
    assert_eq!(h.app.filter_query, "");
    assert!(h.shows("db-01"));
    assert!(!h.app.should_quit());
}

#[tokio::test]
async fn backspace_widens_the_match() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new();
    h.press(KeyCode::Char('/'));
    h.type_str("web-01");
    assert!(!h.shows("web-02"));

    h.press(KeyCode::Backspace);
    h.press(KeyCode::Backspace);
    h.press(KeyCode::Backspace);
    assert_eq!(h.app.filter_query, "web");
    assert!(h.shows("web-02"), "{}", h.screen());
}

/// A blank screen and a dead app look the same, and `Esc` is not guessable.
#[tokio::test]
async fn a_query_that_matches_nothing_says_so() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new();
    h.press(KeyCode::Char('/'));
    h.type_str("nosuchhost");

    let screen = h.screen();
    assert!(
        screen.contains("No host matches"),
        "an empty result must explain itself, got:\n{screen}"
    );
    assert!(
        screen.contains("Esc"),
        "and must say how to get out, got:\n{screen}"
    );
}

/// Hidden panels with no indication is a monitor that quietly stops monitoring.
#[tokio::test]
async fn an_applied_filter_is_always_visible() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new();
    h.press(KeyCode::Char('/'));
    h.type_str("web");
    h.press(KeyCode::Enter);

    assert!(
        h.screen().contains("[filter: web]"),
        "with the prompt closed, the keybar must still say a filter is on:\n{}",
        h.screen()
    );
}

/// The selection drives the keybar's mode badge and every view-switching key,
/// so it must not be left pointing at a host that is not on screen.
#[tokio::test]
async fn the_selection_moves_off_a_hidden_host() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new();
    h.press(KeyCode::Char('4')); // cache-01
    assert_eq!(h.app.selected_panel, 3);

    h.press(KeyCode::Char('/'));
    h.type_str("web");

    let shown = h.app.filtered_indices();
    assert!(
        shown.contains(&h.app.selected_panel),
        "selection {} is not among the visible panels {shown:?}",
        h.app.selected_panel
    );
}

#[tokio::test]
async fn the_keybar_advertises_the_key() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new();
    h.draw();
    assert!(
        h.screen().contains("Filter"),
        "an unadvertised key is an undiscoverable feature:\n{}",
        h.screen()
    );
}

/// The number keys count panes on screen, not entries in the config.
///
/// They used to index the unfiltered list and clamp to its end. With `/db`
/// showing one pane, `2` selected a host that was not on screen -- and every
/// view key after it acted on that host, invisibly. The panes and the keys that
/// name them have to be counting the same things.
#[test]
fn number_keys_select_visible_panes_only() {
    let _keychain = isolate_keychain();
    let mut h = Harness::new();

    // web-01, db-01, web-02, cache-01 -> "web" leaves panes 1 and 2 on screen,
    // which are indices 0 and 2 in the configured list.
    h.press(KeyCode::Char('/'));
    h.type_str("web");
    h.press(KeyCode::Enter);
    assert_eq!(
        h.app.filtered_indices(),
        vec![0, 2],
        "the filter must leave exactly the two web hosts"
    );

    h.press(KeyCode::Char('2'));
    assert_eq!(
        h.app.selected_panel, 2,
        "the second pane on screen is web-02, whatever its index in the config"
    );
    assert_eq!(
        h.app.panels[h.app.selected_panel].server.host, "web-02",
        "and the selection must name the host the user counted to"
    );

    h.press(KeyCode::Char('3'));
    assert_eq!(
        h.app.selected_panel, 2,
        "there is no third pane on screen, so nothing may move -- the same \
         answer a click on no pane gets"
    );
}

/// Without a filter the number keys behave exactly as before.
#[test]
fn number_keys_still_select_by_position_when_nothing_is_filtered() {
    let _keychain = isolate_keychain();
    let mut h = Harness::new();

    h.press(KeyCode::Char('3'));
    assert_eq!(h.app.selected_panel, 2);
    assert_eq!(h.app.panels[h.app.selected_panel].server.host, "web-02");
}

/// `Home` goes as far back as the pane can show, `End` comes back, and every
/// press in between moves the view by exactly one line.
///
/// Three defects meet here. `Home` used to subtract one from the offset, and
/// the offset counts lines scrolled *back* -- so the key advertised as "top"
/// moved one line towards the newest. `End` used to reset every pane in the
/// grid, so returning one pane to the bottom threw away where the user had
/// scrolled all the others to. And the offset was bounded in two places by
/// different rules: `App::scroll_up` clamps to `pane_len - 1` because it cannot
/// know a pane's height, while the view can only go back `pane_len - height` --
/// so an offset could sit a whole pane-height past anything the view could use,
/// and the next dozen presses the other way moved nothing at all.
#[test]
fn home_and_end_move_the_selected_pane_to_its_ends() {
    let _keychain = isolate_keychain();
    let mut h = Harness::new();

    // Give the panes something long enough to scroll through.
    for panel in &mut h.app.panels {
        panel.view = (0..40).map(|n| format!("line {n}")).collect();
    }

    // Scroll pane 1 back, and leave it there as the control.
    h.press(KeyCode::Char('2'));
    h.press(KeyCode::Up);
    h.press(KeyCode::Up);
    assert_eq!(h.app.panels[1].scroll_offset, 2);

    h.press(KeyCode::Char('1'));
    h.press(KeyCode::Home);
    let top = h.app.panels[0].scroll_offset;
    assert!(
        top > 0,
        "Home is the oldest line the pane holds, not one line towards the newest"
    );
    let at_top = h.screen();

    // The press after Home has to move the view. It used to be spent walking
    // back through the gap between the two clamps, with nothing on screen
    // changing, for as many presses as the pane is tall.
    h.press(KeyCode::Down);
    assert_eq!(
        h.app.panels[0].scroll_offset,
        top - 1,
        "one press moves one line"
    );
    assert_ne!(
        h.screen(),
        at_top,
        "and it must move the view: an offset stored beyond what the view can \
         use makes the key read as dead"
    );

    h.press(KeyCode::End);
    assert_eq!(h.app.panels[0].scroll_offset, 0, "End is the newest line");
    assert_eq!(
        h.app.panels[1].scroll_offset, 2,
        "and it must leave every other pane where the user put it"
    );
}
