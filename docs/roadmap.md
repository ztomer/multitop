# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history (`git log --oneline`) and in the test suite. When an item here is
finished, delete it rather than ticking it off.

## Future backlog (not now)

| Item | Why deferred |
|------|--------------|
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
