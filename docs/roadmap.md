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

## 5. Rotate the sudo password used during live verification

The sudo password for the three test hosts was pasted into a Claude Code session
transcript on 2026-08-02 in order to verify the stdin handshake. It is therefore
on disk in `~/.claude/projects/`. Change it on all three machines.

## Deferred

| Item | Why |
|------|-----|
| TPM2 wrapper | Would make Linux fingerprint unlock actually release a key. Until it exists, `fprintd` cannot unlock anything, so `try_unlock_biometric` does not prompt for it. |
| Post-quantum KEM | Not warranted for a device-local file threat model. |
