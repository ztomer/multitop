# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history and in the test suite. When an item here is finished, delete it rather
than ticking it off.

## 0. OPEN: upgrades fail because there is no stored sudo password

**Cause, and it was mine.** Verifying the password flow on 2026-08-03 I drove
the app through the SSO prompt with `XDG_CONFIG_HOME` isolated but *not* the
keychain -- the credential store is per-user, not per-config-dir -- which wrote
the literal string `test` over the real `multitop`/`__sso_master__` login-keychain
item. I deleted it rather than leave a wrong password in place. A later check
confirmed the state: no per-host entry exists for any of the four configured
hosts, and no SSO entry either.

So the app has no sudo password for any host. The remote `upgrade_cmd` needs
sudo, gets none, and the run fails. Nothing in the SSH or handshake path is
wrong -- the exact remote command was replayed against 192.168.0.33 by hand and
behaved correctly: the readiness sentinel arrived, the password was consumed,
and sudo rejected a deliberately wrong one with exit 1.

**The fix is one action:** press `s` in Server Settings and enter the SSO master
password again. Then confirm an upgrade completes.

If it still fails afterwards, the next thing to capture is the exact text in the
upgrade pane. No string in the workspace says "unreachable", so that wording is
coming from ssh or sudo output being surfaced verbatim, and knowing which line
it is decides where to look.

## 2. Decide the fate of two unused vault API functions

`UnlockedVault::remove_password` and `Vault::get_unlocked` are implemented and
tested with no production callers. `remove_password` would be per-host removal
*from the vault*, which is distinct from the credential-store deletion that `d`
already performs in the Passwords section — if that distinction is not wanted,
delete it. Both are listed in `tools/test_only_baseline.txt`.

## 2a. The review is not finished

Recorded because "are we done?" deserves an answer that is not a feeling.

The bar set for this work was: **a full review round that produces no new
findings.** No round has met it. The last four defects were all reported by the
user rather than found by the suite or by a review pass:

| Defect | Found by |
|--------|----------|
| `SIGTTIN` stopped the app and abandoned the terminal | user |
| The sudo handshake deadlocked, reported as an unreachable host | user |
| Tests reached the real OS keychain and blocked the suite on a dialog | user |
| Upgrades failing for want of a stored password | user |

By rule 1 -- fix the harness before the bug -- four in a row means the harness
still has holes and user QA is still the detection layer. Two were closed with
new gates (`check_keychain_isolation.py`) and new e2e coverage
(`config_panel_e2e.rs`, `filter_e2e.rs`); that is the pattern to keep.

Areas that have had no adversarial pass at all, and are where the last two bugs
came from: process and terminal lifecycle (signals, suspend/resume, child
process groups), and the rendering path (`ui.rs`, `refit.rs`, `ansi.rs`) beyond
the Configuration panel.

## 3. Clear the test-only baseline

`tools/test_only_baseline.txt` lists functions exercised by tests and by no
production path. The gate (`tools/check_test_only_code.py`) stops new ones
appearing; the existing list has to be worked down by hand, and the file can only
shrink — a stale entry fails the gate too.

Ten entries remain:

| Entry | Shape |
|-------|-------|
| `crates/vault/src/api.rs:remove_password` | Item 2 above |
| `crates/vault/src/api.rs:get_unlocked` | Item 2 above |
| `crates/multitop/src/app.rs:set_filtering` | Item 1 above |
| `crates/vault/src/crypto.rs:from_config` | Production always passes `argon2_params: None`, so the config path is unreachable |
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

## 4. Live validation of the recently shipped UI paths

These landed with unit and policy tests but have never been exercised against a
real terminal, a real vault, or real hosts. Green tests are not a shipped
feature.

| Path | How to exercise it |
|------|--------------------|
| Master-password rotation | `R` in the Passwords section; rotate, quit, restart, confirm the stored sudo passwords still decrypt with the new master password and not the old one |
| SSH import | `I` in the Servers section against a real `~/.ssh/config`; confirm nothing already configured lost its `upgrade_cmd` |
| Server-list editing | Add and remove a server while the app is running; confirm panels respawn, stats land on the right host, and no panel keeps a dead host's sparkline |
| Sudo-password handshake | A real upgrade on a host that needs sudo, confirming the password reaches `sudo -S` over stdin and never appears in `ps` |

The password handshake was verified live against three hosts once; the other
three rows have not been.

## 5. Confirm the two 2026-08-03 fixes against real hosts

Both were diagnosed from a screenshot and fixed with tests, and neither has been
seen working on a real machine yet.

**The app was stopped, not crashed.** The screenshot said
`[1] + suspended (tty input)` -- `SIGTTIN`, raised when a process touches the
controlling terminal from a background process group. At its default disposition
it *stops* the process, and a stopped process runs no destructors, so
`TerminalGuard` never restored the terminal: raw mode still on, alternate screen
still active, and the shell's prompt drawn on top of the last frame. That is why
the window looked empty and why a window screenshot showed nothing.
`run.rs` now registers handlers for `SIGTTIN`/`SIGTTOU` (a caught signal does
not stop the process) and rebuilds the terminal on `SIGCONT`.

**"Not reachable" after entering the password was a deadlock, not the network.**
The sudo handshake waited for the readiness sentinel with no time bound, and
closed the child's stdin only on the success path. A remote that stayed silent
left both sides waiting -- multitop for a line, the remote for its stdin to
close. `tasks::deliver_sudo_password` now bounds the wait and closes the pipe on
both paths, and says so instead of letting it surface as a connection failure.

To confirm: run a real upgrade on a host that needs sudo and watch it complete;
then `kill -TTIN` the running app from another terminal and check it keeps
drawing rather than stopping.

## 5a. Finish keychain isolation for tests

`tools/check_keychain_isolation.py` is a gate in the hook and in CI, and it
reports clean. It is not yet the whole story:

- The check is per test body and resolves helpers one level deep. It scans
  `crates/*/tests` only, so **unit tests inside `src/` are not covered** --
  and the vault's keychain use (`lockout.rs`, `rollback.rs`) is gated on a
  `use_keychain` flag rather than on `cfg(test)`, so a unit test can reach the
  real keychain.
- A probe that makes every real-keychain call panic still reported **9 hits**
  across the workspace after the `tests/` directories were cleaned. Those
  remaining hits have not been attributed yet. Reproduce with: replace each
  `keyring::Entry::new(...)` in `crates/vault/src/{lockout,rollback}.rs` and the
  non-mock branches of `crates/multitop/src/password_store.rs` with a `panic!`,
  then run the workspace suite and read the failing test names.

Until that reaches zero, running the suite can still raise a keychain dialog.

## 6. Rotate the sudo password used during live verification

The sudo password for the three test hosts was pasted into a Claude Code session
transcript on 2026-08-02 in order to verify the stdin handshake. It is therefore
on disk in `~/.claude/projects/`. Change it on all three machines.

## 7. `G` — per-pane CPU / memory / network graphs

Requested 2026-08-03. **Last** — start only once items 1-6 are closed.

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
- Sparklines are still behind the `P` toggle and marked experimental. Decide
  whether `G` replaces that toggle or sits beside it.

## Deferred

| Item | Why |
|------|-----|
| TPM2 wrapper | Would make Linux fingerprint unlock actually release a key. Until it exists, `fprintd` cannot unlock anything, so `try_unlock_biometric` does not prompt for it. |
| Post-quantum KEM | Not warranted for a device-local file threat model. |
