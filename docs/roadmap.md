# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history and in the test suite. When an item here is finished, delete it rather
than ticking it off.

## 1. Filtering

**Status:** scaffolding only, with no way to reach it.

`filter_query`, `is_filtering`, `filtered_indices`, `set_filtering`, and the
`AppMode::Filtering` variant all exist in `crates/multitop/src/app.rs`
(lines 31, 66, 490–512). `filtered_indices` implements substring matching on the
host and has zero callers; nothing writes `filter_query`; no key is bound and the
keybar does not mention it.

**What is missing:** a key (`/` is conventional) that enters `AppMode::Filtering`
and captures typed characters into `filter_query`, `Esc` to leave, `ui.rs`
honouring `filtered_indices` when laying out panels, and a keybar hint. Decide
whether filtering hides panels or dims them — hiding changes the grid geometry
mid-session, which the region layout may not expect.

Either build it or delete the scaffolding. Leaving a half-feature in place is
what kept it invisible this long.

## 2. Decide the fate of two unused vault API functions

`UnlockedVault::remove_password` and `Vault::get_unlocked` are implemented and
tested with no production callers. `remove_password` would be per-host removal
*from the vault*, which is distinct from the credential-store deletion that `d`
already performs in the Passwords section — if that distinction is not wanted,
delete it. Both are listed in `tools/test_only_baseline.txt`.

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
