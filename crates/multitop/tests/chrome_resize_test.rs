//! Pedantic regression tests for the terminal resize → re-render path.
//!
//! The critical invariant: resizing the terminal window must NOT restart SSH
//! agents. Instead, monitor tasks re-read dimensions from a shared watch
//! channel every frame and re-render at the current size. These tests verify
//! that every layer of this path is wired correctly.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::sync::watch;

use multitop::app::Msg;
use multitop::render_payload::render_payload;
use multitop_agent::color;
use multitop_agent::proc::{Proc, Usage};
use multitop_agent::proto::Payload;
use multitop_agent::render::Snapshot;
use multitop_agent::SortBy;

fn sample_snapshot() -> Snapshot {
    Snapshot {
        host: "test-host.example.com (10.0.0.1)".into(),
        agent_version: "0.0.0".into(),
        cpu_pct: 42.0,
        cores: (0..8)
            .map(|c| (c, 10.0 + c as f64 * 8.0, Some(50.0 + c as f64)))
            .collect(),
        temp_unit: Default::default(),
        mem: Usage::new(16_000_000_000, 8_000_000_000),
        disk: Usage::new(500_000_000_000, 200_000_000_000),
        rx_rate: 50_000.0,
        tx_rate: 150_000.0,
        procs: (0..15)
            .map(|i| Proc {
                pid: 2000 + i,
                name: format!("svc-{i}"),
                cpu: 5.0,
                mem: 80_000_000,
            })
            .collect(),
    }
}

fn sample_fetch() -> Payload {
    Payload::Fetch(multitop_agent::fetch::FetchSnapshot {
        user_host: "admin@test-host".into(),
        agent_version: "0.0.0".into(),
        os: "Ubuntu 24.04 LTS".into(),
        kernel: "6.8.0-45-generic".into(),
        uptime: "12d 3h".into(),
        host_model: "Test Model".into(),
        cpu_model: "Test CPU (8)".into(),
        memory_str: "8.0GiB/16.0GiB (50%)".into(),
        disk_str: "200.0GiB/500.0GiB (40%)".into(),
    })
}

fn sample_docker() -> Payload {
    Payload::Docker {
        host: "test-host.example.com".into(),
        rows: (0..20)
            .map(|i| multitop_agent::docker::Row {
                name: format!("container-{i}"),
                status: if i % 3 == 2 {
                    "Exited (0)".into()
                } else {
                    "Up 3 hours".into()
                },
                cpu: format!("{:.1}%", i as f64 * 0.5),
                cpu_pct: i as f64 * 0.5,
                mem: format!("{}MiB / {}MiB", 64 + i * 8, 256 + i * 16),
                mem_bytes: (64 + i * 8) << 20,
            })
            .collect(),
    }
}

fn frame_lines(msg: Msg) -> Vec<String> {
    match msg {
        Msg::Frame { lines, .. } => lines,
        _ => panic!("expected Msg::Frame, got {msg:?}"),
    }
}

// ---------------------------------------------------------------------------
// Pure-function tests
// ---------------------------------------------------------------------------

#[test]
fn render_payload_monitor_produces_more_lines_at_larger_dims() {
    let payload = Payload::Monitor(sample_snapshot());

    let small = render_payload(&payload, (40, 10), SortBy::Cpu, &color::ANSI);
    let large = render_payload(&payload, (200, 60), SortBy::Cpu, &color::ANSI);

    assert!(
        large.len() > small.len(),
        "wider+taller dims should produce more rendered lines: {} vs {}",
        large.len(),
        small.len()
    );
}

#[test]
fn render_payload_fetch_produces_more_lines_at_larger_dims() {
    let payload = sample_fetch();

    let small = render_payload(&payload, (40, 5), SortBy::Cpu, &color::ANSI);
    let large = render_payload(&payload, (80, 24), SortBy::Cpu, &color::ANSI);

    assert!(
        large.len() > small.len(),
        "taller dims should show more fetch detail rows: {} vs {}",
        large.len(),
        small.len()
    );
}

#[test]
fn render_payload_docker_produces_more_lines_at_larger_dims() {
    let payload = sample_docker();

    let small = render_payload(&payload, (40, 6), SortBy::Cpu, &color::ANSI);
    let large = render_payload(&payload, (80, 24), SortBy::Cpu, &color::ANSI);

    assert!(
        large.len() > small.len(),
        "taller dims should show more docker rows: {} vs {}",
        large.len(),
        small.len()
    );
}

#[test]
fn render_payload_plain_palette_has_no_ansi_escapes() {
    let payload = Payload::Monitor(sample_snapshot());
    let lines = render_payload(&payload, (80, 24), SortBy::Cpu, &color::PLAIN);
    for (i, line) in lines.iter().enumerate() {
        assert!(
            !line.contains('\x1b'),
            "plain palette should produce no escapes at line {i}: {line:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Watch-channel integration tests
// ---------------------------------------------------------------------------

#[test]
fn watch_channel_carries_dims_update() {
    let (tx, rx) = watch::channel((80u16, 24u16));

    assert_eq!(*rx.borrow(), (80, 24));

    tx.send((120, 40)).unwrap();
    assert_eq!(*rx.borrow(), (120, 40));

    tx.send((40, 8)).unwrap();
    assert_eq!(*rx.borrow(), (40, 8));
}

#[tokio::test]
async fn monitor_loop_re_reads_dims_each_frame() {
    let (payload_tx, mut payload_rx) = mpsc::unbounded_channel::<Payload>();
    let (msg_tx, mut msg_rx) = mpsc::channel::<Msg>(16);
    let (dims_tx, dims_rx) = watch::channel((40u16, 10u16));
    let dims_rx = Arc::new(dims_rx);

    tokio::spawn(async move {
        let pal = &color::ANSI;
        while let Some(payload) = payload_rx.recv().await {
            let dims = *dims_rx.borrow();
            let lines = render_payload(&payload, dims, SortBy::Cpu, pal);
            if msg_tx.send(Msg::Frame { panel: 0, lines }).await.is_err() {
                break;
            }
        }
    });

    let snap = Payload::Monitor(sample_snapshot());

    payload_tx.send(snap.clone()).unwrap();
    let frame1 = tokio::time::timeout(Duration::from_secs(1), msg_rx.recv())
        .await
        .expect("timeout waiting for frame")
        .expect("channel closed");
    let count1 = frame_lines(frame1).len();

    dims_tx.send((160, 60)).unwrap();

    payload_tx.send(snap).unwrap();
    let frame2 = tokio::time::timeout(Duration::from_secs(1), msg_rx.recv())
        .await
        .expect("timeout waiting for frame")
        .expect("channel closed");
    let count2 = frame_lines(frame2).len();

    assert!(
        count2 > count1,
        "monitor loop must re-read dims each frame: \
         count at 40x10 = {count1}, count at 160x60 = {count2}"
    );
}

// ---------------------------------------------------------------------------
// Structural test: the only caller of restart_all_agents is handle_key,
// which only fires it for C/M sort keys. Resize events must NEVER reach it.
// ---------------------------------------------------------------------------

#[test]
fn resize_path_must_not_call_restart_all_agents() {
    let run_source = include_str!("../src/run.rs");

    let restart_call_lines: Vec<usize> = run_source
        .lines()
        .enumerate()
        .filter(|(_, l)| l.contains("restart_all_agents"))
        .map(|(i, _)| i + 1)
        .collect();

    assert!(
        restart_call_lines.len() >= 2,
        "expected at least 2 calls (C and M key handlers), found {} \
         — if you renamed or removed it, check the resize path isn't calling it",
        restart_call_lines.len()
    );

    for line_no in &restart_call_lines {
        let start = line_no.saturating_sub(3);
        let end = (*line_no + 2).min(run_source.lines().count());
        let context: Vec<&str> = run_source.lines().skip(start).take(end - start).collect();
        let ctx_block = context.join("\n");
        assert!(
            !ctx_block.to_lowercase().contains("resize"),
            "restart_all_agents at line {line_no} is near 'resize' context:\n{ctx_block}"
        );
    }
}

// ---------------------------------------------------------------------------
// Every Payload variant renders without panic
// ---------------------------------------------------------------------------

#[test]
fn render_payload_handles_every_variant() {
    let pal = &color::ANSI;
    let dims = (80, 24);
    let sort = SortBy::Cpu;

    let monitor = render_payload(&Payload::Monitor(sample_snapshot()), dims, sort, pal);
    assert!(!monitor.is_empty(), "Monitor payload should produce output");

    let fetch = render_payload(&sample_fetch(), dims, sort, pal);
    assert!(!fetch.is_empty(), "Fetch payload should produce output");

    let docker = render_payload(&sample_docker(), dims, sort, pal);
    assert!(!docker.is_empty(), "Docker payload should produce output");
}
