# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history and in the test suite. When an item here is finished, delete it rather
than ticking it off.

## Open items

None. `tools/test_only_baseline.txt` is empty and every item that was here has
shipped. Add the next one above this line and delete this paragraph.

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
| A `docker compose pull` block repainted with `ESC[nA` logged a copy per tick | `tasks::Painter` + `Msg::AuxRepaint` into `RingLines::overwrite_from_end` |
| The event loop wedged when an upgrade flooded the channel on another view | Fair select + bounded drain (budget=32) |
| `switch_stats` retired the in-flight `gen` and discarded upgrade output | Preserve `gen`+`scroll` for `STARTED` panels |
| Scrolling reset on every view switch | `enter_upgrade_view` no longer resets; mid-upgrade panels keep offset |
| Docker/Fetch showed "loading..." on re-entry despite cached data | Render cached payload on entry; `dims` threaded as parameter |
| Vault prompted for biometric + password when the user wanted one password | Skip biometric, go straight to password prompt |
| A filtered grid re-split the screen but every agent kept rendering for the old pane size | `agent_dims` measured from `regions`, fed `App::visible_panes()` |
| Touch ID unlock was unreachable: the fix for the double prompt left nothing able to start it | `Vault::biometric_available` decides which door to offer *before* a prompt goes up; `App::begin_vault_unlock` routes to it or to the master password |

Thirteen in a row. The pattern that closed the last five: e2e tests that drive
real `KeyEvent`s through `run::handle_key` and **count what the presses actually
started**, rather than asserting on the final state.

### The streak is broken, and that is the point

The adversarial review of those last five changes found four defects the suite
did not, and none of them reached a user first:

| Defect | Why the suite missed it |
|--------|-------------------------|
| Pressing a view's own key — a documented no-op — threw away every pane's scroll position | The guard returned early *after* the reset, and the test asserted only that no work was spawned |
| Leaving the Upgrade view mid-run left the Monitor pane scrolled to the log's offset | Three regression tests pinned the *shared field* rather than the round trip, so the leak was asserted as correct in three files |
| `begin_vault_unlock` set a biometric wait its one caller overwrote on the next line, and returned a handle that caller discarded | Nothing read the state in between, so the dead assignment was invisible |
| A regression test still asserted "must await biometric before prompting for a password" — the behaviour the fix removed | It stayed green *because* of the dead assignment above |

The last two are one defect. A test that pins behaviour the product no longer
has is worse than no test: it reports that the old path still works. The
generalisation, worth asking whenever a fix removes a step — **what was testing
the step you just deleted, and is it still green?** If it is, it is now lying.
