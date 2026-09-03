//! Coverage-targeted tests for the five bug-fix areas.
//!
//! These exercise code paths that the regression tests don't reach, to push
//! multitop crate line coverage toward the 95% floor.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod config_and_filter_keys;
mod draw_and_frames;
mod handle_key;
mod keybar_state_eventloop;
mod packet_apply;
mod panels_upgrade_themes;
mod small_units;
mod spawn_upgrade;
mod ui_layout_cache_auxline_vault;

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use multitop::app::{App, Msg, VaultState};
use multitop::config::Server;
use multitop::panel::{Mode, Panel, RingLines, UpgradeState};
use multitop::password_store;
use ratatui::layout::{Rect, Size};
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio_stream::StreamExt;

use multitop::modals::Waiting;
use multitop::password_actions::apply;
use multitop::passwords::handle_key as passwords_handle_key;
use multitop::passwords::{open, PasswordAction, ServerDraft};
use multitop::run::{handle_key, panel_at_pos, Tasks};
use multitop::state::HostUpdate;
use multitop::ui::{agent_dims, keybar_badges, mode_pair, regions, visible, visible_upgrade};
use multitop_agent::fetch::FetchSnapshot;
use multitop_agent::render::Snapshot;
use multitop_agent::SortBy;
use multitop_vault::{Vault, VaultConfig};
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Style};
use secrecy::SecretString;

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 0,
        user: "testuser".to_string(),
        upgrade_cmd: Some("true".to_string()),
        custom_command: None,
    }
}

fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test();
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}
