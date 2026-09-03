# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history (`git log --oneline`) and in the test suite. When an item here is
finished, delete it rather than ticking it off.

## Deferred

| Item | Why |
|------|-----|
| Post-quantum KEM | Not warranted for a device-local file threat model. |

## Recent learnings (pruned 2026-09-03)

The `docs/roadmap.md` history that was here (Layers 1-3, 40+ defect rows,
instruments, pattern) has been pruned to `git log` and to
`docs/detection-record.md` per the house rule: one forward-looking file, no
graveyard. What was learned in this session is captured as skills, not as
ever-longer markdown:

- **Hello must be first, valid, and single** — `Proto::Hello` carries
  `agent_version` + `proto_version`/`min` with `is_valid`/`is_compatible` and
  `needs_replacement` (never downgrade). Duplicate or invalid Hello is a
  protocol violation, not a second chance. See `crates/agent/src/proto/mod.rs:31`.
- **Embedded agent must match workspace version** — `build.rs` now `panic!` for
  `release` when `CARGO_PKG_VERSION` not inside `multitop-agent` bytes, and
  `tools/check_agent_version.py` gates it in hook/CI/local-ci. Stale `0.44.0`
  inside `0.44.1` looped `Hello 0.44.0 vs 0.44.1` re-uploading same bytes.
- **Stable ad-hoc signature** — `codesign --identifier com.ztomer.multitop` in
  `build.sh` + `tools/check_codesign.py` (auto-fixes) so `Always Allow` in
  `login.keychain` persists. Without it each `cargo build` got `multitop-abc123`
  and keychain `genp multitop` prompted again.
- **Vault is source of truth, but must fallback on miss** —
  `try_load_vault_password` `crates/multitop/src/vault.rs:71` now falls back
  `password_store::load` when `unlocked.get_password` is `None`; `seed_vault`
  imports from keychain; `VaultUnlocked` `crates/multitop/src/app/apply.rs:344`
  reloads `vault.hosts()` into `panel.sudo_password` so upgrade view shows
  `Stored` not `will prompt`. `vault.bin` `650B` now holds `0.33:22`/`0.90:22`.
- **Build is `./build.sh` only** — `cargo build -p multitop` without it embeds
  `missing` or stale; `check_agent_version` and `check_codesign` now enforce;
  `docs/detection-record.md` holds the full table.

