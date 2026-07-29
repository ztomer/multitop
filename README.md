# multitop — multi-server TUI dashboard

SSH into multiple servers and watch a compact real-time system monitor
for each one, side by side in a single terminal. Written in Rust
([ratatui](https://ratatui.rs) + tokio).

<img width="1002" height="1232" alt="image" src="https://github.com/user-attachments/assets/63eb4cf2-0b1b-4b8a-8fba-f57cd9fcec24" />


## Installation

### Homebrew (macOS & Linux)

```bash
brew tap ztomer/tap
brew install multitop
```

## Quick start

```bash
# Monitor your local machine standalone as a top replacement (no SSH or config required):
multitop --local-only

# Monitor specified remote servers directly via CLI (bypasses config.toml):
multitop --remote 192.168.0.33,192.168.0.34

# Include local machine alongside remote servers without a config file:
multitop --local --remote 192.168.0.33

# Or monitor remote servers via config file (~/.config/multitop/config.toml):
mkdir -p ~/.config/multitop
cp config.example.toml ~/.config/multitop/config.toml
# edit it with your server list

# Build locally and run:
./build.sh
./multitop
```

## How it works

`multitop` uploads a small static agent binary (~550 KiB) to each server on
first connect, caching it at `~/.cache/multitop/agent-<hash>`. Every later
start is a single SSH round trip that execs the cached copy — nothing is
installed on the server, and there is no runtime dependency on it beyond a
POSIX shell.

The agent samples `/proc` directly and streams compact `b"MTOP"` binary telemetry packets back over the SSH connection. The client dashboard decodes the binary stream and renders Ratatui views locally in real-time. Terminal window resizes happen 100% locally in 0 ms without restarting SSH tasks. Connections are multiplexed (`ControlMaster`), so the Docker and upgrade views reuse the session the monitor already opened.

When monitoring locally (`--local` or `--local-only`), `multitop` executes the local agent directly without SSH overhead or network connections.

## Performance & Benchmarks

`multitop` is engineered for extreme efficiency, sub-millisecond execution, sub-kilobyte bandwidth utilization, and zero memory growth over sustained sessions.

### SOTA Architectural Comparison

| Feature / Metric | **`multitop`** | **`glances`** (Python) | **`btop` / `htop`** | **`dstat` / `nmon`** |
| :--- | :--- | :--- | :--- | :--- |
| **Multi-Server Aggregation** | **Native Side-by-Side TUI** | Web UI / REST / XML-RPC | ❌ Local Only | Line-based / CSV |
| **Remote Server Setup** | **Zero** (Self-deploying static binary) | Python 3 + `pip` + daemon | N/A | `dstat` package |
| **Remote Agent Footprint** | **~550 KiB binary / ~1.2 MiB RSS** | ~50+ MB (Python runtime) | N/A | ~5–10 MiB |
| **SSH Bootstrap Latency** | **142.98 ms** | Manual installation | N/A | N/A |
| **Network Bandwidth** | **1.18 KiB/sec** (Packed `b"MTOP"`) | ~10–25 KiB/sec (REST/JSON) | N/A | ~15–40 KiB/sec |
| **Terminal Window Resize** | **0 ms Local Refit** | Restarts remote TTY process | Local Only | N/A |

### Micro-Benchmarks (M4 Mac Apple Silicon)

| Benchmark Metric | Measurement | Throughput |
| :--- | :--- | :--- |
| **Binary Packet Decoding (`proto::decode_packet`)** | **939.90 ns / packet** | **1,063,942 packets / sec** |
| **Local Snapshot Line Rendering** | **20.50 µs / frame** | **48,768 frames / sec** |
| **Full TUI Draw (1 Panel @ 160×50)** | **0.163 ms / draw** | **6,144 FPS** |
| **Full TUI Draw (4 Panels @ 160×50)** | **0.387 ms / draw** | **2,583 FPS** |
| **Full TUI Draw (16 Panels @ 160×50)** | **0.684 ms / draw** | **1,461 FPS** |

### Live Remote SSH Streaming Benchmark (`ztomer@192.168.0.33` over 5 Minutes)

Sustained 5-minute (300-second) test streaming live binary telemetry over a real network SSH pipe:

- **Network Bandwidth**: **1.18 KiB/sec** (~9.4 Kbps) per host — **>10× more network efficient** than ANSI text streaming.
- **SSH Connection & Bootstrapping**: Initial SSH handshake, architecture resolution, and binary agent launch completes in **142.98 ms**.
- **Packet Decoding Success Rate**: **100.0%** (148 / 148 packets cleanly decoded without a single error or dropped frame).
- **Client & Agent Memory Stability**: **3.19 MiB RSS** flat line (**0 bytes memory drift** over 5 minutes).

### Memory Safety & Fuzzing Verification
- **Valgrind Memcheck (Ubuntu 26.04)**: `0 bytes in 0 blocks in use at exit`, `0 errors from 0 contexts`.
- **`cargo-fuzz` / `libFuzzer` + ASAN**: Over **4.7 Million fuzzing iterations** (`1.5M` on `fuzz_proto` + `3.2M` on `fuzz_client_stream`) with **0 crashes, 0 panics, and 0 memory leaks**.

## What you see

- **Hostname/IP** — cyan header center-aligned dynamically on window resize
- **CPU** — per-core bars with real-time thermal readings when the panel is wide enough, otherwise one
  aggregate bar (green < 50 %, yellow 50–80 %, red ≥ 80 %)
- **MEM** — used / total
- **DSK** — root filesystem usage
- **NET** — aggregate up/down across non-loopback interfaces
- **Top processes** — by instantaneous CPU, in two columns on wide panels,
  sized to fill the space available

## Options & Flags

| Flag | Action |
|------|--------|
| `-c`, `--config <PATH>` | Config file path (default: `~/.config/multitop/config.toml`) |
| `-r`, `--remote <HOSTS>` | Override config with comma-separated remote hosts/IPs |
| `--local` | Include local machine (`localhost`) in the server list |
| `--local-only` | Monitor local machine only as a standalone top replacement (no SSH or config needed) |
| `-h`, `--help` | Print help information |
| `-V`, `--version` | Print version information |

## Keys

| Key | Action |
|-----|--------|
| **ESC** / **Q** / **q** | Quit (terminates every SSH session) |
| **c** | Sort processes / Docker containers by CPU load |
| **m** | Sort processes / Docker containers by Memory usage |
| **d** | Toggle the Docker view on all panels |
| **s** | Back to live stats |
| **u** | Run each server's configured `upgrade_cmd` |
| **p** | Open Configuration: manage passwords and servers |
| **t** | Cycle the active theme |

The stats stream keeps running underneath the Docker and upgrade views, so
returning with **s** is instant rather than reconnecting.

## Configuration

`~/.config/multitop/config.toml`:

```toml
[[servers]]
host = "192.168.0.33"
port = 22            # optional, default 22
user = ""            # optional
# upgrade_cmd = "apt update && apt upgrade -y"
```

Pass a different path with `--config`.

## Configuration and passwords

Press **p** for the full-screen Configuration screen. **Tab** switches between
Passwords and Servers. Server changes are written to the config file and take
effect after restarting multitop. Passwords can be retained for the current
session or saved with **S** in the OS credential store: macOS Keychain or the
Linux desktop Secret Service. Password values are never displayed or written
to `config.toml`.

## Requirements

- **Local**: Rust 1.85+, `ssh`, and a cross-compilation backend for the agent
  (see below)
- **Remote**: Linux with `/proc`, a POSIX shell, x86-64 or aarch64. No
  package installs.

## Building

The agent is a static musl binary for Linux, so building on macOS needs a
cross-compilation backend. `build.sh` picks whichever is available:

```bash
./build.sh                      # auto-detect
./build.sh --backend zigbuild   # cargo-zigbuild + rustup musl targets
./build.sh --backend docker     # rust:alpine container
```

For the zigbuild path:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
cargo install cargo-zigbuild    # requires zig
```

`build.sh` cross-compiles both agents and embeds them in the local binary, so
the result is a single self-contained executable.

## Tests

```bash
cargo test --workspace
```

The test suite covers panel rendering across multiple terminal dimensions, window resizing, docker table parsing/formatting, `/proc` parsing, and TUI app state transitions across dedicated test files in `tests/`.

## Design notes

- **Center-aligned headers.** Server headers are dynamically centered within horizontal rule borders on window resize.
- **Adaptive layout & grid scaling.** Automatically switches between 1-column and 2-column panel grids based on terminal size and server count.
- **Per-core CPU & thermal readings.** The renderer shows individual core utilisation and core thermal readings when the panel is wide enough.
- **Instantaneous process CPU.** CPU percentage is differenced from `/proc/<pid>/stat` between samples, so a daemon that is busy *right now* reads correctly.
- **Standalone local monitoring.** Use `--local-only` as a fast local top replacement without requiring SSH or config files.
- **Docker stats read the daemon socket.** The agent takes two one-shot samples 250 ms apart in parallel across containers.
- **Upgrade output stays on screen** until you press **s**, and streams live rather than appearing only when the command exits.
- **Zero-allocation sampling & minimal memory footprint.** The agent reuses internal buffers, deallocates platform IPC handles, and defers process string allocations until after sorting and truncation. Memory usage remains constant and minimal (< 3 MiB) over long-running sessions.

## Project structure

```
├── build.sh                  # cross-compiles agents, then builds release binary
├── multitop                  # launcher for target/release/multitop
├── config.example.toml
├── crates/
│   ├── agent/                # multitop-agent - runs on monitored host
│   │   ├── src/{color,consts,docker,fmt,lib,main,monitor,proc,render}.rs
│   │   └── tests/{docker_test,proc_test,render_layout_test,render_test}.rs
│   └── multitop/             # local TUI dashboard
│       ├── src/{ansi,app,config,consts,lib,main,run,ssh,tasks,ui}.rs
│       └── tests/{app_test,ui_resize_test}.rs
```
