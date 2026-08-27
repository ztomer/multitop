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
| Post-quantum KEM | Not warranted for a device-local file threat model. |

---

## Detection record

What found each defect, kept because the answer changed and the change is the
point. By rule 1 (fix the harness before the bug), a defect reported by a user
means the harness had a hole; the goal has always been to stop being the last
line of detection.

| Defect | Found by | Closed with |
|--------|----------|-------------|
| `SIGTTIN` stopped the app and abandoned the terminal | user | Signal handlers + `SIGCONT` rebuild |
| The sudo handshake deadlocked, reported as an unreachable host | user | Bounded wait, pipe closed on both paths |
| Tests reached the real OS keychain and blocked the suite on a dialog | user | `check_keychain_isolation.py` gate |
| Upgrades failing for want of a stored password | user | Per-host passwords; sudo rejection signalled distinctly |
| Answering the vault offer dropped the user out of Server Settings | user | Modals compose over the panel |
| The creation prompt took the master password three times | user | In-flight state; stale failure cannot undo a success |
| One upgrade cost two password prompts | user | Vault is the source of truth; no credential-store read to *report* |
| A `\r` progress bar logged one line per tick | user | `tasks::painted_states` |
| A `docker compose pull` block repainted with `ESC[nA` logged a copy per tick | user | `tasks::Painter` + `Msg::AuxRepaint` into `RingLines::overwrite_from_end` |
| The event loop wedged when an upgrade flooded the channel on another view | user | Fair select + bounded drain (budget=32) |
| `switch_stats` retired the in-flight `gen` and discarded upgrade output | user | Preserve `gen`+`scroll` for `STARTED` panels |
| Scrolling reset on every view switch | user | `enter_upgrade_view` no longer resets; mid-upgrade panels keep offset |
| Docker/Fetch showed "loading..." on re-entry despite cached data | user | Render cached payload on entry; `dims` threaded as parameter |
| Vault prompted for biometric + password when the user wanted one password | user | Skip biometric, go straight to password prompt |
| `/` matched two fixed fields, so a process, container, image or OS on screen was unfindable | user | `Panel::matches_filter` answers from the view the panel is in; the container image now reaches the client |
| A filtered grid re-split the screen but every agent kept rendering for the old pane size | roadmap | `agent_dims` measured from `regions`, fed `App::visible_panes()` |
| Touch ID unlock was unreachable: the fix for the double prompt left nothing able to start it | `check_test_only_code.py` | `Vault::biometric_available` decides which door to offer *before* a prompt goes up; `App::begin_vault_unlock` routes to it or to the master password |
| A session's reason for ending was kept or lost at random -- `select!` raced stderr against stdout's EOF | coverage run | Drain stderr before reporting the close |
| The stale-agent sweep generated `for f in agent-*; do rm -f "$f"; done` -- delete every agent including the one in use -- in any build with no agent embedded. Two tests asserted that shape as required, one of them naming the empty-keep case in a comment | roadmap (checking a user claim) | A catch-all delete never appears without something to keep; the sweep is a no-op when it knows nothing to spare |
| `multitop-agent --help` printed a binary monitor packet naming a host called `beelink (--help)`, and on older builds streamed forever. Any unrecognised flag became the *host label* | user (a hung test run) | `--help`, `--version` and unknown flags answered before the positional arguments are read |
| A test asserting "the spawn found the agent" passed on the one machine where it had not: it accepted `stdout.contains("multitop-agent")`, and the shell's own `command not found: multitop-agent` contains that | user | It skips with a stated reason when the binary is absent, and asserts a clean exit and real output when it is present |
| A test read a child's stdout to EOF with no bound, so an agent that streamed took the whole suite with it -- no failure, no cause, forever | user | Bounded, and pointed at a one-shot mode every agent version terminates on |
| Linux fingerprint unlock was ceremony: `fprintd` answered yes and no key was released, because nothing created a TPM2 wrapper | roadmap | `tpm2::seal` at create, `unseal` behind the verified finger. Machine binding, not biometric protection -- a TPM cannot check a fingerprint, and the module says so |
| The graph view's CPU heading was invisible, drawn onto the row the banner overwrites | user | A placeholder first line, like every other renderer emits |
| CI had been red on every run for days -- an Ubuntu runner with no `libdbus-1-dev`, so every cargo step died in a build script before compiling any of this project | user | The system library installed by a composite action every job uses; `check_gate_parity.py` so the three gate lists cannot drift again |
| `/` could only find a host by a process near the top of its table, because the agent truncates that table to what fits the pane -- and filtering makes panes bigger, so the same query answered differently a second later | user | The snapshot carries every distinct process name; the table stays capped |
| Esc/Enter in completed upgrade or secondary views quit rather than returning to monitor | user | `KeyCode::Esc` from views and `Enter` from completed upgrade route to `switch_stats` |
| Four defects in one five-change round (below) | adversarial review | see the table under "The streak is broken" |

### The streak is broken, and that is the point

For fourteen defects running, the answer in the middle column was "user". It is
no longer, and the last four entries are why:

* a **gate** found the dead Touch ID path -- `check_test_only_code.py` reported
  `unlock_biometric` as reachable only from tests, which is what a feature with
  no way to start it looks like from the outside;
* the **coverage run** found the lost session reason. The same test passed under
  `cargo test` and failed under instrumentation, because `select!` randomises
  which ready branch it takes and the slower build changed which one was ready
  first. A flake is a defect wearing a disguise: `Permission denied (publickey)`
  was being thrown away on a coin flip;
* the **adversarial review** found four in a single five-change round.

None of those three reached a user. That is the whole objective, and it is worth
stating plainly rather than leaving the reader to count rows.

### What the adversarial review found, and why the suite did not

| Defect | Why the suite missed it |
|--------|-------------------------|
| Pressing a view's own key -- a documented no-op -- threw away every pane's scroll position | The guard returned early *after* the reset, and the test asserted only that no work was spawned |
| Leaving the Upgrade view mid-run left the Monitor pane scrolled to the log's offset | Three regression tests pinned the *shared field* rather than the round trip, so the leak was asserted as correct in three files |
| `begin_vault_unlock` set a biometric wait its one caller overwrote on the next line, and returned a handle that caller discarded | Nothing read the state in between, so the dead assignment was invisible |
| A regression test still asserted "must await biometric before prompting for a password" -- the behaviour the fix removed | It stayed green *because* of the dead assignment above |

The last two are one defect. A test that pins behaviour the product no longer
has is worse than no test: it reports that the old path still works. The
generalisation, worth asking whenever a fix removes a step -- **what was testing
the step you just deleted, and is it still green?** If it is, it is now lying.

### The pattern that closes them

E2e tests that drive real `KeyEvent`s through `run::handle_key` and **count what
the presses actually started**, rather than asserting on the final state. Where
that is not enough, a structural gate: every defect above that a gate found was
found by a gate written for a *different* defect.
