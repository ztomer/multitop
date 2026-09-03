use super::*;

#[test]
fn the_scroll_badge_survives_into_the_rendered_frame() {
    let _keychain = isolate_keychain();
    let mut app = multitop::app::App::new(vec![multitop::config::Server {
        host: "web-01".to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: None,
        custom_command: None,
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
        custom_command: None,
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
                custom_command: None,
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
        custom_command: None,
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
            custom_command: None,
        }]);
        match key {
            "f" => drop(app.toggle_fetch((80, 24))),
            "d" => drop(app.toggle_docker((80, 24))),
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
