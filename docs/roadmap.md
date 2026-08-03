# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history and in the test suite. When an item here is finished, delete it rather
than ticking it off.

## 1. Filtering

**Status:** scaffolding only, with no way to reach it.

`filter_query`, `is_filtering`, `filtered_indices`, `set_filtering`, and the
`AppMode::Filtering` variant all exist in `crates/multitop/src/app.rs`.
`filtered_indices` implements substring matching on the host and has zero
callers; nothing writes `filter_query`; no key is bound and the keybar does not
mention it.

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

`tools/test_only_baseline.txt` lists the functions exercised by tests and by no
production path. The gate stops new ones appearing; the existing list has to be
worked down by hand. Items 1–2 above are its interesting entries; the rest are
unused accessors and one delegating wrapper.

## Deferred

| Item | Why |
|------|-----|
| TPM2 wrapper | Would make Linux fingerprint unlock actually release a key. Until it exists, `fprintd` cannot unlock anything, so `try_unlock_biometric` does not prompt for it. |
| Post-quantum KEM | Not warranted for a device-local file threat model. |
