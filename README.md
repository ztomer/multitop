# multitop — multi-server TUI dashboard

SSH into multiple servers and watch a compact real-time system monitor
for each one, side by side in a single terminal. Written in Rust
([ratatui](https://ratatui.rs) + tokio).

Self-deploys a tiny static agent binary to each host on first connect —
zero setup on the remote side. Also monitors the local machine, runs
upgrade commands across servers with power-loss detection, and includes
a Docker container view. Works on macOS (Apple Silicon) and Linux.

<img width="1902" height="1232" alt="image" src="https://github.com/user-attachments/assets/d145f190-03b3-49e3-8fa7-1501e1aa73a7" />


## Installation

### Homebrew (macOS Apple Silicon & Linux)

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
- **Update view** (`u`) — per-server update status; press `u` again to run
- **Settings screen** (`e`) — servers, their passwords, and the vault
- **Filter** (`/`) — narrow the grid to hosts matching what you type

The update view is deliberately two presses. The first switches to it and
starts nothing, showing for each server what command would run, when it last
ran and how that went, whether a sudo password is ready, and — for a server
with no `upgrade_cmd` — what to add to the config. Only the second press asks
for confirmation and runs, so you always see what you are about to do first.

The stats stream keeps running underneath the Docker and upgrade views, so
returning with **s** is instant rather than reconnecting.

## Keys

| Key | Action |
|-----|--------|
| **ESC** / **Q** / **q** | Quit (terminates every SSH session). With a filter applied, the first **ESC** clears it |
| **c** | Sort processes / Docker containers by CPU load |
| **m** | Sort processes / Docker containers by Memory usage |
| **d** | Toggle the Docker view on all panels |
| **s** | Back to live stats |
| **u** | Show the update status view; press again to run the updates |
| **f** | Toggle the Fetch view |
| **e** | Open Settings: servers, passwords, vault |
| **/** | Filter the grid by host or user; **Enter** keeps it, **ESC** clears it |
| **1**–**9** | Select a panel |
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

## Encrypted Sudo Password Vault

Multitop includes an encrypted vault for sudo passwords with biometric unlock:

- **macOS**: Touch ID / Face ID via Secure Enclave (ECIES P-256)
- **Linux**: Fingerprint via fprintd (D-Bus)
- **Fallback**: the vault master password (Argon2id, auto-tuned to available RAM)

The master password unlocks the vault. It is not a sudo password: each server
has its own, kept inside.

### Features

- **AES-256-GCM** encryption with **Ed25519** signature verification
- **HKDF key separation** — distinct encryption and signing sub-keys
- **Argon2id** key derivation (auto-tuned: RAM/4, clamped 64–1024 MiB)
- **Rate limiting** — exponential backoff (1s, 2s, 4s… max 60s), hard lockout after 10 failures (5 min)
- **Rollback protection** — monotonic counter stored in OS keychain; rejects replaced/old vault files
- **mlock** — vault key locked in RAM (best-effort on macOS/Linux)
- **Keychain storage** — rollback counter stored in OS keychain (`multitop-vault-rollback`)

### Vault Integration

- **Upgrade flow**: Press `u` to open the update view → press `u` again → if the vault is locked, biometric prompt → fallback to master password → confirm modal → passwords auto-loaded into panels
- **Created on demand**: the first time you save a sudo password, multitop offers to create the vault and asks for a master password. There is nothing to set up in advance.
- **Priority**: Vault passwords take precedence over OS keychain entries

## Configuration and passwords

Press **e** for the full-screen Settings screen: one list of servers with
the password status of each.

| Key | Action |
|-----|--------|
| **Enter** / **E** | Edit this server — host, user, port, upgrade command, password |
| **A** | Add a server |
| **D** | Delete a server (asks first) |
| **I** | Add hosts from `~/.ssh/config` that are not configured yet |
| **R** | Change the vault master password |
| **S** | Toggle sparklines (experimental) |
| **ESC** / **Q** | Return |

**Every server has its own sudo password**, typed in that server's row. There is
no shared one. A password set for one host is set for that host and no other;
leaving the field empty removes the one already stored. Passwords go to the OS
credential store — macOS Keychain, or the Linux desktop Secret Service — and to
the vault when it is unlocked. Saving the first password offers to create the
vault, over this screen: answering the offer leaves you where you were.

Once a vault exists it is the source of truth, and the credential store is not
consulted to *report* on credentials — only to fall back on if the vault turns
out not to hold a host's password when a run needs it. That keeps a single
upgrade to a single unlock instead of an OS credential dialog followed by the
vault prompt.

Server changes are written to the config file and take effect immediately;
panels are rebuilt without a restart.

Password values are never displayed or written to `config.toml`. A
`sudo_password` key there is not supported — it is plaintext on disk and was
never read. If one is found at startup it is moved into the credential store
and deleted from the file.

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

See `docs/performance.md` for full benchmark details, SOTA comparison tables,
and fuzzing verification results.

## Requirements

- **Local**: macOS on Apple Silicon, or Linux. Rust 1.85+, `ssh`, and a
  cross-compilation backend for the agent (see below).
- **Remote**: Linux with `/proc`, a POSIX shell, x86-64 or aarch64. No
  package installs. This is the agent's architecture, not the machine you run
  multitop on.

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

The test suite covers panel rendering across multiple terminal dimensions, window resizing, docker table parsing/formatting, `/proc` parsing, vault operations (init, unlock, rate limiting, rollback, biometric), and TUI app state transitions across dedicated test files in `tests/`.

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
- **Agent cache cleanup.** After each agent upload, stale `agent-*` binaries are removed from `~/.cache/multitop/`, keeping only the current x86_64 and aarch64 builds.
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
│   ├── multitop/             # local TUI dashboard
│   │   ├── src/{ansi,app,config,consts,lib,main,run,ssh,tasks,ui,vault,mlock,lockout,rollback,fprintd,secure_enclave}.rs
│   │   └── tests/{app_test,ui_resize_test,vault_upgrade_e2e}.rs
│   └── vault/                # encrypted password vault (separate crate)
│       ├── src/{api,crypto,format,lockout,mlock,rollback,fprintd,secure_enclave}.rs
│       └── tests/*.rs
```

## Security

The vault implementation has undergone three rounds of security review. Key properties:

- **No plaintext secrets** in memory after lock — `zeroize` on drop
- **Signature verification before decrypt** — prevents ciphertext malleability
- **Canary in plaintext** — detects wrong password vs corruption
- **Rate limiting** — persists across restarts via companion lockout file
- **Rollback detection** — counter stored in system keychain, survives vault file replacement
- **mlock best-effort** — prevents swapping vault key to disk (logs warning on EPERM/ENOMEM)
- **Secure overwrite** — 3-pass on key rotation (random/zero/random); full-disk encryption recommended for true protection

## License

MIT