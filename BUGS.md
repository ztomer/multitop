# Bugs

## a. No output on 192.168.0.90 on updater

**Status:** FIXED (2026-07-31)

### Root cause

192.168.0.90 has **no `upgrade_cmd` configured** — it is commented out in
`config.example.toml` and `-r <host>` servers get `upgrade_cmd: None`
(main.rs:129-134). When `u` ran, the panel for that server was put into a dead
end: a single dim "No upgrade_cmd configured for this server" line, with
`upgrade_state` left at `NIL` and `last_upgrade` empty. The confirm modal
claimed "run updates on all servers" and never disclosed the skip. The SSH
transport itself was proven healthy (10/10 remote E2E tests pass against the
real 192.168.0.90 host), so the perceived "no output" was the silent-skip UX.

### Fix

1. **`run_upgrade()` (app.rs:241-262)** — a server without `upgrade_cmd` now
   reaches a terminal `UpgradeState::DONE` with `upgrade_gen` recorded, a
   host-specific two-line message ("No upgrade_cmd configured for <host> —
   skipped" + a hint to set `upgrade_cmd`), and `last_upgrade` populated so the
   message persists.
2. **`upgrade_skip_hosts()` (app.rs:90-98)** — new helper listing hosts an
   upgrade will skip.
3. **`draw_upgrade_modal` (modals.rs:60-69)** — the confirm modal now lists
   "Skipped (no upgrade_cmd): <hosts>" with a config hint before the user
   confirms; popup height bumped to 12.
4. **`spawn_upgrade` (tasks.rs:186-213)** — `\r\n`/`\r` line splitting strips
   the line ending before splitting and drops whitespace-only fragments, so
   PTY output never yields spurious blank lines.

### Regression tests (crates/multitop/tests/upgrade_loop_e2e.rs)

- `test_upgrade_skip_server_reaches_terminal_state` — skipped server → DONE,
  message names host, says "skipped", references upgrade_cmd. Fails pre-fix.
- `test_upgrade_mixed_servers_only_configured_run` — only configured servers
  get `RunUpgrade`; skipped reaches DONE. Fails pre-fix.
- `test_upgrade_skip_message_persists_across_views` — skip message survives
  `switch_stats` → `show_upgrade_output`. Fails pre-fix.
- `test_upgrade_skip_then_u_shows_message_not_modal` — after a skip, pressing
  `u` shows the persisted message, not the modal / blank panel. Fails pre-fix.
- `test_upgrade_skip_hosts_helper_lists_unconfigured` — modal data helper.

**Verification:** 26/26 local upgrade tests + 10/10 remote E2E tests against
the real 192.168.0.90 host pass; full `cargo test -p multitop` green.

---

## b. Vault not working reliably (4 password attempts, 1 fingerprint)

**Status:** FIXED (2026-07-31)

### Root cause

The user's pattern — 4 manual password entries and 1 fingerprint — was the
TUI's fault, not the vault's: the `u` key handler never tried biometrics.
Whenever the vault was locked and the user pressed `u`, the TUI went straight
to a password prompt and called `unlock_with_password()` synchronously
(run.rs:267). The vault's `unlock_biometric()` API — which tries Secure
Enclave / Touch ID first, then falls back to the password — was never called
from the upgrade path. So every vault-gated upgrade demanded a password by
hand; the single "fingerprint" the user saw was the OS keychain prompt from
`password_store::load()`, not the vault's own biometric flow. Combined with
the lockout backoff (1s → 2s → 4s → 5-min) and the SE-key-invalidation errors
that were silently swallowed (api.rs:250), repeated manual entries produced
the "unreliable" experience.

### Fix

1. **Biometric-first unlock (run.rs `u` handler)** — pressing `u` with a
   locked vault now spawns an async `vault.unlock_biometric(false)` attempt.
   Success sends `Msg::VaultUnlocked(UnlockedVault)`; failure (unavailable,
   cancelled, or SE error) sends `Msg::VaultBiometricFailed`. The password
   prompt appears ONLY after a biometric failure, never first.
2. **`begin_vault_unlock()` (app.rs)** — new testable method encoding the
   decision: a locked vault enters the awaiting-biometric state and hands the
   caller the shared handle; `None` means no vault / already unlocked, so the
   handler proceeds to the upgrade modal. The `u` handler is now a thin wrapper
   over it.
3. **`Msg::VaultUnlocked` / `Msg::VaultBiometricFailed` (types.rs)** — new
   message variants handled in `apply()` (app.rs): success clears the awaiting
   state, stores the unlocked vault, and shows the upgrade modal; failure
   clears the awaiting state and shows the password prompt. `Msg` is now
   `#[derive(Debug)]` only (the unlocked vault isn't `Clone`/`PartialEq`).
4. **Redacted `Debug` for `UnlockedVault` (vault/src/api.rs)** — the unlocked
   vault now prints only its hosts + file path, never key material.
5. **Awaiting-biometric modal (modals.rs + ui.rs)** — while the SE prompt is
   up, the TUI renders a dedicated "Attempting Touch ID / fingerprint…" screen
   (`/theme[0]/`) and blocks key handling (`if app.vault_awaiting_biometric`)
   so a busy state can't be mistaken for a dead button.

### Regression tests (crates/multitop/tests/vault_upgrade_e2e.rs)

- `test_vault_locked_u_key_tries_biometric_first` — locked vault → biometric
  attempt, not password prompt; unlocked/no vault → proceeds to modal.
  **Proven to fail against pre-fix behavior** (password-prompt-first path).
- `test_vault_biometric_success_proceeds_to_modal` — `VaultUnlocked` stores
  the vault and opens the upgrade modal.
- `test_vault_biometric_failed_falls_back_to_password` — `VaultBiometricFailed`
  opens the password prompt.
- `test_vault_biometric_task_emits_fallback_on_unavailable` — an
  unavailable-biometrics `Err` is converted into `VaultBiometricFailed` (no
  lockout events, no password leakage).
- `test_vault_biometric_failures_do_not_trigger_lockout` — a rejected
  biometric does not count as a password failure toward the lockout backoff.

**Verification:** full `cargo test -p multitop -- --test-threads=1` green
(26 upgrade + 12 vault-upgrade E2E tests), full `cargo test -p multitop-vault`
green (112 tests).

---

## c. Sparklines don't really work

**Status:** FIXED (2026-07-31)

### Root cause

When sparklines were enabled, they were still invisible on anything but a very
wide header: `render_bar()` returned the full 30-sample history, and the header
code (ui.rs:342-366) **dropped the entire sparkline** whenever `M:<bar>` /
`C:<bar>` exceeded the available rule length (`len <= left_rule_len` failed).
On a 2-panel 80-column split, `left_rule_len` is ~10 vs the ~32 chars a full
30-sample bar needs, so the bar never rendered at all. Independent of width, a
0% sample rendered as a space (`BARS[0] == ' '`), making idle bars invisible
whitespace. And `reset_scroll()` only reset the *selected* panel even though
`run_upgrade()`/`show_upgrade_output()`/view toggles operate on all panels.

### Fix

1. **`render_bar_limited(max_chars)` (sparkline.rs)** — new method renders at
   most `max_chars` samples, keeping the **most recent** ones, so a narrow
   header still shows the latest trend. `render_bar()` delegates to it with the
   full capacity (back-compat).
2. **Truncate in the header (ui.rs:331-366)** — the header now asks for
   `render_bar_limited(left_rule_len - 2)` (and likewise for CPU) and always
   renders the result; the "drop the whole bar if it doesn't fit" branch is
   gone. Sparklines now appear at any width where the rule area is ≥ 3 chars.
3. **Visible zero bars (sparkline.rs)** — `BARS[0]` is now `▁` (U+2581) instead
   of `' '`, so a 0% value is a visible low block rather than whitespace.
4. **`reset_scroll()` (app.rs:520-524)** — now resets `scroll_offset` on
   **every** panel, matching the all-panels semantics of its callers
   (`run_upgrade`, `show_upgrade_output`, `toggle_fetch`, `toggle_docker`,
   `switch_stats`, and the `u` handler).

### Regression tests

- `crates/multitop/src/sparkline.rs` (unit):
  - `zero_value_renders_visible_block` — 0% values render as `▁`, not spaces.
  - `render_limited_keeps_most_recent_samples` — `render_bar_limited(n)` keeps
    the newest `n` samples and matches the tail of the full render; empty/zero
    limits return empty.
- `crates/multitop/tests/ui_resize_test.rs` (e2e render harness):
  - `sparkline_renders_truncated_on_narrow_split_panel` — renders a 2-panel
    app with full 30-sample histories at 45×12 and asserts a sparkline block
    glyph appears in the header AND the header fits the panel width.
    **Proven to fail against pre-fix behavior** (full-width bar dropped).
  - `reset_scroll_clears_all_panels_not_just_selected` — sets non-zero scroll
    on every panel, resets, asserts all are 0. Fails pre-fix (selected-only).

**Verification:** full `cargo test -p multitop -- --test-threads=1` green.

### Notes

- Sparklines remain **opt-in** (`show_sparklines = true` in config, or `p` in
  the settings panel); the default is still off per `config.example.toml`.
- The block glyphs `▁▂▃▄▅▆▇█` still require a Unicode-capable terminal; that
  is a terminal concern, not a code defect.

---

## d. Fix warnings, set up compilation cache

**Status:** Under investigation

### Findings

1. **868 clippy warnings** across the workspace (all-crates, all-targets). Breakdown
   by category:
   - 208× `unwrap()` on `Result` values
   - 65× `unwrap()` on `Option` values
   - 62× missing `# Errors` doc sections on `Result`-returning functions
   - 41× `#[must_use]` suggestions
   - 36× `panic!` in production code (test files)
   - 34× `#[must_use]` on functions
   - 25× unnecessary structure name repetition (`Self::` → `Self::`)
   - 25× `const fn` opportunities
   - 25× redundant closures
   - 11× unused `async` functions
   - Many cast/width warnings (u16→u8, usize→f64 precision, etc.)

2. **`cargo build` produces 2 warnings** in the release/dev build:
   - `crate/vault/src/crypto.rs:387` — unused import `File`
   - `crate/vault/src/mlock.rs:54` — variable does not need to be `mut`

3. **No compilation cache configured.** There is no `.cargo/config.toml` in the
   repo (neither at workspace root nor in `~/.cargo/`). `sccache` is not installed
   on this machine. `cargo` defaults to no caching beyond the local `~/.cargo/`
   registry cache; there is no `CARGO_INCREMENTAL` environment variable set.

4. **`Makefile` `clippy` target would currently fail.** It uses
   `-D warnings` (Makefile:19), which turns all 868 warnings into errors. This
   means `make clippy` and CI (`ci.yml`) would fail as-is.

5. **No `.cargo/config.toml` exists.** Without a cargo config file, there is no
   way to set up:
   - `RUSTC_WRAPPER` for sccache/llc-wrapper
   - `CARGO_INCREMENTAL = true` for faster incremental debug builds
   - `sccache` as a rustc wrapper via `[build] rustc-wrapper = "sccache"`
   - Build script flags for caching

6. **Release profile is optimized for binary size, not build speed.** The workspace
   `Cargo.toml` sets `lto = "fat"`, `codegen-units = 1` for release (Cargo.toml:40-43).
   These slow down incremental debug builds since they don't affect dev profile but
   indicate a preference for optimization over build speed. The agent package uses
   `opt-level = "z"` (Cargo.toml:48) which is appropriate for the uploaded binary.

7. **Build script warnings.** `build.rs` generates 6 clippy warnings. The build
   script embeds agent binaries via `env!("OUT_DIR")` (ssh_opts.rs:3), but there's
   no caching mechanism for the generated agents.rs content.

### Relevant files
- `Cargo.toml` (workspace) — lint configuration, profile settings
- `Makefile:19` — `clippy` target with `-D warnings`
- `crates/vault/src/crypto.rs:387` — unused import `File`
- `crates/vault/src/mlock.rs:54` — unnecessary `mut`
- `ci.yml` — CI pipeline (likely runs `cargo clippy`)
- No `.cargo/config.toml` exists in the repo

### Recommended next steps
- Add `.cargo/config.toml` with sccache wrapper config
- Install sccache (`brew install sccache`)
- Consider `#[allow(clippy::unwrap_used)]` for test modules and `#[allow(clippy::panic)]` for tests
- Run `cargo clippy --fix` to auto-fix the 119+ auto-fixable suggestions in the lib target
- Split warnings into: fix unused imports/doc lint first, then address unwrap/expect/panic
