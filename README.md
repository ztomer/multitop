# monitor — multi-server dashboard

SSH into multiple servers and run the compact monitor in a stacked tmux layout.
One pane per server, stacked vertically. ESC exits cleanly; Ctrl-C kills the
monitor in the current pane, remaining panes continue.

## Quick start

```bash
./monitor
```

Reads `~/.config/monitor/config.toml`, creates a tmux session, SSHs into each
server, and runs the compact system monitor in each pane.

## Config

Toml file at `~/.config/monitor/config.toml`:

```toml
[[servers]]
host = "192.168.0.33"
port = 22
user = ""

[[servers]]
host = "192.168.0.90"
port = 22
user = ""

[[servers]]
host = "192.168.0.158"
port = 22
user = ""
```

`user` is optional — when omitted SSH uses the invoking user's OS account.

## What you see

Each pane shows the remote server's compact status panel:

```
hostname (10.0.0.33)  ──────────────────
 CPU [########........] 48%
 MEM [############....] 62%  6.4/10.3 GiB
 DSK [###...............] 18%  167/931 GiB
 NET  ↑1.2M  ↓3.4M
 ───────────────────────────────────────
   PID  NAME           CPU    MEM
   123 firefox        15%  210MiB
   456 chrome         12%  180MiB
   789 python3         8%   45MiB
   012 tmux            5%   12MiB
   345 sshd            3%    8MiB
```

- **Hostname/IP** in cyan — identifies which pane is which
- **CPU** overall usage, bar color: green < 50%, yellow 50-80%, red >= 80%
- **Top processes** sorted by CPU — fills available vertical space (5 shown here in a ~20-row terminal, adjusts on resize)
- **MEM** total/used, bar colors match CPU thresholds
- **DSK** total/used, green < 70%, yellow 70-90%, red >= 90%
- **NET** aggregate upload/download across all interfaces (2s window)
- Bar width adjusts to terminal size

The monitor is delivered as an embedded Python script in the SSH command — no
remote install needed, just requires `python3` on the server.

## How it works

1. Validates `tmux` and `ssh` are present (pre-flight check)
2. Parses `~/.config/monitor/config.toml` with Python's `tomllib`
3. Config validation — rejects non-list `servers`, missing `host`, whitespace in `user`
4. Kills any pre-existing `multi_server_monitor` tmux session (warns if clients attached)
5. Creates a new tmux session
6. Opens SSH + compact monitor in the first pane
7. Splits vertically for each additional server
8. Applies `even-vertical` layout
9. Binds ESC to kill-session (cleaned up on exit)
10. Attaches

## Requirements

- **Local**: tmux, ssh, python3 >= 3.11 (for tomllib)
- **Remote**: python3, a Linux `/proc` filesystem (the monitor reads /proc/stat,
  /proc/meminfo, /proc/net/dev, /proc/self/mountinfo)

## Exit

| Key | Action |
|-----|--------|
| **ESC** | Kills the entire tmux session immediately (binding removed on exit) |
| **Ctrl-C** | Kills the monitor in the current pane only; session ends when all panes close |

Tmux prefix shortcuts (default Ctrl-B) also work for navigating panes.

## Tests

```bash
pip install pytest pytest-cov
python3 -m pytest tests/ --cov=monitor
```

39 tests, 98% coverage.

## Project structure

```
├── monitor              # Shell wrapper → monitor.py
├── monitor.py           # Orchestrator — config, tmux session, SSH commands
├── compact_monitor.py   # Embedded system monitor (runs remotely via SSH)
├── setup.cfg            # pytest + coverage config
├── tests/
│   ├── __init__.py
│   ├── conftest.py      # Shared fixtures (tmp_toml)
│   └── test_monitor.py  # 39 tests
└── README.md
```

## Existing alternatives

| Tool | What it does | Gap |
|------|-------------|------|
| `clusterssh` (`brew install clusterssh`) | Multi-SSH in separate xterm windows | No tmux layout, no monitor integration |
| `tmuxinator` | Declarative tmux session management | No SSH automation |
| `sshpilot` | GUI SSH client at `~/.config/sshpilot/` | GUI-based, no dashboard layout |
| `btop` | Full TUI system monitor | Too tall for 1/3-screen panes |
