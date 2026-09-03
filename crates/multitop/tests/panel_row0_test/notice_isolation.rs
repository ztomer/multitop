use super::*;
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
        custom_command: None,
    }]);
    app.panels[0].note(NOTICE.to_string());

    // The notice's own distinguishing tail: the head of it survived the bug,
    // because the bug was about which pane is drawn, not about truncation.
    for view in ["monitor", "upgrade", "fetch", "docker", "back to monitor"] {
        match view {
            "upgrade" => {
                let _ = app.enter_upgrade_view();
            }
            "fetch" => drop(app.toggle_fetch((80, 24))),
            "docker" => drop(app.toggle_docker((80, 24))),
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
        custom_command: None,
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

/// "N earlier notices above" must be true.
///
/// `visible_upgrade` derived its pinned count as `header.len()`, which was right
/// exactly as long as the header held nothing but pinned content. The
/// twenty-ninth pass then appended the *held* notices to it -- lines whose whole
/// purpose is to sit above the content and be reached by scrolling -- and the
/// clamp pinned them. At 40x31 the pane drew "1 earlier notice above" with that
/// notice four rows higher, on screen, and gave sixteen of thirty rows to
/// notices while the live log got six.
///
/// The class, for the third pass running: a quantity derived from something that
/// stopped being the right thing to derive it from. Measuring a slice is a
/// derivation like any other.
#[test]
fn a_held_notice_is_above_the_content_not_on_screen_twice() {
    let _keychain = isolate_keychain();
    let mut app = multitop::app::App::new(vec![multitop::config::Server {
        host: "web-01".to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: Some("apt upgrade".to_string()),
        custom_command: None,
    }]);
    for n in 0..4 {
        app.panels[0].note(format!(
            "notice {n}: could not save upgrade state (Permission denied (os error 13)) \
             -- an interrupted run will not be detectable after a restart."
        ));
    }
    app.enter_upgrade_view();
    for i in 0..30 {
        app.panels[0].last_upgrade.push(format!("log line {i}"));
    }

    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 31)).unwrap();
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
        rows.iter().any(|r| r.contains("earlier notice above")),
        "the pane must say what it held back:\n{screen}"
    );
    assert!(
        !rows.iter().any(|r| r.contains("notice 0")),
        "a notice reported as ABOVE the content must not also be on screen:\n{screen}"
    );
    assert!(
        rows.iter().filter(|r| r.contains("log line")).count() >= 8,
        "the live log is what this pane is for; notices must not take it:\n{screen}"
    );

    // And "above" has to mean reachable.
    app.scroll_to_top();
    term.draw(|f| multitop::ui::draw(f, &mut app)).unwrap();
    let rows: Vec<String> = term
        .backend()
        .buffer()
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<Vec<_>>()
        .chunks(cols)
        .map(<[&str]>::concat)
        .collect();
    assert!(
        rows.iter().any(|r| r.contains("notice 0")),
        "Home must reach the notice the pane said was above it:\n{}",
        rows.join("\n")
    );
}
