# Performance & Benchmarks

`multitop` is engineered for extreme efficiency, sub-millisecond execution,
sub-kilobyte bandwidth utilization, and zero memory growth over sustained
sessions.

## SOTA Architectural Comparison

| Feature / Metric | **`multitop`** | **`glances`** (Python) | **`btop` / `htop`** | **`dstat` / `nmon`** |
| :--- | :--- | :--- | :--- | :--- |
| **Multi-Server Aggregation** | **Native Side-by-Side TUI** | Web UI / REST / XML-RPC | Local Only | Line-based / CSV |
| **Remote Server Setup** | **Zero** (Self-deploying static binary) | Python 3 + `pip` + daemon | N/A | `dstat` package |
| **Remote Agent Footprint** | **~650 KiB binary / ~2.7 MiB RSS (~316 KiB private)** | ~50+ MB (Python runtime) | N/A | ~5–10 MiB |
| **SSH Bootstrap Latency** | **142.98 ms** | Manual installation | N/A | N/A |
| **Network Bandwidth** | **1.18 KiB/sec** (Packed `b"MTOP"`) | ~10–25 KiB/sec (REST/JSON) | N/A | ~15–40 KiB/sec |
| **Terminal Window Resize** | **0 ms Local Refit** | Restarts remote TTY process | Local Only | N/A |

## Micro-Benchmarks (M4 Mac Apple Silicon)

| Benchmark Metric | Measurement | Throughput |
| :--- | :--- | :--- |
| **Binary Packet Decoding (`proto::decode_packet`)** | **1.11 µs / packet** | **898,303 packets / sec** |
| **Local Snapshot Line Rendering** | **29.34 µs / frame** | **34,078 frames / sec** |
| **Full TUI Draw (1 Panel @ 160×50)** | **0.17 ms / draw** | **5,986 FPS** |
| **Full TUI Draw (4 Panels @ 160×50)** | **0.42 ms / draw** | **2,381 FPS** |
| **Full TUI Draw (16 Panels @ 160×50)** | **0.72 ms / draw** | **1,394 FPS** |

## Live Remote SSH Streaming Benchmark (`ztomer@192.168.0.33` over 5 Minutes)

Sustained 5-minute (300-second) test streaming live binary telemetry over a real network SSH pipe:

- **Network Bandwidth**: **1.18 KiB/sec** (~9.4 Kbps) per host — **>10× more network efficient** than ANSI text streaming.
- **SSH Connection & Bootstrapping**: Initial SSH handshake, architecture resolution, and binary agent launch completes in **142.98 ms**.
- **Packet Decoding Success Rate**: **100.0%** (148 / 148 packets cleanly decoded without a single error or dropped frame).
- **Client & Agent Memory Stability**: **2.69 MiB RSS** flat line (**316 KiB private**, **0 bytes memory drift** over 5 minutes, verified by valgrind memcheck across multiple hosts).

## Memory Safety & Fuzzing Verification

- **Valgrind Memcheck (Ubuntu 26.04, release build)**: `0 bytes definitely lost`, `0 bytes indirectly lost` — clean across both monitored hosts. The ~154 KB of `still reachable` + `possibly lost` at exit is internal glibc/Rust allocator metadata, not user code leaks.
- **SSH Disconnect Safety (v0.20.7)**: The agent's stdin watchdog detects EOF and self-terminates within ≤2 s when the SSH pipe breaks, preventing stray agents even if the local process crashes.
- **Cross-Platform Process Scanning (v0.20.7)**: macOS process enumeration uses `proc_pidinfo` when `/proc` is unavailable.
- **Upgrade State Machine (v0.20.8)**: Per-server `UpgradeState` enum (NIL/STARTED/DONE) replaces the global bool+counter. View switches during an upgrade no longer orphan the state — `upgrade_gen` tracks the upgrade task independently of `panel.gen`.
- **Concurrent Upgrade Lock (v0.20.8)**: Atomic `mkdir`-based lock prevents concurrent upgrades across clients/sessions on the same server (stale locks >6 h auto-broken). Local PID-based lock prevents two multitop processes from upgrading the same machine simultaneously.
- **Power-Loss Detection (v0.20.8)**: `upgrade_started_at` marker in `state.toml` detects client-side power loss. On next launch, the modal shows `⚠ Previous upgrade was interrupted! Check server state.` if the last upgrade didn't complete.
- **Exit-Code-Aware Completion (v0.20.8)**: `AuxDone` carries a `success` flag. Only clean exits (exit code 0) persist `last_update`. Server power loss produces `⚠ disconnected (upgrade may be incomplete)` instead of a false `─ done`.
- **Upgrade Hardening (v0.20.9)**: `upgrade_started_at` is only set when at least one panel has `upgrade_cmd` (prevents false power-loss warnings). Password-resume path now sets the timestamp. Local lock has a timestamp-based 6-hour staleness fallback when the PID file is missing (e.g. disk full during `echo $$`).
- **`cargo-fuzz` / `libFuzzer` + ASAN**: Over **114 Million fuzzing iterations** across 6 targets (`fuzz_proto`, `fuzz_client_stream`, `fuzz_proc_stat`, `fuzz_meminfo`, `fuzz_net_dev`, `fuzz_fetch`) with **0 crashes, 0 panics, and 0 memory leaks**.
- **Callgrind CPU Profile (.33, debug build)**: 227M instructions over 10 s; top self-time in `parse_pid_stat` (1.00%) and stdlib I/O routines — agent hot path is already allocation-free.
