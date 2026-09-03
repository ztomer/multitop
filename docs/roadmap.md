# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history (`git log --oneline`) and in the test suite. When an item here is
finished, delete it rather than ticking it off.

## Phase 1 — Quiet polish (1–2 days, no agent fields)

Small, no-risk wins that remove papercuts before adding power.

| Item | What | Why now | Touches |
|------|------|---------|---------|
| **Help overlay `?`** | `Keys` table from `README.md:82` over dim `Rams` overlay, `Esc` closes | `t` is undiscoverable; `?` is the one key every TUI owns | `crates/multitop/src/ui/draw.rs`, `tui/lib.sh` `NO_COLOR` |
| **Layout memory** | `state.toml` stores `selected_panel`, `filter_query`, `sort`, `view per host` | Restarts where you left, not at `beelink` | `crates/multitop/src/state.rs`, `crates/multitop/src/run/event_loop.rs:140` |
| **Focus `Enter`/`z`** | Zoom one panel to `120x50` `STREAM_COLS/LINES` via `agent_dims` from `regions` | Same path that fixed filtered grid | `crates/multitop/src/run/dims.rs`, `crates/multitop/src/app/views.rs` |
| **Command palette `:`** | `> Filter beelink` `> Upgrade storage` `> Add server` `Trie` of `KeyCode` | Replaces 6 scattered `tui` hints `check_key_hints.py` flags | `crates/multitop/src/run/handle_key.rs:357` |
| **Filter save + regex** | `Ctrl-S` saves `1..3`, `ip:`, `cpu>50`, `image:nginx` via `Panel::matches_filter` | `/` already searches `host/user` + view text | `crates/multitop/src/filter.rs`, `crates/multitop/src/panel.rs` |
| **Yank `y`** | `g` graphs `y` yanks `host:port` to clipboard via `arboard` | Quiet success, no new agent | `crates/multitop/src/ui/draw.rs` |
| **Vault onboarding inline** | `e` → `A` Add → password field offers `Create vault` inline, never leaves `Settings` | Current `seed_vault_from_panels` drops you out | `crates/multitop/src/passwords`, `crates/multitop/src/app/vault.rs:219` |

Pass: `tmux_harness.py` `wait_for_screen("?")` + `80x24`/`140x40` light/dark.

## Phase 2 — Observe (1 week, reuse `History` ring)

Turn the `History` ring already on disk in RAM into signal.

| Item | What | Why | Touches |
|------|------|-----|---------|
| **Alerts as panels** | One threshold per metric `alert_cpu=80` in `config.toml`, header tints `amber→red`, `H` shows `last 30m` sparkline of breach | Reuse `History` + `METER_HIGH_PCT` constants, no new agent fields | `crates/agent/src/consts.rs:37`, `crates/multitop/src/history.rs` |
| **Per-host health `0-100`** | `CPU/MEM/DSK/NET + failed systemd units + pending upgrades + vault locked` → `/ unhealthy` filter | Lets `/` become `/ unhealthy` | `crates/multitop/src/filter.rs`, `crates/agent/src/proc.rs` |
| **History to disk** | Spill `~/.cache/multitop/history/<host>.zst` `1h/1d` via `zstd` already in `Cargo.toml`, `G` draws `1d` not `200` ticks | `History` currently RAM-only | `crates/multitop/src/history.rs`, `crates/multitop/src/graphs.rs` |
| **Graphs zoom `+/-`** | Pinch/keys zoom `History` ring, `y` yank point | Same ring, no new wire | `crates/multitop/src/graphs.rs` |

Pass: `cargo bench --bench client_bench` under `MAX_RENDER_NS`, `History` `zst` round-trip.

## Phase 3 — Act (1–2 weeks, reuse `Exec` pty)

Do without leaving the grid. All three reuse `crates/agent/src/exec` pty + `Painter` + `MAX_EXEC_CHUNK`.

| Item | What | Guard |
|------|------|-------|
| **Top process actions `x/o/r`** | `x` kill `kill -9 <pid>`, `o` `journalctl -f -u <unit>` or `tail -f /proc/<pid>/fd`, `r` renice — confirm modal names `host:pid:name` | Same `Confirm::Upgrade` pattern `crates/multitop/src/run/handle_key.rs:79` |
| **File tail `l`** | `tail -n 200 -F /var/log/syslog` as framed `Exec` | `Painter` reuse, `RingLines` |
| **Upgrade dry-run + diff** | `u` first shows `apt list --upgradable` parsed via agent `apt` shim as second column `12 pkgs (kernel 6.8→6.9 held)` | Agent `apt` shim, no sudo until confirm |
| **Custom `exec` panels** | `[[panels]] command="nvidia-smi …"` or `pg_stat_activity` — agent runs on `250ms`, client treats as `Fetch` card | `crates/multitop/src/render_payload.rs:20` dispatch |

Pass: `tests/test_exec_live.py` oracle `ls -l ; ls -l` still `cold==warm==unmuxed`, `test_tmux_e2e.py` `x` shows `Confirm` not `STARTED`.

## Phase 4 — Notify (future, after Phase 2 `HostUpdate` on disk)

`HostUpdate` `finished_at`/`success` already in `state.toml` `crates/multitop/src/app/upgrade.rs:220`. Add `[[alerts]] webhook="https://ntfy.sh/…"` or `desktop` via `notify-rust`, `apprise` style. No new agent.

Pass: `cargo test` `upgrade_view` `HostUpdate` round-trip, manual `ntfy` curl.

## Future backlog (not now)

| Item | Why deferred |
|------|--------------|
| **Web companion `multitop --serve :8080`** | Reuses `b"MTOP"` decoder as `xterm.js`/JSON, but is a new binary, new auth surface (`Hello` + `Token`), new deploy. Put behind `Phase 4` + `cargo bench` headroom. |
| **Mobile companion** | Same `MTOP` over `WebSocket`, needs `Hello` + `Token` hardening + APNs. |
| **Post-quantum KEM** | Not warranted for device-local file threat model. |
| **Plugin SDK (WASM)** | Custom `exec` panels cover 80% without sandbox. |

## Deferred (pre-existing)

| Item | Why |
|------|-----|
| Post-quantum KEM | Not warranted for a device-local file threat model. |

## Recent learnings (pruned 2026-09-03)

History that was here (Layers 1-3, 40+ defect rows) pruned to `git log` and `docs/detection-record.md` per house rule. What was learned is captured as skills:

- **Hello must be first, valid, and single** — `Proto::Hello` `agent_version` + `proto/min` `is_valid`/`is_compatible`/`needs_replacement` (never downgrade). Duplicate is violation. `crates/agent/src/proto/mod.rs:31`.
- **Embedded agent must match workspace version** — `build.rs` `panic!` for `release` when `CARGO_PKG_VERSION` not in `agent` bytes, `tools/check_agent_version.py` gates hook/CI/local-ci.
- **Stable ad-hoc signature** — `codesign --identifier com.ztomer.multitop` in `build.sh` + `tools/check_codesign.py` (auto-fixes) so `Always Allow` persists.
- **Vault is source of truth, but fallback on miss** — `try_load_vault_password` `crates/multitop/src/vault.rs:71` falls back `password_store::load`; `seed_vault` imports from keychain; `VaultUnlocked` `crates/multitop/src/app/apply.rs:344` reloads `vault.hosts()` into `panel.sudo_password`.
- **Build is `./build.sh` only** — `cargo build -p multitop` alone embeds `missing` or stale.
