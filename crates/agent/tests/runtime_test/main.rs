//! The agent's three views and its repaint loop, driven into a byte sink.
//!
//! `run_agent` itself owns stdout and never returns in monitor mode, so the
//! parts worth pinning are the emitters and the loop: given a snapshot and a
//! writer, what goes on the wire, and what makes the loop stop.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{self, Write};

use multitop_agent::color::{ANSI, PLAIN};
use multitop_agent::docker::Row as DockerRow;
use multitop_agent::fetch::FetchSnapshot;
use multitop_agent::fmt::fullwidth;
use multitop_agent::monitor::Monitor;
use multitop_agent::proc::{Proc, Usage};
use multitop_agent::proto::{decode_packet, Payload};
use multitop_agent::render::Snapshot;
use multitop_agent::{
    emit_docker, emit_fetch, emit_monitor, monitor_loop, palette_for_env, parse_args, Args, Mode,
    SortBy,
};

/// A writer that fails on the nth write, standing in for the reader hanging up.
struct FailsAfter {
    writes_left: usize,
}

impl Write for FailsAfter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.writes_left == 0 {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "reader gone"));
        }
        self.writes_left -= 1;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn snapshot() -> Snapshot {
    Snapshot {
        host: "web-01".into(),
        agent_version: "9.9.9".into(),
        cpu_pct: 12.0,
        cpu_mhz: Some(3600.0),
        proc_names: Vec::new(),
        cores: vec![(0, 5.0, Some(40.0))],
        mem: Usage::new(8 << 30, 2 << 30),
        disk: Usage::new(256 << 30, 64 << 30),
        rx_rate: 1000.0,
        tx_rate: 2000.0,
        procs: vec![Proc {
            pid: 1,
            name: "init".into(),
            cpu: 1.0,
            mem: 1024,
        }],
        ..Default::default()
    }
}

fn fetch_snapshot() -> FetchSnapshot {
    FetchSnapshot {
        user_host: "root@web-01".into(),
        agent_version: "9.9.9".into(),
        os: "Debian GNU/Linux 12".into(),
        kernel: "6.1.0".into(),
        uptime: "3d 4h 5m".into(),
        host_model: "QEMU".into(),
        cpu_model: "AMD EPYC (8)".into(),
        memory_str: "2.0G/8.0G (25%)".into(),
        disk_str: "64.0G/256.0G (25%)".into(),
    }
}

fn docker_rows() -> Vec<DockerRow> {
    vec![
        DockerRow {
            name: "web".into(),
            status: "Up 3 days".into(),
            image: "nginx:latest".into(),
            cpu: "12.5%".into(),
            cpu_pct: 12.5,
            mem: "128.0M/512.0M".into(),
            mem_bytes: 134_217_728,
        },
        DockerRow {
            name: "db".into(),
            status: "Up 1 hour".into(),
            image: "nginx:latest".into(),
            cpu: "90.0%".into(),
            cpu_pct: 90.0,
            mem: "1.0G/2.0G".into(),
            mem_bytes: 1 << 30,
        },
    ]
}

// ------------------------------------------------------------------- fetch

mod cli_words_and_entry;
mod docker_table_edges;
mod event_loop_and_ticks;
mod render_views;
