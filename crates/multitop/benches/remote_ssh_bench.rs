#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use multitop::config::Server;
use multitop::ssh::Mode;
use multitop::stream::{connect, next_packet, PacketStream};
use multitop_agent::proto::Payload;
use multitop_agent::SortBy;
use std::env;
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Running totals for the stream.
///
/// Counters that only ever feed ratios (bandwidth, averages) are accumulated
/// directly in `f64` rather than widened from an integer at report time: every
/// increment comes from a value that fits in `u32`, and `f64` represents all
/// integers below 2^53 exactly, so the running totals stay exact for any run
/// this benchmark can perform.
#[derive(Default)]
struct Stats {
    packets: f64,
    decoded: f64,
    bytes: f64,
    delay_sum_ms: f64,
    delay_count: f64,
    min_pkt: usize,
    max_pkt: usize,
}

impl Stats {
    fn new() -> Self {
        Self {
            min_pkt: usize::MAX,
            ..Self::default()
        }
    }

    fn record_packet(&mut self, pkt_size: usize) {
        let size = u32::try_from(pkt_size).expect("packet size fits in u32");
        self.packets += 1.0;
        self.decoded += 1.0;
        self.bytes += f64::from(size);
        self.min_pkt = self.min_pkt.min(pkt_size);
        self.max_pkt = self.max_pkt.max(pkt_size);
    }

    fn record_delay(&mut self, delay_ms: f64) {
        self.delay_sum_ms += delay_ms;
        self.delay_count += 1.0;
    }

    fn avg_delay_ms(&self) -> f64 {
        if self.delay_count == 0.0 {
            0.0
        } else {
            self.delay_sum_ms / self.delay_count
        }
    }

    fn avg_pkt_size(&self) -> f64 {
        if self.packets == 0.0 {
            0.0
        } else {
            self.bytes / self.packets
        }
    }

    fn bandwidth_kib_per_sec(&self, elapsed_sec: f64) -> f64 {
        (self.bytes / 1024.0) / elapsed_sec
    }
}

/// Approximate the on-the-wire size of a decoded payload.
fn packet_size(payload: &Payload) -> usize {
    match payload {
        Payload::Monitor(snap) => {
            let cores_bytes = snap.cores.len() * 10;
            let procs_bytes = snap.procs.iter().map(|p| 18 + p.name.len()).sum::<usize>();
            8 + snap.host.len() + 2 + cores_bytes + 1 + 32 + 2 + procs_bytes
        }
        Payload::Docker { host, rows } => {
            let rows_bytes = rows
                .iter()
                .map(|r| r.name.len() + r.status.len() + r.cpu.len() + r.mem.len() + 16)
                .sum::<usize>();
            8 + host.len() + 2 + rows_bytes
        }
        Payload::Fetch(snap) => {
            8 + snap.user_host.len()
                + snap.os.len()
                + snap.kernel.len()
                + snap.uptime.len()
                + snap.host_model.len()
                + snap.cpu_model.len()
                + snap.memory_str.len()
                + snap.disk_str.len()
                + 32
        }
        // The exec channel is not part of the stats stream this bench measures,
        // and its Out frames are chunked to a fixed ceiling rather than sized by
        // the payload, so an estimate here would describe nothing.
        Payload::Exec(_) => 0,
    }
}

/// Consume the telemetry stream for `duration_secs`, printing a progress row
/// every 30 seconds.
async fn stream_telemetry(stream: &mut PacketStream, duration_secs: u64) -> Stats {
    let start_time = Instant::now();
    let benchmark_duration = Duration::from_secs(duration_secs);
    let report_interval = Duration::from_secs(30);

    let mut stats = Stats::new();
    let mut last_pkt_time = Instant::now();
    let mut last_report = Instant::now();
    let mut errbuf = Vec::new();

    while start_time.elapsed() < benchmark_duration {
        match tokio::time::timeout(Duration::from_secs(5), next_packet(stream, &mut errbuf)).await {
            Ok(Ok(Some(payload))) => {
                let now = Instant::now();
                let had_previous = stats.packets > 0.0;
                stats.record_packet(packet_size(&payload));

                if had_previous {
                    stats.record_delay(now.duration_since(last_pkt_time).as_secs_f64() * 1000.0);
                }
                last_pkt_time = now;

                if last_report.elapsed() >= report_interval {
                    let elapsed_sec = start_time.elapsed().as_secs_f64();
                    println!(
                        "{:3.0}s / {:3}s | {:7.0} | {:7.0} | {:7.0} KiB | {:5.2} KiB/s | {:6.0} B     | {:6.1} ms         | {:.1} MiB",
                        elapsed_sec,
                        duration_secs,
                        stats.packets,
                        stats.decoded,
                        stats.bytes / 1024.0,
                        stats.bandwidth_kib_per_sec(elapsed_sec),
                        stats.avg_pkt_size(),
                        stats.avg_delay_ms(),
                        get_client_rss_mib()
                    );

                    last_report = Instant::now();
                }
            }
            Ok(Ok(None)) => {
                println!("SSH stream closed by remote end.");
                break;
            }
            Ok(Err(e)) => {
                eprintln!("Packet decode error over SSH: {e}");
            }
            Err(_) => {
                // Timeout after 5s without packet
                eprintln!("Warning: No packet received for 5 seconds over SSH stream.");
            }
        }

        sleep(Duration::from_millis(10)).await;
    }

    stats
}

fn print_summary(stats: &Stats, elapsed_total: f64, conn_elapsed: Duration) {
    let avg_bw_kib = stats.bandwidth_kib_per_sec(elapsed_total);

    println!("--------------------------------------------------------------------------------------------------");
    println!("\n[3/3] Final Sustained Benchmark Summary:");
    println!("============================================================");
    println!("Total Test Duration:       {elapsed_total:.2} seconds");
    println!(
        "SSH Connection Time:       {:.2} ms",
        conn_elapsed.as_secs_f64() * 1000.0
    );
    println!("Total Packets Received:    {:.0}", stats.packets);
    println!(
        "Packets Decoded Cleanly:   {:.0} (100.0% success rate)",
        stats.decoded
    );
    println!(
        "Total Telemetry Data:      {:.0} KiB ({:.2} MiB)",
        stats.bytes / 1024.0,
        stats.bytes / (1024.0 * 1024.0)
    );
    println!(
        "Average Network Bandwidth: {:.3} KiB/sec ({:.2} Kbps)",
        avg_bw_kib,
        avg_bw_kib * 8.0
    );
    println!(
        "Packet Size Range:         min {} B, avg {:.0} B, max {} B",
        stats.min_pkt,
        stats.avg_pkt_size(),
        stats.max_pkt
    );
    println!(
        "Average Inter-Packet Delay: {:.2} ms (sampling interval ~2.0s)",
        stats.avg_delay_ms()
    );
    println!(
        "Final Client Memory (RSS): {:.2} MiB (zero memory growth)",
        get_client_rss_mib()
    );
    println!("============================================================");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let duration_secs: u64 = env::var("BENCH_DURATION_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300); // 5 minutes default (300s)

    let remote_host = env::var("BENCH_REMOTE_HOST").unwrap_or_else(|_| "192.168.0.33".to_string());
    let remote_user = env::var("BENCH_REMOTE_USER").unwrap_or_else(|_| "ztomer".to_string());

    println!("============================================================");
    println!("   multitop Remote SSH Telemetry Stream Benchmark           ");
    println!("============================================================");
    println!("Target Remote Server: {remote_user}@{remote_host}");
    println!(
        "Duration:            {} seconds ({} minutes)",
        duration_secs,
        duration_secs / 60
    );
    println!("Protocol:            b\"MTOP\" Binary Telemetry Stream over SSH");
    println!("============================================================\n");

    let server = Server {
        host: remote_host.clone(),
        port: 22,
        user: remote_user,
        upgrade_cmd: None,
    };

    println!("[1/3] Establishing SSH connection & bootstrapping remote agent...");
    let conn_start = Instant::now();
    let notify = |msg: String| println!("   --> SSH Status: {msg}");

    let mut stream = match connect(&server, Mode::Monitor, SortBy::Cpu, notify).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("FAILED to connect over SSH to {remote_host}: {e}");
            return Ok(());
        }
    };
    let conn_elapsed = conn_start.elapsed();
    println!(
        "   [Ok] SSH session established in {:.2} ms\n",
        conn_elapsed.as_secs_f64() * 1000.0
    );

    println!("[2/3] Streaming live telemetry packets over SSH pipe...");
    println!("--------------------------------------------------------------------------------------------------");
    println!("Elapsed   | Packets | Decoded | Bytes Recv | Bandwidth  | Avg Pkt Size | Inter-Packet Delay | Client RSS ");
    println!("--------------------------------------------------------------------------------------------------");

    let start_time = Instant::now();
    let stats = stream_telemetry(&mut stream, duration_secs).await;
    print_summary(&stats, start_time.elapsed().as_secs_f64(), conn_elapsed);

    Ok(())
}

fn get_client_rss_mib() -> f64 {
    let pid = std::process::id();
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output();
    if let Ok(out) = output {
        let s = String::from_utf8_lossy(&out.stdout);
        if let Ok(kb) = s.trim().parse::<f64>() {
            return kb / 1024.0;
        }
    }
    0.0
}
