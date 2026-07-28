# multitop — multi-server TUI dashboard

SSH into multiple servers and watch a compact real-time system monitor
for each one, side by side in a single terminal. Built with
[Textual](https://textual.textualize.io).

```
hostname (10.0.0.33)  ─────────────────────────────────────────
 CPU [################........................] 45%
 MEM [####################....................] 62%  6.2/10.0 GiB
 DSK [#######.................................] 18%  167/931 GiB
 NET  ↑1.2M  ↓3.4M
 ──────────────────────────────────────────────────────────────
   PID  NAME           CPU      MEM
   123 firefox        15%  210MiB
   456 chrome         12%  180MiB
   789 python3         8%   45MiB
```

## Quick start

Copy the example config and edit it:

```bash
mkdir -p ~/.config/multitop
cp config.example.toml ~/.config/multitop/config.toml
# edit ~/.config/multitop/config.toml with your server list
```

Then run:

```bash
./multitop
```

## What you see

Each bordered panel shows one server's status:

- **Hostname/IP** — cyan header, identifies the server
- **CPU** — overall usage bar (green < 50%, yellow 50–80%, red ≥ 80%);
  2+ cores shows per-core bars inline
- **Top processes** — sorted by CPU, fills available vertical space
- **MEM** — total / used with bar
- **DSK** — root filesystem usage
- **NET** — aggregate upload / download across non-loopback interfaces (2 s window)

The compact monitor is delivered as an embedded Python script over SSH —
no remote install needed, just `python3` on the server.

## Keys

| Key | Action |
|-----|--------|
| **ESC** | Quit entirely (kills all SSH processes) |

## Requirements

- **Local**: python3 ≥ 3.11, ssh
- **Remote**: python3, Linux `/proc` filesystem

## Install

```bash
pip install textual
./multitop
```

## Tests

```bash
pip install pytest pytest-cov
python3 -m pytest tests/ --cov=multitop
```

134 tests, 100 % coverage.

## Project structure

```
├── multitop              # Shell wrapper → multitop.py
├── multitop.py           # Textual TUI app
├── compact_monitor.py    # Embedded system monitor (runs remotely via SSH)
├── config.example.toml   # Example config — bootstrap from this
├── setup.cfg             # pytest + coverage config
├── tests/
│   ├── __init__.py
│   ├── conftest.py       # Shared fixtures
│   ├── test_multitop.py  # TUI / config tests
│   └── test_compact_monitor.py
└── README.md
```
