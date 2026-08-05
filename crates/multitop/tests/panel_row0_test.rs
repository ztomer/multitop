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
    // Row 0 is the banner's, and it is the only pinned row an ordinary pane has;
    // the rest scrolls under it.
    let mut view = vec![String::new()];
    view.extend((0..200).map(|i| format!("line {i}")));
    app.panels[0].view = view;
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

/// A notice must be reachable by scrolling.
///
/// `App::pane_len` counted `view.len()` and stopped there, while the pane
/// `ui::pane_lines` composes is `view` *plus every notice, wrapped to the pane's
/// width*. Two derivations of one quantity, and the scroll clamp used the
/// shorter one: `Home` -- documented as "the oldest line the pane holds" --
/// stopped `notes` lines short of the top, and the lines it could not reach were
/// the notices themselves.
#[test]
fn home_reaches_the_oldest_line_when_the_pane_carries_notices() {
    let _keychain = isolate_keychain();
    let mut app = multitop::app::App::new(vec![multitop::config::Server {
        host: "web-01".to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: None,
    }]);
    app.panels[0].note(
        "FIRST could not save upgrade state (Permission denied (os error 13)) -- an \
         interrupted run will not be detectable after a restart."
            .to_string(),
    );
    for n in 0..3 {
        app.panels[0].note(format!(
            "note {n}: {} password(s) could not be written to the new vault; they \
             remain in the OS credential store.",
            n + 1
        ));
    }
    app.scroll_to_top();

    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
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

    assert!(
        rows.iter().any(|r| r.contains("FIRST")),
        "Home must reach the oldest line the pane holds, got:\n{}",
        rows.join("\n")
    );
}

/// Every view a key can enter must say something on the frame it is entered.
///
/// The class, not the instance. `ui::draw` composes the host banner over row 0
/// of every pane, so a body written starting at row 0 loses its first line and
/// a *one-line* body is lost whole. `Panel::new` and `show_last_frame` reserved
/// that row; `Msg::Status` and `Msg::AuxBegin` did not, and were fixed; then
/// `toggle_fetch` and `toggle_docker` turned out not to either -- so pressing
/// `f` or `d` rendered the banner and an empty box, for as long as the fetch
/// took, or forever if the connection had hung. An empty box is exactly what
/// `a_connecting_host_says_connecting` exists to prevent.
///
/// This asserts the rule rather than the sites: enter each view, draw, and
/// require the pane to have something in it. `tools/check_row0_owner.py` is the
/// structural half -- it stops a sixth site being written at all.
#[test]
fn entering_a_view_puts_something_on_the_screen() {
    let _keychain = isolate_keychain();
    for (key, expect) in [
        ("f", "Fetching"),
        ("d", "Docker loading"),
        ("s", "waiting for data"),
    ] {
        let mut app = multitop::app::App::new(vec![multitop::config::Server {
            host: "web-01".to_string(),
            port: 22,
            user: "admin".to_string(),
            upgrade_cmd: None,
        }]);
        match key {
            "f" => drop(app.toggle_fetch()),
            "d" => drop(app.toggle_docker()),
            _ => drop(app.switch_stats()),
        }
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 8)).unwrap();
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
        assert!(
            rows.iter().any(|r| r.contains(expect)),
            "pressing `{key}` must say what it is doing; an empty pane is \
             indistinguishable from a hung connection. Got:\n{}",
            rows.join("\n")
        );
    }
}

/// A notice is drawn wherever the user is, not where they were when it arrived.
///
/// The class: `Panel::note` chose a pane from the mode **at write time**, and
/// `ui::pane_lines` chooses a pane from the mode **at draw time**. Nothing made
/// those agree. Every startup notice is written while the panels are in Monitor
/// mode, so it landed in `notes` -- which the Upgrade branch of `pane_lines` did
/// not draw at all. Pressing `u` erased all of them, and the sharpest one is a
/// notice *about the Upgrade pane*: that its scrollback was clamped, and why.
///
/// Asserted across every view a key can reach, rather than at the one site.
#[test]
fn a_notice_survives_every_view_switch() {
    const NOTICE: &str = "config: upgrade_history_lines = 0 would leave the Upgrade \
                          pane with nothing to show; using 50 instead.";
    let _keychain = isolate_keychain();
    let mut app = multitop::app::App::new(vec![multitop::config::Server {
        host: "web-01".to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: Some("apt upgrade".to_string()),
    }]);
    app.panels[0].note(NOTICE.to_string());

    // The notice's own distinguishing tail: the head of it survived the bug,
    // because the bug was about which pane is drawn, not about truncation.
    for view in ["monitor", "upgrade", "fetch", "docker", "back to monitor"] {
        match view {
            "upgrade" => app.enter_upgrade_view(),
            "fetch" => drop(app.toggle_fetch()),
            "docker" => drop(app.toggle_docker()),
            "back to monitor" => drop(app.switch_stats()),
            _ => {}
        }
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(70, 14)).unwrap();
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
        assert!(
            rows.iter().any(|r| r.contains("using 50 instead")),
            "the {view} view dropped a notice the user has to act on:\n{}",
            rows.join("\n")
        );
    }
}

/// Notices must never take the pane away from what they are about.
///
/// `MAX_NOTES` bounds how many notices a panel *keeps*, and its comment says the
/// bound exists so a repeated one "cannot crowd out the pane it is drawn in".
/// It could not do that: a pane's cost is in wrapped lines, and at forty columns
/// one notice is four of them. Four notices are sixteen rows; the pane is eleven.
/// Rendered at 40x12 over a live monitor frame, not one line of the machine was
/// on screen -- no cpu, no memory, no load, no uptime.
///
/// The rule asserted here is the one `pane_window` already applies to the pinned
/// block: half the pane at most, and say what was held back rather than dropping
/// it silently.
#[test]
fn notices_never_take_the_pane_from_the_host() {
    let _keychain = isolate_keychain();
    let mut app = multitop::app::App::new(vec![multitop::config::Server {
        host: "web-01".to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: None,
    }]);
    app.panels[0].show_frame(vec![
        String::new(),
        " cpu   12%  mem  41%".to_string(),
        " load  0.4 0.3 0.2".to_string(),
        " disk  61% of 500G".to_string(),
        " up    31 days".to_string(),
    ]);
    for n in 0..4 {
        app.panels[0].note(format!(
            "notice {n}: could not save upgrade state (Permission denied (os error 13)) \
             -- an interrupted run will not be detectable after a restart."
        ));
    }

    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
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
    let screen = rows.join("\n");

    assert!(
        rows.iter().any(|r| r.contains("cpu")),
        "the host the notices are about must still be on screen:\n{screen}"
    );
    assert!(
        rows.iter().any(|r| r.contains("notice 3")),
        "the newest notice must be on screen:\n{screen}"
    );
    assert!(
        rows.iter().any(|r| r.contains("earlier notices above")),
        "notices held back must be counted, not dropped in silence:\n{screen}"
    );

    // And held back is not thrown away: the twenty-seventh pass made every
    // notice reachable by `Home`, and this pass's bound does not get to undo it.
    app.scroll_to_top();
    term.draw(|f| multitop::ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer();
    let rows: Vec<String> = buf
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<Vec<_>>()
        .chunks(cols)
        .map(<[&str]>::concat)
        .collect();
    assert!(
        rows.iter().any(|r| r.contains("notice 0")),
        "Home must still reach the oldest notice:\n{}",
        rows.join("\n")
    );
}
