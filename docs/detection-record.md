# Detection Record

What found each defect, kept because the answer changed and the change is the point. By rule 1 (fix the harness before the bug), a defect reported by a user means the harness had a hole; the goal has always been to stop being the last line of detection.

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
| Post-upgrade freeze: panes unresponsive, app unquittable | user | SIGUSR1/USR2 diag dumps (classifier); PTY stress in release exonerated suspects 2-3; `event_loop_e2e.rs` drives the real loop (harness-hole close). Suspect 1 fixed: credential reads moved off the loop thread (Layer 3) |
| `multitop-agent --help` printed a binary monitor packet naming a host called `beelink (--help)`, and on older builds streamed forever. Any unrecognised flag became the *host label* | user (a hung test run) | `--help`, `--version` and unknown flags answered before the positional arguments are read |
| A test asserting "the spawn found the agent" passed on the one machine where it had not: it accepted `stdout.contains("multitop-agent")`, and the shell's own `command not found: multitop-agent` contains that | user | It skips with a stated reason when the binary is absent, and asserts a clean exit and real output when it is present |
| A test read a child's stdout to EOF with no bound, so an agent that streamed took the whole suite with it -- no failure, no cause, forever | user | Bounded, and pointed at a one-shot mode every agent version terminates on |
| Linux fingerprint unlock was ceremony: `fprintd` answered yes and no key was released, because nothing created a TPM2 wrapper | roadmap | `tpm2::seal` at create, `unseal` behind the verified finger. Machine binding, not biometric protection -- a TPM cannot check a fingerprint, and the module says so |
| The graph view's CPU heading was invisible, drawn onto the row the banner overwrites | user | A placeholder first line, like every other renderer emits |
| CI had been red on every run for days -- an Ubuntu runner with no `libdbus-1-dev`, so every cargo step died in a build script before compiling any of this project | user | The system library installed by a composite action every job uses; `check_gate_parity.py` so the three gate lists cannot drift again |
| `/` could only find a host by a process near the top of its table, because the agent truncates that table to what fits the pane -- and filtering makes panes bigger, so the same query answered differently a second later | user | The snapshot carries every distinct process name; the table stays capped |
| Four defects in one five-change round (below) | adversarial review | see the table under "The streak is broken" |
| The upgrade channel was the one SSH channel that was not framed: raw text over `ssh -tt`, whose shape `ssh` decided. Probed against one host with one command, it produced three different byte streams -- multiplexed (`\n`), unmultiplexed (`\r\n`, which a stale *file* at the `ControlPath` silently selects), and local (two pipes, no pty). Seven of the rows above are defects in that one channel; none are in the three framed ones | user (output logged twice; upgrades leaving the app stuck) | `ProtoMode::Exec`: the agent runs the command on a pty it owns and reports it as framed packets. `ssh -tt` deleted |
| `painted_states("a\r")` is `["a", ""]`, whose last element is empty -- and a pty ends every line `\r\n`. On the unmultiplexed transport the whole upgrade log collapsed to blank lines. The line reader before it was safe by accident: `tokio`'s `Lines` strips the `\r` for you | rewriting the reader onto bytes | The CR of a CRLF is treated as part of the terminator, with a named regression test |
| `ControlMaster=auto` was documented as degrading to an unmultiplexed connection when its `ControlPath` cannot be bound. It does not: `ssh` prints `unix_listener: cannot bind` and **the command never runs**, on every channel. An account with no `~/.ssh` would have found multitop unable to reach anything | probing the comment as a claim | The directory is created 0700, the path is passed absolute, and the sharing options are dropped when it cannot be placed |
| The diagnostic dump announced itself with `eprintln!` -- onto the terminal the TUI holds in raw mode inside the alternate screen. Every signal scribbled a wrapped path across the operator's panel: a tool for reading a display that has stopped making sense was making it stop making sense. **Five sites; fixing one left four** | the tmux e2e harness, which signals repeatedly | One `diag::report`, which writes only when stderr is not a terminal |
| Nothing ran the python end-to-end suites. `cargo test` does not, and neither did the hook, CI or `local-ci.py` -- so the only layers that drive the built *binary* would have rotted quietly, while the suite reported all-green | writing them and looking for what ran them | All three run `pytest tests/`, required by `check_gate_parity.py` as a **directory** so a new suite cannot be forgotten |
| The 95% coverage floor had been **dead in CI for weeks**, exiting 127 on every push: `tools/coverage_check.sh` delegates to `$GOH/gates/coverage_gate.sh`, and no runner has a `gates_of_heck` checkout. The local gate passed the whole time | pre-release CI check while cutting v0.43.0 | CI checks out the (public) shared harness and sets `GOH_DIR`; the ratchet wrapper refuses to run rather than reporting clean when the harness is absent |
| The line-count ratchet and the fuzz build ran **only** in `scripts/local-ci.py`, each with a stated reason. `diag.rs` grew past its ceiling in a commit the hook passed, and `fuzz_proto` stopped compiling when the protocol gained a variant and reached a release | the pre-release gate run | Both in the hook and CI; the ratchet extracted to `tools/ratchet_check.py` so three callers share one definition; `check_gate_parity.py` enforces both |
| The pre-push hook ran the suite **twice** (`gate.sh --full` then `local-ci.py`, both ending in the coverage floor) and ran it again on **tag pushes**, where the commit is already gated. A release therefore paid for it four times, and the tag run is the one a timeout killed | cutting v0.43.0 | One run, and tag-only pushes skip it |
| `DEVELOPMENT.md` said "There is no `rust-toolchain.toml`, on purpose" — the file exists and pins the toolchain, and the reason given (that a pin would break `cargo fuzz`) is wrong because the fuzz gate uses `cargo +nightly`, which overrides a pin | reading the docs as a spec | Corrected, with what was wrong recorded rather than quietly replaced |
| Hello must be first, valid, and single — stale `0.44.0` inside `0.44.1` looped `Hello 0.44.0 vs 0.44.1` re-uploading same bytes; duplicate Hello is a protocol violation | user (stats/graphs failed, docker/fetch passed) | `Proto::Hello` `is_valid`/`is_compatible`/`needs_replacement` + `hello_seen` guard + `build.rs` panic for `release` + `tools/check_agent_version.py` |
| Embedded agent stale after version bump without `build.sh` — `0.44.1` binary embedded `0.44.0` `e084…` | user (same loop) | `build.rs` `panic!` for `release` when `CARGO_PKG_VERSION` not in `agent` bytes; `check_agent_version` gate |
| Ad-hoc `Identifier=multitop-abc123` per `cargo build` → `login.keychain` `genp multitop` prompted every rebuild, `Always Allow` only allowed that one binary | user (password prompt with “Always”) | `build.sh`/`cargo` `codesign --identifier com.ztomer.multitop` + `tools/check_codesign.py` auto-fix |
| Vault `seed` only from `panel.sudo_password` in memory — vault created empty even though `login.keychain` had `0.33:22`/`0.90:22` `mangarkongar`; `VaultUnlocked` never reloaded `vault.hosts()` into `panel` so upgrade view stayed `will prompt` | user (upgrade asked for sudo password) | `vault.rs:71` fallback `password_store::load` when `vault.get_password` is `None`; `app/vault.rs:219` `or_else(|| load().ok().flatten())`; `app/apply.rs:344` reload into `panels` on `VaultUnlocked` |
| Panels restored into Docker/Fetch came back dead — fresh gen-0 panels showing "connecting..." forever: no backing task is ever spawned for a restored view, and the monitor stream updates history without painting. One `d` press poisoned all panels *and* `state.toml`, so every later launch restored the dead view; the install looked guilty because the binary was never the variable | user (empty stats, agents "not connecting") | `Mode::for_startup` maps task-backed views to Monitor on both persist and restore (heals already-poisoned state files); `view_restore_test.rs` pins all six variants plus the fixed-point property |

### The streak is broken, and that is the point

For fourteen defects running, the answer in the middle column was "user". It is no longer, and the last four entries are why:

* a **gate** found the dead Touch ID path -- `check_test_only_code.py` reported `unlock_biometric` as reachable only from tests;
* the **coverage run** found the lost session reason. The same test passed under `cargo test` and failed under instrumentation, because `select!` randomises which ready branch it takes;
* the **adversarial review** found four in a single five-change round.

None of those three reached a user.

### What the adversarial review found, and why the suite did not

| Defect | Why the suite missed it |
|--------|-------------------------|
| Pressing a view's own key -- a documented no-op -- threw away every pane's scroll position | The guard returned early *after* the reset |
| Leaving the Upgrade view mid-run left the Monitor pane scrolled to the log's offset | Three tests pinned the *shared field* rather than the round trip |
| `begin_vault_unlock` set a biometric wait its one caller overwrote on the next line | Nothing read the state in between |
| A regression test still asserted "must await biometric before prompting for a password" | It stayed green *because* of the dead assignment above |

### Instruments that were measuring the wrong thing

Four times a probe reported no defect because it could not have seen one:

* an `ls` oracle compared `ssh host ls` against the same command run through a login shell on a pty -- two different commands;
* a fake `HOME` was set to test a missing `~/.ssh`, and the connection succeeded because `ssh` expands `~` from the passwd entry, not `$HOME`;
* `ps` on `PATH` is a replacement that rejects `-p`;
* the tmux harness resolved its pid by scanning for `"multitop"` and found the operator's own.

And twice a test passed while asserting nothing: `not in_flight` is true of a panel that never ran.

**Rule:** a probe that shows no defect has to be shown capable of showing one.

### The pattern that closes them

E2e tests that drive real `KeyEvent`s and **count what the presses actually started**, plus structural gates for the rest.
