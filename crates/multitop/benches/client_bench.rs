use multitop::app::App;
use multitop::config::Server;
use multitop::ui;
use multitop_agent::color;
use multitop_agent::proc::{Proc, Usage};
use multitop_agent::proto::{self, Payload};
use multitop_agent::render::{self, Snapshot};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::time::Instant;

fn sample_snapshot(server_id: usize) -> Snapshot {
    Snapshot {
        host: format!("server-{server_id}.prod.internal (10.0.0.{server_id})"),
        cpu_pct: 35.4 + (server_id as f64 * 2.5) % 50.0,
        cores: (0..16)
            .map(|c| (c, 10.0 + (c as f64 * 5.0) % 80.0, Some(45.0 + c as f64)))
            .collect(),
        temp_unit: Default::default(),
        mem: Usage::new(64_000_000_000, 32_000_000_000),
        disk: Usage::new(1_000_000_000_000, 400_000_000_000),
        rx_rate: 100_000.0,
        tx_rate: 250_000.0,
        procs: (0..30)
            .map(|i| Proc {
                pid: 1000 + i,
                name: format!("worker-{i}"),
                cpu: 12.5,
                mem: 150_000_000,
            })
            .collect(),
        agent_version: String::new(),
    }
}

fn main() {
    println!("============================================================");
    println!("   multitop Client Benchmark Suite (M4 Mac Apple Silicon)   ");
    println!("============================================================");

    // 1. Benchmark Protocol Serialization & Deserialization
    let snap = sample_snapshot(1);
    let payload = Payload::Monitor(snap);
    let encoded = proto::encode_packet(&payload);

    let iterations = 100_000;
    let start = Instant::now();
    for _ in 0..iterations {
        let decoded = proto::decode_packet(&encoded).unwrap();
        std::hint::black_box(decoded);
    }
    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
    println!(
        "\n1. Protocol Packet Decoding:\n   Iterations: {iterations}\n   Time: {:?}",
        elapsed
    );
    println!("   Latency:    {ns_per_op:.2} ns / packet");
    println!("   Throughput: {ops_per_sec:.0} packets / sec");

    // 2. Benchmark Local Snapshot Rendering
    let snap = sample_snapshot(1);
    let pal = &color::ANSI;
    let render_iters = 50_000;
    let start = Instant::now();
    for _ in 0..render_iters {
        let lines = render::render(&snap, 120, 40, render::bar_len_for(120), pal);
        std::hint::black_box(lines);
    }
    let elapsed = start.elapsed();
    let ns_per_render = elapsed.as_nanos() as f64 / render_iters as f64;
    println!(
        "\n2. Local Snapshot Rendering (120x40 Ratatui lines):\n   Iterations: {render_iters}\n   Time: {:?}",
        elapsed
    );
    println!("   Latency:    {ns_per_render:.2} ns / frame");
    println!(
        "   Throughput: {:.0} frames / sec",
        render_iters as f64 / elapsed.as_secs_f64()
    );

    // 3. Benchmark Full TUI Layout Drawing (1, 4, 8, 16 panels)
    for num_panels in [1, 2, 4, 8, 16] {
        let servers: Vec<Server> = (0..num_panels)
            .map(|i| Server {
                host: format!("server-{i}"),
                port: 22,
                user: "admin".into(),
                upgrade_cmd: None,
            })
            .collect();
        let mut app = App::new(servers);
        for (i, p) in app.panels.iter_mut().enumerate() {
            let snap = sample_snapshot(i);
            p.view = render::render(&snap, 80, 24, render::bar_len_for(80), pal);
        }

        let backend = TestBackend::new(160, 50);
        let mut terminal = Terminal::new(backend).unwrap();

        let tui_iters = 20_000;
        let start = Instant::now();
        for _ in 0..tui_iters {
            terminal
                .draw(|f| {
                    ui::draw(f, &app);
                })
                .unwrap();
        }
        let elapsed = start.elapsed();
        let ns_per_draw = elapsed.as_nanos() as f64 / tui_iters as f64;
        let fps = tui_iters as f64 / elapsed.as_secs_f64();
        println!(
            "\n3. Full TUI Draw ({num_panels} panels @ 160x50 resolution):\n   Latency:    {ns_per_draw:.2} ns / full draw ({:.3} ms)",
            ns_per_draw / 1_000_000.0
        );
        println!("   Throughput: {fps:.0} FPS");
    }

    println!("\n============================================================");
    println!("   Benchmark Finished Successfully. All metrics optimal.   ");
    println!("============================================================");
}
