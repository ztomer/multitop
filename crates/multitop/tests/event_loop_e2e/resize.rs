//! The render size is derived from the terminal size AND the panel count,
//! so a server edit changes it without a `Resize` ever arriving.

use super::*;

/// Removing a server changes the grid -- three panels are two columns, two are
/// one -- so it changes the size every pane gets. The agents render into that
/// size, and they have to be told.
///
/// This failed before the fix: the render size was recomputed only when a
/// `Resize` arrived, so an edit to the server list left every agent drawing for
/// the old grid until the user happened to resize the window.
#[tokio::test]
async fn removing_a_server_resizes_what_the_agents_render() {
    let _keychain = isolate_keychain().await;
    let size = (100, 30);
    let servers = vec![
        test_server("alpha.example"),
        test_server("beta.example"),
        test_server("gamma.example"),
    ];

    let mut h = Harness::start(
        servers,
        size,
        vec![
            // Settings, then remove the selected host: `d` asks, `y` answers.
            key(KeyCode::Char('e')),
            key(KeyCode::Char('d')),
            key(KeyCode::Char('y')),
        ],
    );

    // Not asserted on the way through: the loop consumes the scripted burst
    // faster than the test can observe an intermediate value, and the watch
    // channel keeps only the newest. What matters is where it lands.
    assert_ne!(
        dims_for(size, 3),
        dims_for(size, 2),
        "this test is only meaningful while the panel count changes the size"
    );
    h.expect_dims(dims_for(size, 2), "after the removal").await;
}

/// The render size is derived from the terminal size *and* the panel count.
///
/// The count used to be captured before the first frame and never updated, so
/// the next resize after a server edit recomputed the size from the old count
/// and put the wrong value back -- the one case where resizing the window made
/// the display worse.
#[tokio::test]
async fn a_resize_after_a_server_edit_uses_the_new_count() {
    let _keychain = isolate_keychain().await;
    let size = (100, 30);
    let servers = vec![
        test_server("alpha.example"),
        test_server("beta.example"),
        test_server("gamma.example"),
    ];

    let mut h = Harness::start(
        servers,
        size,
        vec![
            key(KeyCode::Char('e')),
            key(KeyCode::Char('d')),
            key(KeyCode::Char('y')),
            key(KeyCode::Esc),
            // Same terminal size: what is under test is the count, not the size.
            Event::Resize(size.0, size.1),
        ],
    );

    h.expect_dims(dims_for(size, 2), "after the removal").await;
    // Long enough for the resize debounce to fire and publish, if it is going
    // to publish anything at all.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        *h.dims.borrow(),
        dims_for(size, 2),
        "the resize recomputed the render size from the panel count the app \
         started with, not the one it has"
    );
}
