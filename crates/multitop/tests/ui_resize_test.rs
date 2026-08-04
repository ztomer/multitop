#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use multitop::app::{App, Mode};
use multitop::config::Server;
use multitop::ui::{agent_dims, draw, refit_header, refit_line};

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

fn sample_servers(count: usize) -> Vec<Server> {
    (0..count)
        .map(|i| Server {
            host: format!("192.168.0.{}", 10 + i),
            port: 22,
            user: "root".into(),
            upgrade_cmd: None,
        })
        .collect()
}

#[test]
fn refit_header_expands_and_shrinks_dynamically() {
    let _keychain = isolate_keychain();
    let raw_header =
        "\x1b[90m──────────\x1b[0m\x1b[36;1m ｂｅｅｌｉｎｋ \x1b[0m\x1b[90m──────────\x1b[0m";

    // 1. Expand width to 60 cols
    let refitted_wide = refit_header(raw_header, 60).expect("should refit wide");
    assert!(refitted_wide.contains("ｂｅｅｌｉｎｋ"));

    // 2. Shrink width to 20 cols
    let refitted_narrow = refit_header(raw_header, 20).expect("should refit narrow");
    assert!(refitted_narrow.contains("ｂｅｅｌｉｎｋ"));
}

#[test]
fn refit_line_reflows_divider_rules() {
    let _keychain = isolate_keychain();
    let raw_rule = " \x1b[90m────────────────────\x1b[0m";
    let refitted = refit_line(raw_rule, 80);
    let plain = multitop_agent::color::strip_ansi(&refitted);
    assert_eq!(plain.chars().count(), 79);
}

#[test]
fn ui_renders_without_panic_across_resizes() {
    let _keychain = isolate_keychain();
    let servers = sample_servers(3);
    let mut app = App::new(servers);

    for (i, p) in app.panels.iter_mut().enumerate() {
        p.view = vec![
            format!("\x1b[90m──────────\x1b[0m\x1b[36;1m ｓｅｒｖｅｒ_{i} \x1b[0m\x1b[90m──────────\x1b[0m"),
            " CPU [####....] 50%".into(),
            " MEM [########..] 80%".into(),
            " DSK [##........] 20%".into(),
        ];
        p.mode = Mode::Monitor;
    }

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    // Initial render at 80x24
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer1 = terminal.backend().buffer().clone();
    assert_eq!(buffer1.area.width, 80);
    assert_eq!(buffer1.area.height, 24);

    // Resize to 140x50 (large wide terminal)
    terminal.backend_mut().resize(140, 50);
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer2 = terminal.backend().buffer().clone();
    assert_eq!(buffer2.area.width, 140);
    assert_eq!(buffer2.area.height, 50);

    // Resize to 45x12 (compact small terminal)
    terminal.backend_mut().resize(45, 12);
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer3 = terminal.backend().buffer().clone();
    assert_eq!(buffer3.area.width, 45);
    assert_eq!(buffer3.area.height, 12);
}

#[test]
fn agent_dims_recalculate_consistently_on_resize() {
    use ratatui::layout::Size;
    let _keychain = isolate_keychain();

    let sz_initial = Size::new(100, 30);
    let (cols1, rows1) = agent_dims(sz_initial, 4);
    assert_eq!(cols1, 48);
    assert_eq!(rows1, 14);

    let sz_resized = Size::new(160, 60);
    let (cols2, rows2) = agent_dims(sz_resized, 4);
    assert_eq!(cols2, 78);
    assert_eq!(rows2, 29);
}

/// Bug c finding #6: `reset_scroll()` must reset EVERY panel's scroll, not just
/// the selected one (upgrade/fetch/docker views switch all panels at once).
#[test]
fn reset_scroll_clears_all_panels_not_just_selected() {
    let _keychain = isolate_keychain();
    let mut app = App::new(sample_servers(3));
    app.selected_panel = 1;
    for (i, p) in app.panels.iter_mut().enumerate() {
        p.scroll_offset = 5 + i;
    }

    app.reset_scroll();

    for (i, p) in app.panels.iter().enumerate() {
        assert_eq!(p.scroll_offset, 0, "panel {i} scroll must reset");
    }
}
