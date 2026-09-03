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
            custom_command: None,
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

/// Every pane arithmetic at sizes smaller than any content.
///
/// `render_views.rs` says it is a gate for sizes "smaller than the content",
/// and its smallest is 40x12 -- which is small, and is not the case that finds
/// a subtraction. `regions`, `pane_window` and `agent_dims` all divide a rect
/// among panels and window a body inside it; a one-column or one-row terminal
/// is where a `- 2` for a border becomes a wrap.
///
/// The roadmap set this question for the rendering round: whether any pane can
/// be given a negative or wrapping arithmetic result at extreme sizes.
#[test]
fn every_pane_arithmetic_survives_a_terminal_too_small_to_draw_in() {
    // Nothing here touches a credential, but the gate checks *reaching* per file
    // and *diverting* per test -- deliberately, because the reaching call is
    // usually one helper away and text matching cannot follow a call graph. A
    // test that argues with a structural gate is how the gate gets switched off.
    use ratatui::layout::{Rect, Size};

    let _keychain = isolate_keychain();

    for w in [0u16, 1, 2, 3, 5, 10] {
        for h in [0u16, 1, 2, 3, 5, 10] {
            for panels in [0usize, 1, 2, 3, 4, 8] {
                let area = Rect::new(0, 0, w, h);
                let (areas, keybar) = multitop::ui::regions(area, panels);
                for a in &areas {
                    assert!(
                        a.x + a.width <= w && a.y + a.height <= h,
                        "pane {a:?} escapes a {w}x{h} screen with {panels} panels"
                    );
                }
                assert!(
                    keybar.y + keybar.height <= h,
                    "keybar {keybar:?} escapes a {w}x{h} screen"
                );

                let dims = multitop::ui::agent_dims(
                    Size {
                        width: w,
                        height: h,
                    },
                    panels,
                );
                assert!(dims.0 > 0 && dims.1 > 0, "agent told to render {dims:?}");

                // And the windowing, at every scroll offset that could be stored.
                for off in [0usize, 1, 7, 1000] {
                    let body: Vec<String> = (0..5).map(|i| format!("line {i}")).collect();
                    let (out, _badge) =
                        multitop::ui::visible(&body, usize::from(h), usize::from(w), 1, off);
                    assert!(
                        out.len() <= usize::from(h).max(1),
                        "windowed {} lines into a height of {h}",
                        out.len()
                    );
                }
            }
        }
    }
}

/// A filter query is user-typed and unbounded; the rows it is drawn into are
/// not. The way out must survive any of them.
///
/// `draw_no_matches` exists because "an empty result and a dead app look
/// identical otherwise, and the way out -- `Esc` -- is not guessable from a
/// blank terminal". The query is interpolated into its first line and echoed in
/// the keybar, both fixed-width and both hard-clipping. If a long enough query
/// can push the instruction off the screen, the screen built to state the way
/// out stops stating it.
#[test]
fn a_long_filter_query_cannot_push_the_way_out_off_the_screen() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let _keychain = isolate_keychain();
    for query_len in [1usize, 20, 60, 200, 1000] {
        for (w, h) in [(40u16, 12u16), (80, 24), (20, 6)] {
            let mut app = multitop::app::App::new(vec![multitop::config::Server {
                host: "web-01".into(),
                port: 22,
                user: "admin".into(),
                upgrade_cmd: None,
                custom_command: None,
            }]);
            app.filter_query = "z".repeat(query_len);

            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| multitop::ui::draw(f, &mut app)).unwrap();
            let buf = term.backend().buffer().clone();
            let screen: String = (0..h)
                .map(|y| {
                    (0..w)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");

            assert!(
                screen.contains("Esc"),
                "a {query_len}-character query at {w}x{h} left no way out on screen:\n{screen}"
            );
        }
    }
}

/// A modal must never amputate its own way out, at any size.
///
/// The vault prompts clipped at 40 columns -- "Press Enter to unlock, Esc to
/// canc" -- which is the defect the detection record lists as user-reported,
/// fixed for the upgrade confirmation by Kare's ruling and left in these boxes.
/// A password prompt cannot become a keybar row, so it sheds instead: the
/// explanation goes first, then the spacing blanks, and the headline, the field
/// and the footer never do.
#[test]
fn a_vault_prompt_keeps_its_way_out_at_every_size() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let _keychain = isolate_keychain();
    for creating in [false, true] {
        for with_error in [false, true] {
            for (w, h) in [(40u16, 12u16), (56, 14), (80, 24), (200, 50)] {
                let mut app = multitop::app::App::new(vec![multitop::config::Server {
                    host: "web-01".into(),
                    port: 22,
                    user: "admin".into(),
                    upgrade_cmd: None,
                    custom_command: None,
                }]);
                if creating {
                    // Set directly, as the render harness does: the modal is
                    // drawn from this state, and `begin_vault_creation` refuses
                    // when another prompt could be up.
                    app.vault_state = multitop::app::VaultState::Creating {
                        error: with_error.then(|| "Master password cannot be empty".to_string()),
                        in_flight: false,
                    };
                } else {
                    app.set_show_vault_password_prompt(true);
                    if with_error {
                        app.set_vault_password_error(Some("Wrong password".into()));
                    }
                }

                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                term.draw(|f| multitop::ui::draw(f, &mut app)).unwrap();
                let buf = term.backend().buffer().clone();
                let screen: String = (0..h)
                    .map(|y| {
                        (0..w)
                            .map(|x| buf[(x, y)].symbol().to_string())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                let what = format!("creating={creating} error={with_error} at {w}x{h}");
                for needed in ["Enter", "Esc", "cancel"] {
                    assert!(
                        screen.contains(needed),
                        "{what}: the footer lost {needed:?}:\n{screen}"
                    );
                }
                assert!(
                    screen.contains("master password"),
                    "{what}: the headline is gone:\n{screen}"
                );
            }
        }
    }
}

/// A status line must survive the banner that is drawn over row 0.
///
/// `ui::draw` replaces `lines[0]` with the host banner on every frame, which is
/// why `Panel::new` reserves row 0 with a placeholder -- Round B found that "a
/// one-line body is eaten, so the connecting state is a blank box". `Msg::Status`
/// still assigned `vec![text]`: one line, which *is* row 0, so a fetch or docker
/// connection error was written into the pane and destroyed by the banner on the
/// same frame it arrived.
#[test]
fn a_status_line_is_not_eaten_by_the_banner() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let _keychain = isolate_keychain();
    let mut app = multitop::app::App::new(vec![multitop::config::Server {
        host: "web-01".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: None,
        custom_command: None,
    }]);
    app.apply(multitop::app::Msg::Status {
        panel: 0,
        gen: app.panels[0].gen,
        text: "ssh command not found".to_string(),
    });

    let mut term = Terminal::new(TestBackend::new(60, 8)).unwrap();
    term.draw(|f| multitop::ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let screen: String = (0..8)
        .map(|y| {
            (0..60)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        screen.contains("ssh command not found"),
        "the status the panel was given never reached the screen:\n{screen}"
    );
}
