//! Edges the ordinary paths never reach: a zero width, a key pressed with
//! nothing open to receive it, a tie in a sort, a pool that ran out.
//!
//! Small branches, but each is a `return` or a `continue` that only runs when
//! something is degenerate — which is exactly when a panic or a wrong answer
//! is least welcome and least likely to be noticed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::KeyCode;
use multitop::app::App;
use multitop::config::Server;
use multitop::layout::share_width;
use multitop::password_store;
use multitop::passwords::{handle_key, open, PasswordAction};
use multitop_agent::color::PLAIN;
use multitop_agent::docker::{render, Row};
use multitop_agent::SortBy;

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: Some("true".to_string()),
    }
}

async fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

// ------------------------------------------------------ the settings editor
mod agent_and_vault;
mod passwords_and_layout;
mod surplus_and_cells;
