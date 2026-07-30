# multitop — multi-server TUI dashboard

SSH into multiple servers and watch a compact real-time system monitor
for each one, side by side in a single terminal. Written in Rust
([ratatui](https://ratatui.rs) + tokio).

Self-deploys a tiny static agent binary to each host on first connect —
zero setup on the remote side. Also monitors the local machine, runs
upgrade commands across servers with power-loss detection, and includes
a Docker container view. Works on macOS and Linux.

<img width="1902" height="1232" alt="image" src="https://github.com/user-attachments/assets/d145f190-03b3-49e3-8fa7-1501e1aa73a7" />


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

## What you see

Each panel shows:

- **Hostname/IP** — cyan header center-aligned dynamically on window resize
- **CPU** — per-core bars with real-time thermal readings when the panel is wide enough, otherwise one
  aggregate bar (green < 50 %, yellow 50–80 %, red ≥ 80 %)
- **MEM** — used / total
- **DSK** — root filesystem usage
- **NET** — aggregate up/down across non-loopback interfaces
- **Top processes** — by instantaneous CPU, in two columns on wide panels,
  sized to fill the space available

Additional views accessible via keys:

- **Docker view** (`d`) — container list with CPU/memory usage, sorted by load
- **Upgrade view** (`u`) — live streaming output of each server's `upgrade_cmd`
- **Configuration screen** (`p`) — manage passwords and server entries

The stats stream keeps running underneath the Docker and upgrade views, so
returning with **s** is instant rather than reconnecting.

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

## How it works

`multitop` uploads a small static agent binary (~650 KiB) to each server on
first connect, caching it at `~/.cache/multitop/agent-<hash>`. Every later
start is a single SSH round trip that execs the cached copy — nothing is
installed on the server, and there is no runtime dependency on it beyond a
POSIX shell.

The agent samples `/proc` directly and streams compact binary telemetry
packets back over the SSH connection. The client decodes the stream and
renders Ratatui views locally in real-time. Terminal window resizes happen
100% locally in 0 ms without restarting SSH tasks. Connections are
multiplexed (`ControlMaster`), so the Docker and upgrade views reuse the
session the monitor already opened.

When monitoring locally (`--local` or `--local-only`), `multitop` executes
the local agent directly without SSH overhead or network connections.

## Performance

`multitop` is engineered for extreme efficiency, sub-millisecond execution,
sub-kilobyte bandwidth utilization, and zero memory growth over sustained
sessions.

| Metric | Measurement |
|--------|-------------|
| **Remote agent footprint** | ~650 KiB binary / ~2.7 MiB RSS (~316 KiB private) |
| **SSH bootstrap latency** | 142.98 ms |
| **Network bandwidth** | 1.18 KiB/sec per host |
| **Packet decoding** | 1.11 µs / packet (898K packets/sec) |
| **Full TUI draw (4 panels)** | 0.42 ms / draw (2,381 FPS) |
| **Memory drift (5 min)** | 0 bytes |

See `PERFORMANCE.md` for full benchmark details, SOTA comparison tables,
and fuzzing verification results.

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
- **Zero-allocation sampling & minimal memory footprint.** The agent reuses internal buffers (`scanned`, `active_pids`, `temp_procs`) across ticks, deallocates platform IPC handles, and defers process string allocations until after sorting and truncation. Memory usage remains constant at < 2.7 MiB RSS (< 320 KiB private) over long-running sessions.
- **Upgrade state machine.** Per-server `UpgradeState` enum (NIL/STARTED/DONE) with power-loss detection via `upgrade_started_at` marker, exit-code-aware completion, and concurrent upgrade locking.
- **Binary protocol.** The agent streams compact `b"MTOP"` packets over SSH — >10× more network efficient than ANSI text streaming.

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
