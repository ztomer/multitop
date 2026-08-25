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
mod banner_and_views;
mod dedup;
mod notice_isolation;
