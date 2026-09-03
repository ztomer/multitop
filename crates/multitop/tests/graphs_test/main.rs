//! The `G` view: what a panel remembers, and how it draws it.
//!
//! Braille is easy to get subtly wrong -- the dot bits are not in the order the
//! pattern suggests, and the fourth row of each column is bolted on at the top
//! of the byte. Every glyph asserted below was worked out from the Unicode dot
//! numbering by hand, not read back off this implementation.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use multitop::app::{App, Msg};
use multitop::config::Server;
use multitop::graphs::{braille_rows, dots_for, render_graphs};
use multitop::history::{History, Series, SAMPLES};
use multitop::panel::Mode;
use multitop::password_store;
use multitop::run::{handle_key, Tasks};
use multitop_agent::proc::{Proc, Usage};
use multitop_agent::proto::Payload;
use multitop_agent::render::Snapshot;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};

const PLAIN: &multitop_agent::color::Palette = &multitop_agent::color::PLAIN;

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: Some("true".to_string()),
        custom_command: None,
    }
}

async fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

fn snapshot(cpu: f64, mem_used: u64, rx: f64, tx: f64) -> Snapshot {
    Snapshot {
        host: "web-01".into(),
        agent_version: "9.9.9".into(),
        cpu_pct: cpu,
        cpu_mhz: Some(3600.0),
        proc_names: Vec::new(),
        cores: vec![(0, cpu, None)],
        mem: Usage::new(100, mem_used),
        disk: Usage::new(100, 10),
        rx_rate: rx,
        tx_rate: tx,
        procs: vec![Proc {
            pid: 1,
            name: "init".into(),
            cpu: 1.0,
            mem: 1024,
        }],
        ..Default::default()
    }
}

// ------------------------------------------------------------------- history
mod history_and_render;
mod view_interaction;
