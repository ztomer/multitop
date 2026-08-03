# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history and in the test suite. When an item here is finished, delete it rather
than ticking it off.

## 1. Live confirmation of everything that is green but unseen

The whole "never run against a real terminal, a real vault, or a real host" list,
in one place. It used to be three sections describing three batches of work; the
batches are shipped and only the confirmation is outstanding, and it is the same
kind of work every time.

Green tests are not a shipped feature. Each row is one sitting with the real app.

| Path | How to exercise it | Ever done? |
|------|--------------------|-----------|
| Per-host sudo passwords | Give each Ubuntu host its own password with `Enter`, run an upgrade, check it completes | no |
| Refused password vs failing command | Set one host's password deliberately wrong; the panel must say the password was refused, not that the command failed | remote side only, against 192.168.0.33 |
| Master-password rotation | `R`, rotate, quit, restart; the stored sudo passwords must decrypt with the new master password and not the old one | no |
| SSH import | `I` against a real `~/.ssh/config`; nothing already configured may lose its `upgrade_cmd` | no |
| Server-list editing | Add and remove a server while running; panels respawn, stats land on the right host, no panel keeps a dead host's sparkline | no |
| Sudo handshake over stdin | A real upgrade on a host that needs sudo; the password must reach `sudo -S` and never appear in `ps` | yes, three hosts |
| `SIGTTIN` no longer stops the app | `kill -TTIN` the running app from another terminal; it must keep drawing rather than stopping and abandoning the terminal | no |
| Sudo handshake deadlock | A remote that stays silent must report a bounded wait, not surface as an unreachable host | no |
| **One prompt per upgrade** | With a vault present, press `u` twice from a fresh start and count the password prompts. Exactly one, and it must be multitop's own "Enter vault master password", not a macOS keychain dialog | no |
| **Vault creation asks once** | Save a first password, type a master password, press Enter once and wait. One prompt, one vault, and Server Settings still on screen underneath | no |
| **Progress output** | Run an upgrade whose command has a `\r` progress bar (`apt upgrade`); the log must gain one line per bar, not one per tick | no |

The last three are the 2026-08-03 evening fixes. The upgrade-prompt row is the
one that matters most: the diagnosis was that the *first* `u` read the OS
credential store to report on credentials, and on macOS that is a system dialog
raised before the vault is ever unlocked. If a second prompt still appears, and
it is multitop's own, the diagnosis was wrong and the state machine needs
another pass.

## 2. Decide the fate of two unused vault API functions

`UnlockedVault::remove_password` and `Vault::get_unlocked` are implemented and
tested with no production callers. `remove_password` would be per-host removal
*from the vault*, which is distinct from the credential-store deletion that an
emptied password field already performs — if that distinction is not wanted,
delete it. Both are listed in `tools/test_only_baseline.txt`.

## 3. The adversarial review is not finished

Recorded because "are we done?" deserves an answer that is not a feeling.

**The bar: a full review round that produces no new findings.** No round has
ever met it. Not "we stopped early" -- the review has never once terminated
correctly.

### Where it has been

| Area | Coverage | How it ended |
|------|----------|--------------|
| Vault + credential path | 7 rounds (1, 2, 3, 4, 4b, 5, 6), all 2026-08-01 | **On a finding.** Round 6 found that a release binary could silently use the mock keystore, and the review stopped there. The next day `29bdbb3` -- rotating the master password could destroy the vault -- turned up during feature work. The subsystem was still producing defects when review stopped looking. |
| Agent parsing / protocol | 6 fuzz targets, 114M iterations, 0 crashes; plus targeted hardening (bogus chunk size, >64 KiB payload desync, null `ifa_addr`) | Mechanized, still running when asked. The only area no user-reported defect has come from. |
| Configuration panel | Keystroke-through-render e2e plus a bounded sweep of every key sequence | Not a review round; a harness that closes one class. |

Every round that ran found something. That is evidence the rounds were
productive *and* evidence they stopped too early.

### Where it has never been

| Area | Files | What that has already cost |
|------|-------|----------------------------|
| Terminal / process lifecycle | `run.rs` event loop, signal handling, child process groups | `SIGTTIN` stopped the app and abandoned the terminal -- found by the user |
| SSH + upgrade transport | `ssh.rs`, `tasks.rs` | The sudo handshake deadlock, and a refused password reported as a failing command -- both found by the user |
| Rendering | `ui.rs`, `refit.rs`, `ansi.rs` beyond the Configuration panel | A footer clipped at 80 and 96 columns, and a modal clipping its own footer -- found by rendering a frame and looking at it, not by any test |
| Persistence | `config.rs`, `state.rs` | Non-atomic state writes, and config comments destroyed on every write -- both found ad hoc |

### The detection record

Every defect below was reported by the user, not by the suite or by a review
pass:

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

Eight in a row. By rule 1 -- fix the harness before the bug -- that means the
harness still has holes and user QA is still the detection layer. The pattern to
keep is what closed the last four: e2e tests that drive real `KeyEvent`s through
`run::handle_key` and **count what the presses actually started**, rather than
asserting on the final state. The final state can look correct while three
vaults' worth of work happened.

### The review log

Findings from the vault rounds exist only in commit messages. "What did round 4
look at and decide was fine?" is unanswerable, which is why the table above can
say which areas were *touched* and not which are *clean*. Rounds from here down
record their scope and their negative results, not only their fixes.

#### Round A -- async prompts, 2026-08-03

**Scope:** every site in `run::handle_key` and `password_actions::apply` that
spawns a task. Seven of them. Each asked: what does the second press do while
the first is in flight; what happens when a stale result arrives after a newer
one succeeded; does the UI say work is happening.

**Found (2):**

- *Master-password rotation had no in-flight state.* The prompt closes on
  Enter because the work goes off-thread, so `r` was accepted again
  immediately. `change_password` reads the vault, rewraps the key and writes it
  back; two overlapping both unlock with the *old* password and both write, so
  the last silently wins while both report success. A mistyped current password
  also spends two of the kill-resistant limiter's tries instead of one.
- *Saving a password killed an upgrade already running on that host.*
  `mode == Upgrade` holds for the whole session once `u` has been pressed, so
  any save while the upgrade view was showing took the resume branch -- which
  replaces the panel's task and aborts what was there. Children are
  `kill_on_drop`, so that killed the SSH session of a running `apt upgrade`,
  interrupting a package transaction on the real machine and leaving the remote
  lock file behind. `execute_cmds` refuses to abort a running upgrade for
  exactly this reason; this path disagreed with it.

**Worth its own line:** the second defect was *pinned by an existing test*.
`tasks_e2e_test::test_task_cancellation_on_panel_switch` asserted "this SHOULD
cancel panel 0's old task". A test asserting the defect is worse than no test:
it makes the bug a requirement and the fix a regression. Check what a failing
test is actually protecting before believing it.

**Checked and already correct (5):** vault unlock by password
(`vault_verifying` gates re-entry, each attempt bumps the epoch); biometric
unlock (`vault_awaiting_biometric` gates it, only fires from `Locked`); vault
creation (fixed earlier the same day); upgrade start from the modal
(`upgrades_in_flight` blocks a second run, and `execute_cmds` will not abort a
running upgrade); fetch/docker/monitor respawns (replace-and-abort is correct
for them, and `c`/`m` no-op when the sort has not changed). Every `Msg` handler
that writes state on failure was re-checked for the stale-result class; all are
epoch- or generation-guarded.

**Next round:** terminal and process lifecycle, which has the worst record of
the never-reviewed areas.

### The next round: terminal and process lifecycle

Round A is done (above). The next area is the one with the worst record among
those never reviewed: `run.rs`'s event loop, signal handling, and the child
process groups behind every SSH session.

What it should ask:

- Suspend and resume (`SIGTSTP` arriving before the handlers are installed,
  `SIGCONT` racing a redraw), and what a `SIGWINCH` during either does.
- Whether any child can outlive the app. Every child is `kill_on_drop`, but a
  task that is never dropped -- one leaked by a `replace` that does not abort --
  keeps its SSH session alive against a host that is no longer shown.
- The quit path: `abort_all` marks `STARTED` panels `DONE`, but an upgrade
  killed mid-transaction leaves the *remote* lock file behind, and nothing tells
  the user that happened.
- Terminal restoration on every exit: panic (the release profile aborts, so
  `Drop` does not run -- `ratatui::init`'s hook is the only cover), `SIGHUP`,
  and the terminal going away mid-frame.

## 4. Clear the test-only baseline

`tools/test_only_baseline.txt` lists functions exercised by tests and by no
production path. The gate (`tools/check_test_only_code.py`) stops new ones
appearing; the existing list has to be worked down by hand, and the file can only
shrink — a stale entry fails the gate too.

Nine entries remain:

| Entry | Shape |
|-------|-------|
| `crates/vault/src/api.rs:remove_password` | Item 2 above |
| `crates/vault/src/api.rs:get_unlocked` | Item 2 above |
| `crates/vault/src/crypto.rs:from_config` | Reached only when `argon2_params` is `Some`, which now happens only under the test-mock flag (`vault::config_for`) |
| `crates/agent/src/render.rs:frame_height` | Accessor |
| `crates/multitop/src/app.rs:had_upgrade` | Accessor |
| `crates/multitop/src/app.rs:vault_unlocked` | Accessor |
| `crates/multitop/src/panel.rs:set_sudo_password` | Accessor |
| `crates/multitop/src/sparkline.rs:render_bar` | Delegating wrapper |
| `crates/vault/src/lockout.rs:uses_keychain` | Accessor |

The reason this is worth the effort is not the dead function itself: it is that
a *duplicate* of its logic is what production calls, the tests guard the dead
copy, and the live copy drifts unwatched. `rollback::parse_stored_counter` and
`LockoutState::on_failure` were both exactly that.

## 5. Finish keychain isolation for tests

`tools/check_keychain_isolation.py` is a gate in the hook and in CI, and it
reports clean. It is not yet the whole story:

- The check is per test body and resolves helpers one level deep. It scans
  `crates/*/tests` only, so **unit tests inside `src/` are not covered** --
  and the vault's keychain use (`lockout.rs`, `rollback.rs`) is gated on a
  `use_keychain` flag rather than on `cfg(test)`, so a unit test can reach the
  real keychain.
- A probe that makes every real-keychain call panic reported **9 hits** across
  the workspace after the `tests/` directories were cleaned. Those hits have not
  been attributed yet. Reproduce with: replace each `keyring::Entry::new(...)` in
  `crates/vault/src/{lockout,rollback}.rs` and the non-mock branches of
  `crates/multitop/src/password_store.rs` with a `panic!`, then run the workspace
  suite and read the failing test names. Re-run the probe first — one source of
  hits was multitop's own vault construction, which now follows the mock flag
  through `vault::config_for`, so the count may already be lower.

Until that reaches zero, running the suite can still raise a keychain dialog.

## 6. The other half of in-place progress output

Reported 2026-08-03: "when updating a line in place (e.g. docker update
percentages) it adds all the update screen instead."

Half of it is fixed. A tool that repaints **one** line with carriage returns
(`apt`, `curl`, a single-layer `docker pull`) now contributes one line to the
log -- the state it ended on -- instead of one line per tick. `tasks::painted_states`
does it and `test_e2e_carriage_return_progress_logs_one_line` pins it.

The half that is not: a tool that repaints **several** lines does it with cursor
movement, not carriage returns -- `docker compose pull` prints a block per
layer, then `ESC[nA` back up and prints the whole block again. Every repaint is
a genuine run of `\n`-terminated lines, so the log grows by a screenful per
frame and nothing above catches it.

Fixing that means interpreting the movement rather than stripping it: a small
virtual screen per panel (`CUU`/`CUD`/`CR`/`EL`/`ED`, absolute `CUP` ignored)
that upgrade output is written *into*, with the panel rendering the screen
rather than a list of lines. `ansi.rs` already parses SGR and drops everything
else, so this is a second, smaller state machine beside it -- not a dependency
on a full terminal emulator crate, and not the multiplexer the shape of it
suggests.

Worth deciding before starting: whether the durable `last_upgrade` log keeps
every frame (scrollback of what really happened) while only the live view is
collapsed, or whether the collapsed view *is* the log. They are different
products.

## 7. Rotate the sudo password used during live verification

The sudo password for the three test hosts was pasted into a Claude Code session
transcript on 2026-08-02 in order to verify the stdin handshake. It is therefore
on disk in `~/.claude/projects/`. Change it on all three machines.

## 8. `G` — per-pane CPU / memory / network graphs

Requested 2026-08-03. **Last** — start only once items 1-7 are closed.

A new view alongside the existing ones, bound to `G` and placed immediately to
the right of `F` (Fetch) in the keybar, drawing CPU, memory and network history
as graphs inside each pane. Use btop's graph glyphs — the braille-style
sub-cell blocks that give several vertical steps per character row — rather
than the single-row bar the sparkline uses today.

Points to settle before writing any of it:

- The agent currently ships point-in-time samples; a graph needs history. Decide
  whether the ring buffer lives in the agent (more data over the wire, survives
  a view switch) or in `Panel` (cheap, but empty for the first N seconds after
  a panel is rebuilt by `replace_panels`).
- `crates/multitop/src/sparkline.rs` already holds per-panel history and a
  renderer. Extend it rather than starting a second one — a parallel renderer
  is how the six drifted previews happened elsewhere.
- Sparklines are still behind the `S` toggle in Server Settings and marked
  experimental. Decide whether `G` replaces that toggle or sits beside it.

## Deferred

| Item | Why |
|------|-----|
| TPM2 wrapper | Would make Linux fingerprint unlock actually release a key. Until it exists, `fprintd` cannot unlock anything, so `try_unlock_biometric` does not prompt for it. |
| Post-quantum KEM | Not warranted for a device-local file threat model. |
