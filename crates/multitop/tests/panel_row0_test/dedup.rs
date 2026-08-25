use super::*;
/// One notice is one notice, however many views it arrived in.
///
/// `Panel::note` branched on the panel's mode: the ring in the Upgrade view,
/// `notes` everywhere else. Once every pane drew `notes` (the twenty-eighth
/// pass), that branch stopped hiding notices and started duplicating them. A
/// state write that fails once in the Monitor view and again during a run left
/// one copy in each buffer, and the Upgrade pane drew both -- the ring copy
/// hard-truncated mid-word, because the ring is fitted to the pane rather than
/// wrapped, which is the very defect the wrapping exists to prevent.

#[test]
fn one_notice_is_one_notice_across_views() {
    const N: &str = "could not save upgrade state (Permission denied) -- an \
                     interrupted run will not be detectable after a restart.";
    let _keychain = isolate_keychain();
    let mut app = multitop::app::App::new(vec![multitop::config::Server {
        host: "web-01".to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: Some("apt upgrade".to_string()),
    }]);
    app.panels[0].note(N.to_string());
    app.enter_upgrade_view();
    app.panels[0].note(N.to_string());

    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 16)).unwrap();
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

    assert_eq!(
        rows.iter()
            .filter(|r| r.contains("could not save upgrade state"))
            .count(),
        1,
        "the same notice must occupy one row, not one per buffer it reached:\n{screen}"
    );
    // And the surviving copy is the wrapped one, which keeps its ending.
    assert!(
        rows.iter().any(|r| r.contains("after a restart")),
        "a notice must not lose its ending to the pane's hard truncation:\n{screen}"
    );
}
