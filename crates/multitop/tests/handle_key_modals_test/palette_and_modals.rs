use super::*;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

#[tokio::test]
async fn test_command_palette_keys_and_execution() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("srv-alpha"), test_server("srv-beta")]);
    let mut keys = Keys::new(2);

    // Open palette
    keys.press(&mut app, KeyCode::Char(':'));
    assert!(app.command_palette_visible);

    // Type filter web and Enter
    keys.type_str(&mut app, "filter web");
    keys.press(&mut app, KeyCode::Enter);
    assert!(!app.command_palette_visible);
    assert_eq!(app.filter_query, "web");

    // Clear filter
    keys.press(&mut app, KeyCode::Char(':'));
    keys.type_str(&mut app, "clear filter");
    keys.press(&mut app, KeyCode::Enter);
    assert!(app.filter_query.is_empty());

    // Sort mem and sort cpu
    keys.press(&mut app, KeyCode::Char(':'));
    keys.type_str(&mut app, "sort mem");
    keys.press(&mut app, KeyCode::Enter);
    assert_eq!(app.sort, multitop_agent::SortBy::Mem);

    keys.press(&mut app, KeyCode::Char(':'));
    keys.type_str(&mut app, "sort cpu");
    keys.press(&mut app, KeyCode::Enter);
    assert_eq!(app.sort, multitop_agent::SortBy::Cpu);

    // Docker and fetch
    keys.press(&mut app, KeyCode::Char(':'));
    keys.type_str(&mut app, "docker");
    keys.press(&mut app, KeyCode::Enter);

    keys.press(&mut app, KeyCode::Char(':'));
    keys.type_str(&mut app, "fetch");
    keys.press(&mut app, KeyCode::Enter);

    // Graphs and stats
    keys.press(&mut app, KeyCode::Char(':'));
    keys.type_str(&mut app, "graphs");
    keys.press(&mut app, KeyCode::Enter);

    keys.press(&mut app, KeyCode::Char(':'));
    keys.type_str(&mut app, "stats");
    keys.press(&mut app, KeyCode::Enter);

    // Theme and yank
    keys.press(&mut app, KeyCode::Char(':'));
    keys.type_str(&mut app, "theme");
    keys.press(&mut app, KeyCode::Enter);

    keys.press(&mut app, KeyCode::Char(':'));
    keys.type_str(&mut app, "yank");
    keys.press(&mut app, KeyCode::Enter);

    // Esc and Backspace in palette
    keys.press(&mut app, KeyCode::Char(':'));
    keys.type_str(&mut app, "foo");
    keys.press(&mut app, KeyCode::Backspace);
    assert_eq!(app.command_input, "fo");
    keys.press(&mut app, KeyCode::Esc);
    assert!(!app.command_palette_visible);
}

#[tokio::test]
async fn test_draw_all_modals() {
    let _g = isolate().await;
    let backend = TestBackend::new(80, 24);
    let mut term = Terminal::new(backend).unwrap();

    let mut app = App::new(vec![test_server("srv-alpha")]);
    app.command_palette_visible = true;
    app.command_input = "fil".to_string();

    term.draw(|f| {
        multitop::modals::draw_command_palette(f, &app);
    })
    .unwrap();

    term.draw(|f| {
        multitop::modals::draw_help(f);
    })
    .unwrap();

    term.draw(|f| {
        multitop::modals::draw_vault_awaiting_biometric(f, multitop::modals::Waiting::Biometric);
    })
    .unwrap();
    term.draw(|f| {
        multitop::modals::draw_vault_awaiting_biometric(f, multitop::modals::Waiting::Verifying);
    })
    .unwrap();
    term.draw(|f| {
        multitop::modals::draw_vault_awaiting_biometric(f, multitop::modals::Waiting::Creating);
    })
    .unwrap();
}
