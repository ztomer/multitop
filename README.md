# multitop — multi-server TUI dashboard

SSH into multiple servers and watch a compact real-time system monitor
for each one, side by side in a single terminal. Written in Rust
([ratatui](https://ratatui.rs) + tokio).

```text
─────────── ｈｏｓｔｎａｍｅ　（１０．０．０．３３） ───────────
 CPU  0:[####....] 45%  1:[##......] 22%  2:[#.......] 11%  3:[###.....] 33%
 MEM [####################....................]  62%  6.2GiB/10.0GiB
 DSK [#######.................................]  18%  167GiB/931GiB
 NET ↑ 1.2M  ↓ 3.4M
 ──────────────────────────────────────────────────────────────────────────
     PID  NAME            CPU      MEM │     PID  NAME            CPU      MEM
     123  firefox        15.2  210.4MiB │     456  chrome         12.1  180.2MiB
     789  python3         8.0   45.1MiB │     101  sshd            0.1    3.2MiB
```

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

# Or monitor remote servers via config:
mkdir -p ~/.config/multitop
cp config.example.toml ~/.config/multitop/config.toml
# edit it with your server list

# Build locally and run:
./build.sh
./multitop

# Include your local machine alongside remote servers:
./multitop --local
```

## How it works

`multitop` uploads a small static agent binary (~550 KiB) to each server on
first connect, caching it at `~/.cache/multitop/agent-<hash>`. Every later
start is a single SSH round trip that execs the cached copy — nothing is
installed on the server, and there is no runtime dependency on it beyond a
POSIX shell.

The agent samples `/proc` directly and streams pre-rendered frames back over
the SSH connection. Connections are multiplexed (`ControlMaster`), so the
Docker and upgrade views reuse the session the monitor already opened.

When monitoring locally (`--local` or `--local-only`), `multitop` executes the local agent directly without SSH overhead or network connections.

## What you see

- **Hostname/IP** — cyan header center-aligned dynamically on window resize
- **CPU** — per-core bars when the panel is wide enough, otherwise one
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
| `--local` | Include local machine (`localhost`) in the server list |
| `--local-only` | Monitor local machine only as a standalone top replacement (no SSH or config needed) |
| `-h`, `--help` | Print help information |
| `-V`, `--version` | Print version information |

## Keys

| Key | Action |
|-----|--------|
| **ESC** / **q** | Quit (terminates every SSH session) |
| **d** | Toggle the Docker view on all panels |
| **s** | Back to live stats |
| **u** | Run each server's configured `upgrade_cmd` |

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

The test suite covers panel rendering across multiple terminal dimensions, docker table parsing/formatting, `/proc` parsing, and TUI app state transitions across dedicated test files in `tests/`.

## Design notes

- **Center-aligned headers.** Server headers are dynamically centered within horizontal rule borders on window resize.
- **Adaptive layout & grid scaling.** Automatically switches between 1-column and 2-column panel grids based on terminal size and server count.
- **Per-core CPU bars.** The renderer shows individual core utilisation when the panel is wide enough, falling back to a single aggregate bar on narrow terminals.
- **Instantaneous process CPU.** CPU percentage is differenced from `/proc/<pid>/stat` between samples, so a daemon that is busy *right now* reads correctly.
- **Standalone local monitoring.** Use `--local-only` as a fast local top replacement without requiring SSH or config files.
- **Docker stats read the daemon socket.** The agent takes two one-shot samples 250 ms apart in parallel across containers.
- **Upgrade output stays on screen** until you press **s**, and streams live rather than appearing only when the command exits.

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
│       └── tests/app_test.rs
```
