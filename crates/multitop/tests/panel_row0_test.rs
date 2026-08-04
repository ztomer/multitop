//! Does `ui::draw` actually put it on the screen?
//!
//! Split from `ui_test.rs` because these build a real `App` and render it,
//! which can reach `password_store` several calls down -- and the isolation
//! gate rightly requires every test in such a file to divert the credential
//! store. The rest of `ui_test.rs` is pure layout arithmetic and should stay
//! that way.
//!
//! What these exist for: both defects below were invisible to tests that called
//! the layout functions in isolation. Only a real buffer can answer "is it on
//! the screen".

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// Divert credentials to the in-memory store, and hold the process-global guard.
///
/// These two tests build a real `App` and render it, and the render path can
/// reach `password_store` several calls down. An integration binary is compiled
/// without `cfg(test)`, so the mock is not in force unless it is asked for --
/// and every rebuild changes the binary's code signature, so macOS raises a
/// keychain dialog and the suite stops until a human dismisses it.
fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test();
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

/// The scroll badge must reach the screen.
///
/// It never had. `visible()` composed `[↑ -N lines]` onto row 0 and `draw` then
/// overwrote row 0 with the host banner, unconditionally, every frame -- so the
/// badge was built and destroyed within one frame and the scroll-position
/// indicator has never once been visible to a user.
///
/// The test that should have caught it called `visible()` in isolation with
/// `target_cols = 0`, which skips the entire badge path, so it passed against
/// code that rendered nothing. This one goes through `ui::draw` into a real
/// buffer, which is the only place the question can actually be answered.
#[test]
fn the_scroll_badge_survives_into_the_rendered_frame() {
    let _keychain = isolate_keychain();
    let mut app = multitop::app::App::new(vec![multitop::config::Server {
        host: "web-01".to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: None,
    }]);
    let mut view = vec![String::new(), " Status     running".to_string()];
    view.extend((0..200).map(|i| format!("line {i}")));
    app.panels[0].view = view;
    app.panels[0].pinned_lines = 2;
    app.panels[0].scroll_offset = 40;

    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
    term.draw(|f| multitop::ui::draw(f, &mut app)).unwrap();
    let screen: String = term
        .backend()
        .buffer()
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect();

    assert!(
        screen.contains("\u{2191} -40 lines"),
        "the scroll badge must be on screen, got:\n{screen}"
    );
}

/// A host that has not connected yet must say so.
///
/// `Panel::new` gives the body one line, `connecting...`, and the banner is
/// composed over row 0 -- so the whole body was eaten and a host coming up
/// rendered as an empty box, indistinguishable from a hung SSH session or a
/// dead app. Across 88 rendered frames the word did not appear once.
#[test]
fn a_connecting_host_says_connecting() {
    let _keychain = isolate_keychain();
    let mut app = multitop::app::App::new(vec![multitop::config::Server {
        host: "web-01".to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: None,
    }]);
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
    term.draw(|f| multitop::ui::draw(f, &mut app)).unwrap();
    let screen: String = term
        .backend()
        .buffer()
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect();

    assert!(
        screen.contains("connecting"),
        "an empty panel is indistinguishable from a dead app, got:\n{screen}"
    );
}

/// Two hosts must never render the same banner.
///
/// The banner was mapped into fullwidth codepoints, which doubled the cell cost
/// and then clipped on the right -- so at four panels on a small terminal
/// `ztomer@webserver-01` became `ｚｔｏｍｅｒ＠ｗｅ`, and the digits, the only
/// part that differs, were exactly what fell off. On a tool where the selected
/// panel is the machine `u` runs `apt upgrade` against, the label that says
/// which machine you are about to touch is the one that must never be wrong.
#[test]
fn two_hosts_never_share_a_banner() {
    let _keychain = isolate_keychain();
    let hosts = [
        "webserver-01",
        "webserver-02",
        "webserver-03",
        "webserver-04",
    ];
    let mut app = multitop::app::App::new(
        hosts
            .iter()
            .map(|h| multitop::config::Server {
                host: (*h).to_string(),
                port: 22,
                user: "ztomer".to_string(),
                upgrade_cmd: None,
            })
            .collect(),
    );

    for width in [40u16, 60, 80, 120] {
        let mut term =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 16)).unwrap();
        term.draw(|f| multitop::ui::draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer();
        let cols = buf.area.width as usize;
        let rows: Vec<String> = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<Vec<_>>()
            .chunks(cols)
            .map(<[&str]>::concat)
            .collect();

        // Every host's distinguishing tail must be somewhere on screen.
        for host in hosts {
            let tail = &host[host.len() - 2..];
            assert!(
                rows.iter().any(|r| r.contains(tail)),
                "{width} cols: nothing on screen distinguishes {host}:\n{}",
                rows.join("\n")
            );
        }
    }
}
