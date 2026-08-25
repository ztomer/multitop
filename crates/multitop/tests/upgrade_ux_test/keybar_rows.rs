use super::*;

/// The one chunk of the confirm row an operator cannot guess is the way out,
/// and it must never be what the width budget drops.
///
/// The shed list used to be built by position -- `shed.push(2)` -- while which
/// index held what depended on whether the optional `· N skipped` chunk was
/// present at all. With nothing skipped, index 2 *was* `[Esc] cancel`, so the
/// first row too narrow to fit shed its own cancel instruction: the exact
/// defect (`Esc t`) the row was built to remove, rebuilt.
#[tokio::test]
async fn the_confirm_row_never_sheds_its_own_way_out() {
    let _k = isolate_keychain_async().await;
    let mut h = Harness::new(vec![
        server("web-01", Some("apt upgrade")),
        server("web-02", Some("apt upgrade")),
    ]);
    h.press('u');
    h.press('u');
    assert!(h.app.show_upgrade_modal(), "the confirmation must be armed");

    for width in 10..=100u16 {
        let row = keybar_text(&h.app, width);
        assert!(
            row.contains("[Esc] cancel"),
            "at {width} columns the row lost its cancel instruction: {row:?}"
        );
        assert!(
            !row.contains("[Esc] canc") || row.contains("[Esc] cancel"),
            "and never a fragment of it: {row:?}"
        );
    }
}

/// The row is assembled from whole chunks against a budget, so it must never be
/// wider than the keybar it is drawn into -- whichever chunks survive.
#[tokio::test]
async fn no_confirm_row_overruns_the_keybar_width() {
    let _k = isolate_keychain_async().await;
    let mut h = Harness::new(vec![
        server("web-01", Some("apt upgrade")),
        server("db-02", None),
        server("db-03", None),
    ]);
    h.press('u');
    h.press('u');
    for width in 24..=120u16 {
        let row = keybar_text(&h.app, width);
        let cells = row.chars().count();
        assert!(
            cells <= width as usize,
            "the armed confirm row used {cells} cells of {width}: {row:?}"
        );
    }
}

/// A previous run that never finished is the one fact on this screen that
/// appears nowhere else. The box this row replaced said so; deleting the box
/// dropped the warning with it, which was not what the ruling decided.
#[tokio::test]
async fn the_confirm_row_warns_that_a_previous_run_never_finished() {
    let _k = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    // A run was stamped as started and no completion ever landed.
    h.app.upgrade_started_at = Some(1_722_000_000);
    h.app.last_update = None;
    h.press('u');
    h.press('u');

    let row = keybar_text(&h.app, 100);
    assert!(
        row.contains("previous run interrupted"),
        "an unfinished previous run must be stated before starting another: {row:?}"
    );

    // A completed run afterwards clears it.
    h.app.last_update = Some(1_722_000_600);
    let row = keybar_text(&h.app, 100);
    assert!(
        !row.contains("previous run interrupted"),
        "a run that finished is not an interrupted one: {row:?}"
    );
}

/// The quit confirmation kills a live `apt upgrade` on N production hosts. It
/// must act on the keys it names and nothing else -- `Enter` is what an
/// operator presses to dismiss something they have not read.
#[tokio::test]
async fn the_quit_confirmation_ignores_keys_it_does_not_name() {
    let _k = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    assert!(h.app.upgrades_in_flight());

    h.press_key(KeyCode::Esc);
    assert!(h.app.quit_armed(), "the first press asks rather than kills");

    h.press_key(KeyCode::Enter);
    assert!(!h.app.should_quit(), "Enter must not confirm a kill");
    h.press('y');
    assert!(!h.app.should_quit(), "nor y, which the row does not name");
    assert!(h.app.quit_armed(), "and the question is still standing");

    let row = keybar_text(&h.app, 100);
    assert!(row.contains("[Q] quit anyway"), "row: {row:?}");
    assert!(row.contains("[Esc] stay"), "row: {row:?}");
    assert!(row.contains("web-01"), "the host at risk is named: {row:?}");

    h.press('q');
    assert!(h.app.should_quit(), "the key the row names does confirm");
}

/// The upgrade confirmation runs `apt upgrade` on every visible host. Like the
/// quit confirmation above, it must act on the keys it names and nothing else.
///
/// It named `[U] go  [Esc] cancel` and also confirmed on `y`, `Y` and `Enter` --
/// three keys that start package transactions on production machines without
/// appearing anywhere on the screen that asked. `Enter` is the worst of them,
/// for the same reason the quit row dropped it: it is what an operator hits to
/// dismiss a row they have not read.
///
/// Extra *cancel* keys are deliberately not part of this rule. A stray key that
/// cancels can only ever be the safe answer.
#[tokio::test]
async fn the_upgrade_confirmation_ignores_keys_it_does_not_name() {
    let _k = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    h.press('u');
    h.press('u');
    assert!(h.app.show_upgrade_modal(), "the question is up");

    let row = keybar_text(&h.app, 100);
    assert!(row.contains("[U] go"), "row: {row:?}");
    assert!(row.contains("[Esc] cancel"), "row: {row:?}");
    assert!(
        !row.contains("[y]") && !row.contains("[Enter]"),
        "the row offers neither: {row:?}"
    );

    h.press_key(KeyCode::Enter);
    assert!(
        !h.app.upgrades_in_flight(),
        "Enter must not start an upgrade it was never offered for"
    );
    h.press('y');
    assert!(
        !h.app.upgrades_in_flight(),
        "nor y, which the row does not name"
    );
    assert!(
        h.app.show_upgrade_modal(),
        "and the question is still standing"
    );

    h.press('u');
    assert!(
        h.app.upgrades_in_flight(),
        "the key the row names does confirm"
    );
}
