use super::*;

// ------------------------------------------------------------- ordinary keys

#[tokio::test]
async fn the_sort_keys_only_restart_the_agents_when_the_sort_changes() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    let mut k = Keys::new(1);

    // Already sorting by CPU: pressing `c` changes nothing, so nothing is torn
    // down and rebuilt.
    assert_eq!(app.sort, multitop_agent::SortBy::Cpu);
    k.press(&mut app, KeyCode::Char('c'));
    assert_eq!(app.sort, multitop_agent::SortBy::Cpu);

    k.press(&mut app, KeyCode::Char('m'));
    assert_eq!(app.sort, multitop_agent::SortBy::Mem);
    k.press(&mut app, KeyCode::Char('M'));
    assert_eq!(app.sort, multitop_agent::SortBy::Mem);
    k.press(&mut app, KeyCode::Char('C'));
    assert_eq!(app.sort, multitop_agent::SortBy::Cpu);
}

#[tokio::test]
async fn paging_scrolls_further_than_a_single_step() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    let mut k = Keys::new(1);

    // Fill the pane so there is somewhere to scroll to. The offset counts
    // lines back from the bottom, so scrolling up raises it.
    for i in 0..100 {
        app.panels[0].last_upgrade.push(format!("line {i}"));
    }
    app.enter_upgrade_view();

    k.press(&mut app, KeyCode::End);
    assert_eq!(
        app.panels[0].scroll_offset, 0,
        "End is the bottom of the log"
    );

    k.press(&mut app, KeyCode::Up);
    let one_step = app.panels[0].scroll_offset;
    assert!(one_step > 0, "Up did not move");

    k.press(&mut app, KeyCode::End);
    k.press(&mut app, KeyCode::PageUp);
    assert!(
        app.panels[0].scroll_offset > one_step,
        "a page moved no further than a single line"
    );

    // And back down again by the same amounts.
    let paged = app.panels[0].scroll_offset;
    k.press(&mut app, KeyCode::Down);
    assert!(
        app.panels[0].scroll_offset < paged,
        "Down did not move back"
    );
    k.press(&mut app, KeyCode::PageDown);
    assert_eq!(
        app.panels[0].scroll_offset, 0,
        "a page down overshot the bottom"
    );

    // Home goes as far back as the log reaches.
    k.press(&mut app, KeyCode::Home);
    assert!(
        app.panels[0].scroll_offset > paged,
        "Home did not reach the top"
    );

    // `j` and `k` are the same keys by another name.
    k.press(&mut app, KeyCode::Char('j'));
    k.press(&mut app, KeyCode::Char('k'));
}

// -------------------------------------------------------- restarting agents

#[tokio::test]
async fn changing_the_sort_in_the_docker_view_restarts_the_docker_pollers_too() {
    // The monitor streams are restarted on any sort change. The docker pollers
    // carry the sort as well, and a panel left polling with the old one shows a
    // table ordered by something the keybar no longer says.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha", "beta"]);
    let mut k = Keys::new(2);

    let cmds = app.toggle_docker((80, 24));
    assert!(!cmds.is_empty(), "entering the docker view must spawn work");
    assert!(app.in_docker());

    k.press(&mut app, KeyCode::Char('m'));
    assert_eq!(app.sort, multitop_agent::SortBy::Mem);
    assert!(
        app.in_docker(),
        "the restart dropped the view the user was in"
    );

    // Back again, so both sort keys run against a docker grid.
    k.press(&mut app, KeyCode::Char('c'));
    assert_eq!(app.sort, multitop_agent::SortBy::Cpu);
    assert!(app.in_docker());
}

#[tokio::test]
async fn a_sort_change_outside_the_docker_view_starts_no_docker_pollers() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config(&dir, &["alpha"]);
    let mut k = Keys::new(1);

    assert!(!app.in_docker());
    k.press(&mut app, KeyCode::Char('m'));
    assert_eq!(app.sort, multitop_agent::SortBy::Mem);
    assert!(
        !app.in_docker(),
        "a sort change put the app in a view nobody asked for"
    );
}
