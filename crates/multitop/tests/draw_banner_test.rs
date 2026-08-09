//! The panel banner, drawn at sizes it does not fit into.
//!
//! Row 0 is composed once from the banner and the scroll badge. Every defect
//! this row has had was a width computed twice and disagreeing with itself, so
//! what matters here is that the name still gets drawn when there is no room
//! for a rule either side of it — and that the panel is skipped, not panicked
//! on, when there is no room at all.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop::app::{App, Mode};
use multitop::config::Server;
use multitop::password_store;
use multitop_agent::proc::{Proc, Usage};
use multitop_agent::proto::Payload;
use multitop_agent::render::Snapshot;
use ratatui::backend::TestBackend;

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
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

/// Draw one frame at `size` and hand back everything on it.
fn drawn(app: &mut App, size: (u16, u16)) -> String {
    let backend = TestBackend::new(size.0, size.1);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| multitop::ui::draw(f, app))
        .expect("the frame must draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect()
}

fn monitor_snapshot(host: &str) -> Payload {
    Payload::Monitor(Snapshot {
        host: host.into(),
        agent_version: "9.9.9".into(),
        cpu_pct: 30.0,
        cpu_mhz: Some(3600.0),
        proc_names: Vec::new(),
        cores: vec![(0, 10.0, None)],
        mem: Usage::new(8 << 30, 2 << 30),
        disk: Usage::new(256 << 30, 64 << 30),
        rx_rate: 1.0,
        tx_rate: 1.0,
        procs: vec![Proc {
            pid: 1,
            name: "init".into(),
            cpu: 1.0,
            mem: 1024,
        }],
        ..Default::default()
    })
}

#[tokio::test]
async fn the_banner_prefers_the_name_the_agent_reports() {
    // The agent knows the host's own name and address; the config only knows
    // what was typed. When both are available the reported one wins.
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("configured-name")]);
    app.panels[0].last_monitor = Some(monitor_snapshot("reported-name (10.0.0.4)"));
    app.panels[0].last_frame = Some(vec!["body".to_string(); 4]);
    app.panels[0].mode = Mode::Monitor;
    app.panels[0].show_frame(vec!["body".to_string(); 4]);

    let text = drawn(&mut app, (120, 30));
    assert!(
        text.contains("reported-name"),
        "the reported name is missing:\n{text}"
    );
}

#[tokio::test]
async fn a_banner_with_no_room_for_a_rule_still_draws_the_name() {
    // Narrow enough that the name plus two spaces exceeds the row: the rules
    // are dropped rather than the name.
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("a-fairly-long-hostname.example.com")]);
    app.panels[0].show_frame(vec!["body".to_string(); 3]);

    let text = drawn(&mut app, (32, 12));
    // Fitted from the right, so the distinguishing tail survives and the head
    // is what gets cut — the digits are the only part that differs per host.
    assert!(
        text.contains("hostname.example.com"),
        "the name was dropped entirely at a narrow width:\n{text}"
    );
    assert!(
        !text.contains("\u{2500}\u{2500}\u{2500}"),
        "a rule was drawn where there was no room for one:\n{text}"
    );
}

#[tokio::test]
async fn a_wide_banner_draws_a_rule_either_side_of_the_name() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("web-01")]);
    app.panels[0].show_frame(vec!["body".to_string(); 3]);

    let text = drawn(&mut app, (120, 30));
    assert!(text.contains("web-01"), "{text}");
    assert!(
        text.contains("\u{2500}\u{2500}"),
        "no rule was drawn at a wide size:\n{text}"
    );
}

#[tokio::test]
async fn a_pane_with_no_room_at_all_is_skipped_rather_than_drawn_into() {
    // A grid this small leaves panes of zero usable width once the side margin
    // is taken off. Drawing into one is a panic; skipping it is the answer.
    let _g = isolate().await;
    let mut app = App::new(vec![
        test_server("alpha"),
        test_server("beta"),
        test_server("gamma"),
        test_server("delta"),
    ]);
    for p in &mut app.panels {
        p.show_frame(vec!["body".to_string(); 2]);
    }

    for size in [(1u16, 1u16), (2, 2), (4, 3), (8, 4), (3, 10)] {
        let _ = drawn(&mut app, size);
    }
}

#[tokio::test]
async fn a_scrolled_pane_says_how_far_back_it_is_looking() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("web-01")]);
    for i in 0..200 {
        app.panels[0].last_upgrade.push(format!("line {i}"));
    }
    app.enter_upgrade_view();
    app.scroll_up(30);

    let text = drawn(&mut app, (120, 30));
    assert!(
        text.contains("lines"),
        "a scrolled pane gave no sign it was not at the bottom:\n{text}"
    );
}

#[tokio::test]
async fn both_banner_styles_draw_the_host() {
    let _g = isolate().await;
    for style in ["plain", "wide"] {
        let mut app = App::new(vec![test_server("web-01")]);
        app.banner_style = multitop::layout::BannerStyle::parse(style);
        app.panels[0].show_frame(vec!["body".to_string(); 3]);

        // Cell by cell: a fullwidth glyph occupies two cells and the second
        // reads as a blank, so the cells are joined with the padding removed
        // before looking for the name.
        let text: String = drawn(&mut app, (120, 30))
            .chars()
            .filter(|c| *c != ' ')
            .collect();
        let expected = if style == "wide" {
            multitop_agent::fmt::fullwidth("web-01")
        } else {
            "web-01".to_string()
        };
        assert!(
            text.contains(&expected),
            "{style}: the host is missing:\n{text}"
        );
    }
}

#[tokio::test]
async fn a_scroll_badge_that_leaves_no_room_for_the_name_still_draws_the_badge() {
    // The badge and the banner share row 0. Once the badge has taken the width
    // there is nothing left to centre a name in — and the badge is the part
    // that has to survive, because it is the only thing saying the pane is not
    // showing the end of the log.
    let _g = isolate().await;

    for width in 16u16..=20 {
        let mut app = App::new(vec![test_server("web-01")]);
        for i in 0..200 {
            app.panels[0].last_upgrade.push(format!("line {i}"));
        }
        app.enter_upgrade_view();
        app.scroll_up(40);

        let text = drawn(&mut app, (width, 14));
        let row0: String = text.chars().take(width as usize).collect();
        assert!(
            row0.contains('\u{2191}'),
            "width {width}: the scroll badge was dropped: {row0:?}"
        );
        assert!(
            !row0.contains('\u{2500}'),
            "width {width}: a rule was drawn into a row with no room: {row0:?}"
        );
    }
}
