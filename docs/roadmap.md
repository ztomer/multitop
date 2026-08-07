# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history and in the test suite. When an item here is finished, delete it rather
than ticking it off.

## 1. Coverage pass — 95% floor (multitop crate)

`tools/coverage_check.sh` runs `cargo llvm-cov -p multitop` with
`--fail-under-lines 95`, ignoring OS-bound files (`ssh.rs`, `password_store.rs`,
`sparkline.rs`, `main.rs`, `spawn.rs`). **Currently at 83.46%.** The gate is
structural: enforced by CI job + pre-commit hook, cannot be bypassed with
`--no-verify`.

Biggest gaps (all need frame-inspection or async-loop tests):
- `run.rs` event-loop body (the `tokio::select!` block — hard to drive from a test)
- `ui.rs` draw functions (need `TestBackend` frame inspection, assert on buffer contents)
- `tasks.rs` `spawn_upgrade` internals (partially covered by integration tests)
- `modals.rs` draw functions

The harness exists: `tests/coverage_e2e.rs` already drives the loop with scripted
events and inspects the `TestBackend` buffer. Extend that pattern.

## 2. File-split pass — no file over 500 LOC

`run.rs` **split 2026-08-05** into `run/` submodules (all under 500 LOC):
`mod.rs` (127), `terminal.rs` (159), `tasks.rs` (122), `dims.rs` (137),
`event_loop.rs` (401), `handle_key.rs` (438), `spawn.rs` (227).

Remaining:

| File | Lines | Split target |
|------|-------|--------------|
| `app.rs` | 1547 | Apply state machine / upgrade flow / vault state / view switches |
| `ui.rs` | 1107 | Draw / layout / keybar / modals |
| `tasks.rs` | 837 | spawn_upgrade / streaming / painted_states |
| `ssh.rs` | 810 | Connection / command / upload |
| `stream.rs` | 631 | Handshake / packet / framing |
| `config.rs` | 547 | Load / save / validation |
| `passwords.rs` | 779 | Editor / actions / draft |

Split cleanly at module boundaries. Each new file = one `mod` in `lib.rs`.

NOTE: Splitting impl-block-heavy files (app.rs, ui.rs, tasks.rs) requires care —
each submodule needs its own imports and `crate::` paths for cross-module types.
Multiple attempts hit import/visibility issues; this needs a careful pass with
function-boundary-aware splitting.

BLOCKER: nightly compiler (1.99.0-nightly) has a regression where `let-else`
patterns fail with "size for values of type `str` cannot be known at compile
time". This prevents the codebase from compiling at all. Pin to an older
nightly or wait for the fix before continuing.

### Done

| File | Split into |
|------|------------|
| `run.rs` | `run/` (`mod.rs`, `terminal.rs`, `tasks.rs`, `dims.rs`, `event_loop.rs`, `handle_key.rs`, `spawn.rs`) |

## 3. Magic-number gate — no hardcoded values

No hardcoded numbers, strings, or magic numbers in source. Extract to named
constants. New CI + pre-commit gate (a `tools/check_magic_numbers.py` or similar).

Examples to catch: `budget = 32`, `MAX_STDERR_LINES`, timeout durations, sentinel
strings, exit codes, port numbers, array sizes.

## 4. Adversarial code review — five bugfix changes

Full expert-panel review (Linus, Carmack, Hashimoto, Uncle Bob) of every change
from the five-bugfix round. Must converge (all personas return NO FURTHER COMMENTS)
before the round is done.

**The bar: a full review round that produces no new findings.**

### How disagreements are settled (owner decree, 2026-08-03)

Two experts disagreeing is not resolved by the orchestrator. Bring in a third.
For UX/UI questions the third is Kare, and her call is final. Correctness, safety,
gates and cost are not UI questions and do not go to this tiebreak.

## 5. Clear the test-only baseline

`tools/test_only_baseline.txt` lists functions exercised by tests and by no
production path. Six entries remain:

| Entry | Shape |
|-------|-------|
| `crates/vault/src/crypto.rs:from_config` | Reached only when `argon2_params` is `Some`, which now happens only under the test-mock flag |
| `crates/agent/src/render.rs:frame_height` | Accessor |
| `crates/multitop/src/app.rs:had_upgrade` | Accessor |
| `crates/multitop/src/app.rs:vault_unlocked` | Accessor |
| `crates/multitop/src/panel.rs:set_sudo_password` | Accessor |
| `crates/vault/src/lockout.rs:uses_keychain` | Accessor |

The risk: a *duplicate* of the logic is what production calls, the tests guard
the dead copy, and the live copy drifts unwatched.

## 6. The other half of in-place progress output

A tool that repaints **one** line with carriage returns (`apt`, `curl`) now
contributes one line to the log. A tool that repaints **several** lines with
cursor movement (`docker compose pull`, `ESC[nA`) still floods the log.

Fix: a small virtual screen per panel (`CUU`/`CUD`/`CR`/`EL`/`ED`) that upgrade
output is written *into*, with the panel rendering the screen rather than a list
of lines. `ansi.rs` already parses SGR; this is a second, smaller state machine
beside it.

## 7. `G` — per-pane CPU / memory / network graphs

A new view bound to `G`, drawing CPU, memory and network history as graphs inside
each pane. Use btop's braille-style sub-cell blocks.

Points to settle: whether the ring buffer lives in the agent (more data over the
wire, survives a view switch) or in `Panel` (cheap, but empty for the first N
seconds after a panel is rebuilt).

## 8. A filtered grid tells the agents how big its panes are

Filtering hides non-matching panes and re-splits the grid, but the agent still
renders for a pane one quarter of that size. The fix: hand `refresh` the
`filtered_indices().len()` count and trigger a refresh on filter change (debounced).

## Deferred

| Item | Why |
|------|-----|
| TPM2 wrapper | Would make Linux fingerprint unlock actually release a key. Until it exists, `fprintd` cannot unlock anything. |
| Post-quantum KEM | Not warranted for a device-local file threat model. |

---

## Detection record

Every defect below was reported by the user, not by the suite or by a review
pass. By rule 1 (fix the harness before the bug), each one means the harness
still has holes and user QA is still the detection layer.

| Defect | Closed with |
|--------|-------------|
| `SIGTTIN` stopped the app and abandoned the terminal | Signal handlers + `SIGCONT` rebuild |
| The sudo handshake deadlocked, reported as an unreachable host | Bounded wait, pipe closed on both paths |
| Tests reached the real OS keychain and blocked the suite on a dialog | `check_keychain_isolation.py` gate |
| Upgrades failing for want of a stored password | Per-host passwords; sudo rejection signalled distinctly |
| Answering the vault offer dropped the user out of Server Settings | Modals compose over the panel |
| The creation prompt took the master password three times | In-flight state; stale failure cannot undo a success |
| One upgrade cost two password prompts | Vault is the source of truth; no credential-store read to *report* |
| A `\r` progress bar logged one line per tick | `tasks::painted_states` |
| The event loop wedged when an upgrade flooded the channel on another view | Fair select + bounded drain (budget=32) |
| `switch_stats` retired the in-flight `gen` and discarded upgrade output | Preserve `gen`+`scroll` for `STARTED` panels |
| Scrolling reset on every view switch | `enter_upgrade_view` no longer resets; mid-upgrade panels keep offset |
| Docker/Fetch showed "loading..." on re-entry despite cached data | Render cached payload on entry; `dims` threaded as parameter |
| Vault prompted for biometric + password when the user wanted one password | Skip biometric, go straight to password prompt |

Thirteen in a row. The pattern that closed the last five: e2e tests that drive
real `KeyEvent`s through `run::handle_key` and **count what the presses actually
started**, rather than asserting on the final state.
