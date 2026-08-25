// ===========================================================================
// passwords.rs — ServerDraft validation (public)
// ===========================================================================

#[test]
fn server_draft_valid() {
    use multitop::passwords::ServerDraft;
    let draft = ServerDraft {
        original: None,
        host: "valid-host".into(),
        user: "admin".into(),
        port: "22".into(),
        upgrade_cmd: "sudo apt update".into(),
        password: String::new(),
        field: 0,
    };
    let server = draft.into_server().expect("valid");
    assert_eq!(server.host, "valid-host");
    assert_eq!(server.port, 22);
}

#[test]
fn server_draft_invalid_port() {
    use multitop::passwords::ServerDraft;
    let draft = ServerDraft {
        original: None,
        host: "valid-host".into(),
        user: "admin".into(),
        port: "abc".into(),
        upgrade_cmd: "cmd".into(),
        password: String::new(),
        field: 0,
    };
    assert!(draft.into_server().is_err());
}

#[test]
fn server_draft_port_zero() {
    use multitop::passwords::ServerDraft;
    let draft = ServerDraft {
        original: None,
        host: "valid-host".into(),
        user: "admin".into(),
        port: "0".into(),
        upgrade_cmd: "cmd".into(),
        password: String::new(),
        field: 0,
    };
    assert!(draft.into_server().is_err());
}

// ===========================================================================
// config.rs — validation (public)
// ===========================================================================

#[test]
fn config_validate_host() {
    assert!(multitop::config::validate_host("has space").is_err());
    assert!(multitop::config::validate_host("valid-host").is_ok());
}

#[test]
fn config_validate_user() {
    assert!(multitop::config::validate_user("has space").is_err());
    assert!(multitop::config::validate_user("validuser").is_ok());
}

// ===========================================================================
// ui.rs — layout + windowing (public)
// ===========================================================================

#[test]
fn regions_layout() {
    use ratatui::layout::Rect;
    let area = Rect::new(0, 0, 80, 24);
    let (panels, keybar) = multitop::ui::regions(area, 4);
    assert_eq!(panels.len(), 4);
    assert_eq!(keybar.height, 1);
}

#[test]
fn regions_single_panel() {
    use ratatui::layout::Rect;
    let area = Rect::new(0, 0, 80, 24);
    let (panels, _) = multitop::ui::regions(area, 1);
    assert_eq!(panels.len(), 1);
    assert_eq!(panels[0].width, 80);
}

#[test]
fn regions_empty() {
    use ratatui::layout::Rect;
    let area = Rect::new(0, 0, 80, 24);
    let (panels, _) = multitop::ui::regions(area, 0);
    assert_eq!(panels, [] as [ratatui::prelude::Rect; 0]);
}

#[test]
fn visible_shows_all_when_fits() {
    let lines = vec!["a".into(), "b".into(), "c".into()];
    let (out, badge) = multitop::ui::visible(&lines, 10, 1, 80, 0);
    assert_eq!(out.len(), 3);
    assert_eq!(badge, 0);
}

#[test]
fn visible_clamps_height() {
    let lines: Vec<String> = (0..20).map(|i| format!("line{i}")).collect();
    let (out, _) = multitop::ui::visible(&lines, 5, 1, 80, 0);
    assert!(out.len() <= 5);
}

#[test]
fn visible_scrolls_with_offset() {
    let lines: Vec<String> = (0..20).map(|i| format!("line{i}")).collect();
    let (out, badge) = multitop::ui::visible(&lines, 10, 1, 80, 5);
    assert!(badge > 0);
    assert!(out.len() <= 10);
}

#[test]
fn visible_zero_height() {
    let (out, badge) = multitop::ui::visible(&["a".into()], 0, 1, 80, 0);
    assert_eq!(out, [] as [std::string::String; 0]);
    assert_eq!(badge, 0);
}

#[test]
fn visible_upgrade_composes() {
    let header = vec!["Status: ready".into()];
    let mut body = multitop::panel::RingLines::new(100);
    body.push("output".into());
    let tail = vec!["notice".into()];
    let (out, badge) = multitop::ui::visible_upgrade(&header, 1, &body, &tail, 10, 80, 0);
    assert_ne!(out, [] as [std::string::String; 0]);
    assert_eq!(badge, 0);
}

#[test]
fn keybar_badges_shed() {
    use multitop_agent::SortBy;
    use ratatui::style::{Color, Style};
    let pal = &multitop_agent::color::THEMES[0];
    let label = Style::default().fg(Color::DarkGray);
    let key_hi = Style::default().fg(Color::White);
    let sort_label = Style::default().fg(Color::DarkGray);
    let badges =
        multitop::ui::keybar_badges(SortBy::Cpu, pal, label, key_hi, sort_label, Color::Yellow);
    assert_eq!(badges.len(), 3);
}

#[test]
fn mode_pair_highlights_active() {
    use multitop::app::Mode;
    use multitop::ui::mode_pair;
    use ratatui::style::{Color, Style};
    let active = Style::default().fg(Color::Black);
    let off = Style::default().fg(Color::White);
    let (k, l) = mode_pair(Mode::Docker, Mode::Docker, active, off, off);
    assert_eq!(k, active);
    assert_eq!(l, active);
}

// ===========================================================================
// agent_dims — public
// ===========================================================================

#[test]
fn agent_dims_minimum() {
    use ratatui::layout::Size;
    let dims = multitop::ui::agent_dims(
        Size {
            width: 10,
            height: 4,
        },
        0,
    );
    assert!(dims.0 >= multitop::ui::MIN_AGENT_COLS);
    assert!(dims.1 >= multitop::ui::MIN_AGENT_ROWS);
}
