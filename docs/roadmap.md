# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history and in the test suite. When an item here is finished, delete it rather
than ticking it off.

## 1. The adversarial review is not finished

Recorded because "are we done?" deserves an answer that is not a feeling.

**The bar: a full review round that produces no new findings.** No round has
ever met it. Not "we stopped early" -- the review has never once terminated
correctly.

### How disagreements are settled  (owner decree, 2026-08-03)

**Two experts disagreeing is not resolved by the orchestrator. Bring in a
third.** Whoever is running the review does not get to average two opinions, or
to pick the one they already agreed with. Find a third expert whose bar bears on
the question, put the disagreement to them as posed, and take their answer.

**For UX and UI questions the third is Kare, and her call is final.** Layout,
legibility, labels, glyphs, what a screen shows and how it degrades: no further
arbitration.

**The boundary, and it matters.** Correctness, safety, gates and cost are not UI
questions and do not go to this tiebreak:

- Whether an action can destroy something the operator did not intend -- and
  whether they were told first -- is Hashimoto's, and it stays binding.
- Frame-loop and allocation cost is Carmack's, and it stays binding.
- The structural gates (no emoji, keychain isolation, clippy, fmt, test-only
  code) answer to nobody.

A question does not become a UI question by being visible. "Is this word
truncated" is UI. "Does this key kill a package transaction on a production
server" is not, however it is drawn.

### Where it has been

| Area | Coverage | How it ended |
|------|----------|--------------|
| Vault + credential path | 7 rounds (1, 2, 3, 4, 4b, 5, 6), all 2026-08-01 | **On a finding.** Round 6 found that a release binary could silently use the mock keystore, and the review stopped there. The next day `29bdbb3` -- rotating the master password could destroy the vault -- turned up during feature work. The subsystem was still producing defects when review stopped looking. |
| Agent parsing / protocol | 6 fuzz targets, 114M iterations, 0 crashes; plus targeted hardening (bogus chunk size, >64 KiB payload desync, null `ifa_addr`) | Mechanized, still running when asked. The only area no user-reported defect has come from. |
| Configuration panel | Keystroke-through-render e2e plus a bounded sweep of every key sequence | Not a review round; a harness that closes one class. |
| Terminal / process lifecycle | Round C, 2026-08-04, twenty-six passes so far, with `tests/event_loop_e2e.rs` built for it | **On findings, twenty-four times out of twenty-six.** 7, 3, 1, 2, 3, 1, 1, 4, 1, 2, 3, 2, 2, 2, 1, 1, 1, 2, 2, 2, 0, 1, 1, 1. The twenty-fifth fixed the twenty-fourth's finding rather than opening new ground, so it adds no count of its own. The one zero is the twenty-first, and the twenty-second -- finishing the area the twenty-first left half-open -- found something immediately. A partial pass coming back empty is not the bar, and this is the evidence. What is running out is unexamined surface, not defects. Two classes account for most of them: *one quantity derived in two places by different rules*, and class H, *a failure reported as something else* -- the eighth pass was class H four times out of four. **The next pass re-covers `ui::draw`.** The twenty-sixth read its composition and found the banner eating a one-line body; by this round's own record, one pass over an area has never been enough. |
| SSH + upgrade transport | Covered by Round C where the lifecycle reaches it -- child process groups, the two output streams, the session handshake, the packet-decode boundary, and (eighth pass) every `Err` path out of `next_packet` plus the agent-replacement repair | Partial. `read_handshake`, `interpret_packet`, `framing_lost` and `describe_failure` have seams and tests; the bootstrap retry was read and found correct; the lock wrappers were read in the twelfth pass and `upload_agent`'s framing in the thirteenth. The bootstrap quoting and the `-tt` pty path were read in the fourteenth. No named part of the transport is unreviewed now. |

Every round that ran found something. That is evidence the rounds were
productive *and* evidence they stopped too early.

### The keychain probe, and what it is for  (2026-08-04)

The one procedure that answers "can the suite raise a credential dialog"
without waiting for it to happen to someone. Replace every real-keychain call
with a `panic!` and run the workspace:

- `keyring::Entry::new(...)` in `crates/vault/src/lockout.rs` (2 sites) and
  `crates/vault/src/rollback.rs` (2 sites)
- `password_store::entry()` in `crates/multitop/src/password_store.rs`

Last run 2026-08-04: **zero hits**, 47 test binaries green. The figure recorded
here before was nine, from before multitop's vault construction started
following the mock flag through `vault::config_for`.

This is a procedure rather than a gate because it needs a recompile. What *is*
a gate is `tools/check_keychain_isolation.py`, and its blind spot -- it scanned
`crates/*/tests` only, so unit tests inside `src/` were never checked -- is
closed: it now also sweeps `crates/vault/src`, which is where the hazard is
(the vault gates its keychain use on a runtime `use_os_keychain` flag, not on
`cfg(test)`, so a unit test can reach the real store; multitop's own unit tests
cannot, because its mock keys off `cfg!(test)`). The self-test grew a case that
fails without the src sweep.

Re-run the probe after any change to how the vault decides to use the keychain.

### Where it has never been

| Area | Files | What that has already cost |
|------|-------|----------------------------|
| Rendering | `ui.rs`'s modal composition and `draw_no_matches` | A footer clipped at 80 and 96 columns, and a modal clipping its own footer -- found by rendering a frame and looking at it, not by any test. `refit.rs` and `ansi.rs` were read in the twenty-first pass, `layout.rs` and the draw path in the twenty-second and twenty-third. |

**Persistence came off this table on 2026-08-04**, and the fact that it was still
on it after being reviewed is a finding in its own right (nineteenth pass).
Round C's ninth and tenth passes read `config.rs` and `state.rs` and found three
defects there -- a zero-line history setting that silently swallowed the Upgrade
pane, a corrupt `state.toml` read as a first run and then overwritten, and a
resumed upgrade that never recorded that it had started. One of the three
questions that entry posed is now answered; the other two are still open and are
listed with the next round below.

`ui.rs` has been read only where the twelfth pass needed it (the two confirm
rows). `refit.rs` and `ansi.rs` have not been opened by any pass.

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

The streak stops at eight. Round C's forty-six findings were all found by review,
before anyone hit them -- and the reason is the same rule read the other way:
the round's first act was to build the seam the loop had never had. The area
with the worst detection record was the area with no harness at all. Where the
next round has no harness, build it first.

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

#### Round B -- the UX panel, 2026-08-03

**Scope:** the whole client UI, through four expert lenses chosen for bars that
actually apply -- structure (Rams), legibility at real rendered size (Kare),
frame loop and redraw cost (Carmack), operator experience (Hashimoto). Blow and
McMillen were deliberately not run: their bars are game character and depth, and
running a persona whose bar does not apply is how panels manufacture padding.

**What made it possible:** `crates/multitop/tests/render_views.rs`, built for
this round. 22 screens x 4 terminal sizes, rendered to `target/views/` as text,
by one command. Before it, reviewing anything visual meant launching the app
against real hosts and photographing a terminal -- which is why every visual
defect so far was found by the user. It is also a gate: every screen at every
size must render without panicking.

It caught its own bug first. The initial version fed the agent's renderer the
*terminal* size instead of the per-panel size `ui::agent_dims` returns, so every
frame was drawn for a pane bigger than it gets and the tops were silently cut. A
harness that misrepresents the product is worse than none.

**Found:** 22 findings, all verified against source or a rendered frame before
being written down. They were recorded by class rather than by persona; all eight classes are
fixed and the detail is in git history (2026-08-03, 2026-08-04).

**The result worth keeping:** four independent bars kept landing on the same few
root causes.

- `lines[0]` having two owners was found by Kare (a one-line body is eaten, so
  the connecting state is a blank box) and by Carmack (the scroll badge is built
  and destroyed on the same frame, so it has never been on screen). Neither
  could see the other's symptom.
- The keybar overflow was found independently by Rams and Kare, and both
  singled out the same `{:<11}` pad as the insult inside it.
- The confirmation modal's aggregate "all servers" was condemned by Rams (it
  says less than the view it covers) and by Hashimoto (it hides the hosts a
  filter removed from the screen) -- from opposite directions, about the same
  five words.

Convergence from unrelated bars is much harder to dismiss than one loud opinion.

**Two findings landed on this session's own work,** which is the argument for
running the panel on work you are pleased with: the settings row's fixed 75
columns survived the `clip` fix applied hours earlier -- instance cured, class
alive, by the author of the rule against exactly that -- and `render_views.rs`
was itself wrong on its first run.

#### Round C -- terminal and process lifecycle, 2026-08-04

**Scope:** `run.rs`'s event loop, signal handling, terminal restoration on
every exit path, and the child processes behind every SSH session
(`ssh.rs`, `tasks.rs`). The four questions the round was set are answered
below, findings and negative results together.

**What made it possible, and it is the finding behind the findings:** the loop
had no seam. It took the real terminal and read the real stdin, so nothing in
it could be exercised by anything but a person watching a terminal -- which is
why every defect it ever shipped was found by the user. `event_loop` is now
generic over the backend and over the event stream, and the agent render size
is published through a channel the caller owns, so a test can drive scripted
key and resize events against a `TestBackend` and read what the agents were
told. `tests/event_loop_e2e.rs` is that harness. Six of the seven findings
below are pinned by tests that were run red against the old code first.

**Found (7):**

- *The agent render size was derived from a panel count captured before the
  first frame.* Editing the server list changes the grid -- three panels are two
  columns, two are one -- so it changes the size every pane gets, and every
  agent kept rendering for the old one. Worse, the next resize recomputed from
  the same stale count and put the wrong value back, so resizing the window
  made the display worse. Class: **a value re-derived from a snapshot of one of
  its inputs**. Fixed by diffing the whole signature (`DimsInputs { size,
  panels }`) in one place, so there is no per-input hook left to miss an input.
- *`SIGTERM` and `SIGHUP` were fatal.* At their default disposition the process
  ends without running `TerminalGuard`, leaving raw mode and the alternate
  screen up, and without printing which upgrades it had just killed. `SIGHUP`
  arrives exactly when the terminal is going away, which is when a `dpkg`
  transaction most needs saying out loud. Both are now caught and become an
  ordinary quit.
- *Mouse reporting survived every panic.* The release profile aborts, so `Drop`
  does not run and `ratatui`'s panic hook is the only cover -- and it restores
  raw mode and the alternate screen, which is not what this program turned on.
  The shell that inherited the terminal then printed an escape sequence every
  time the pointer crossed the window. Class: **the same list of terminal modes
  written in four places, one of which was a library that had never been told**.
  Fixed with one enter/leave pair, our own panic hook, and a test that asserts
  every mode turned on is turned off again -- it catches five modes, not the one.
- *Children were in multitop's process group, holding its controlling
  terminal.* `ssh` opens `/dev/tty` on its own account for an unknown host key
  or a passphrase, whatever its stdin is connected to; so does a local `sudo`.
  In the foreground group that succeeds: the question is drawn over the frame
  and its answer is taken out of the keystrokes the event loop is reading, so
  the panel sits on `connecting...` while the display comes apart. Every child
  now gets its own process group, and `BatchMode=yes` stops `ssh` reaching for
  the terminal at all -- the same situation is now one legible line in the panel.
- *A view switch during an upgrade dropped the upgrade's handle.* The two
  shared one slot with a flag marking which a switch may not abort. The flag was
  obeyed and the handle was lost anyway, because `replace` hands back what was
  there and the "do not abort" branch let it fall. Nothing tracked the upgrade
  after that, so `abort_all` could not reach it on quit: the one thing the quit
  confirmation promises to stop was the one thing it could not. Upgrades now
  have their own slot, which makes the mistake unrepresentable.
- *A terminal that failed mid-frame skipped the killed-upgrade notice.* It sat
  behind a `?` on the loop's result, so the one exit that kills upgrades
  *without the user asking* was the one exit that said nothing about it. The
  loop now returns a `LoopOutcome` carrying both.
- *stderr was read only until stdout closed.* The two pipes close together when
  the child exits, so whichever branch `select!` polled first decided whether
  the contents of the stderr pipe were read or thrown away -- and stderr is where
  the reason lives: apt's actual complaint, the sudo-help shapes, the held-lock
  sentinel. A run that failed for a nameable reason reported "exited 1" about
  half the time. This is a surviving sibling of class H.

**Checked and already correct (3):** `SIGTTIN`/`SIGTTOU`/`SIGCONT` handling and
the terminal rebuild on resume (a resize that lands while stopped is now acted
on, which it was not); message-epoch guarding after a server edit -- an upgrade
task that outlives the swap has an older `gen` than any new panel, so it cannot
paint one machine's output under another's name; `Tasks::fit_to` aborting the
monitors and tasks of servers that were removed.

**Second pass over the same area, same day. Not dry -- three more (10 in
total).** Which is the whole argument for the bar: the first pass was thorough,
believed itself finished, and was wrong.

- *Editing the server list left running upgrades alive with nothing able to
  report on them.* `replace_panels` moves every generation, so no message from
  a task started against the old list is ever accepted again. The run carried on
  against the remote, invisible -- and was then killed without a word when the
  app quit. Now the run is stopped where the edit happens, the notice names the
  host and the lock file it may have left, and the removal confirmation warns
  *before* the key that does it, which is the standing rule for anything that
  interrupts a package transaction.
- *The reconnect backoff reset on connect rather than on progress.* Connecting
  is not progress: a host that accepts the connection and then fails -- a login
  banner where the protocol should be, an agent version mismatch whose upload
  keeps failing -- reset the count every round and was retried at the shortest
  interval forever. One `ssh` process every two seconds, indefinitely, and in
  the mismatch case a multi-megabyte upload with it. The failing sequence was
  `[2, 2, 2, 2, 2]` where it should be `[2, 5, 10, 20, 20]`.
- *The session handshake read four bytes with `read`, not `read_exact`.* A pipe
  may hand back fewer bytes than asked for; a magic header split across two
  reads was then compared against a buffer holding one or two, the agent's own
  framing was mistaken for a text banner, the rest of the line was consumed
  looking for a newline, and every packet after it read from the wrong offset.
  The panel said `invalid magic header` and reconnected, against a host that
  was working. Found while reviewing the lifecycle, but it belongs to the
  transport -- so that area is now partially reviewed rather than untouched.
  `stream::read_handshake` is split out so a reader that dribbles one byte at a
  time can be pointed at it, which is the only way to see this on purpose.

**Third pass: the round's own output, plus the framing it had only brushed
against. Still not dry -- one more (11 in total).**

- *A packet that arrived intact and could not be read was reported as a closed
  connection.* `decode_packet` returns `None` for a mode byte this build does
  not know; `next_packet` handed that straight back as `Ok(None)`, which every
  caller turns into `Connection to <host> closed`. An agent speaking a dialect
  this client cannot read is not a network problem, and saying "closed" about a
  host that is up and talking sends the operator to look at the wrong thing.
  The reason now goes into the same bounded buffer the stderr lines use, so it
  reaches the panel by the path that already exists. Class H again, and the
  third sibling of it this round.

**Changed, but not a defect found -- worth separating.** The first pass replaced
a propagating `terminal.size()?` with `unwrap_or_default()` in the resume and
server-edit branches, which would treat a terminal that cannot report its size
as a terminal of size zero and publish the minimum render size to every agent.
Reading `ratatui` rather than assuming: `draw` calls `autoresize`, which calls
`size()?`, so a failing size fails the draw first and the loop now ends on
that. The lie was never reachable. It is gone anyway -- keeping the last known
size costs nothing -- but it is recorded here as a correction to this round's
own work, not as a defect the round found. Counting it as a find would inflate
the number, which is the one thing a review log must not do.

**Also checked and correct (2):** the panel index used by
`password_actions::apply` cannot outrun the task vectors -- the `Save` follow-up
after a server edit is bounded by `servers.get(panel)`, and a freshly added
panel is never in `Upgrade` mode, so the resume branch it would need is
unreachable; and a panic inside a spawned task, which would restore the
terminal under a still-running app, cannot do that in the shipping profile
because `panic = "abort"` ends the process instead.

**Fourth pass: how the loop decides which pane a click or a key means. Two
more (13 in total), both the same class.**

The class is the one the first pass named and did not finish: **two places
deriving the layout from different counts.** The first pass caught it between
the grid and the agents; it is also between the grid and every way of pointing
at a pane.

- *A click was hit-tested against a grid that was not on screen.* `ui::draw`
  splits by `filtered_indices().len()`; the hit test split by `panels.len()`
  and then answered with an index into the *unfiltered* list. With `/db`
  showing one pane, clicking it selected some other host, and scrolling
  scrolled that host instead. It also answered "panel 0" for a click that
  matched no pane at all, so clicking the keybar -- the one row that invites
  clicking -- moved the selection.
- *The number keys counted entries in the config, not panes on screen.* Same
  shape: `2` under a filter selected a host that was not drawn, and every view
  key after it acted on that host invisibly. Out of range now does nothing,
  which is the answer a click on no pane gets: the two ways of choosing a pane
  agree.

**Checked and correct (2):** `f`, `d` and the Upgrade view still switch *every*
panel's mode rather than only the visible ones, and that is right -- the view
is global, the destructive *run* is what class F scoped to the filter, and a
per-pane view mode would leave a mixed grid the moment the filter cleared. The
"no host matches" screen's `N configured` really does mean the configured
count, not the visible one.

**Fifth pass: message handling under epoch churn, and the scroll arithmetic the
key and mouse paths feed. Three more (16 in total).**

- *The arm carrying the statistics consulted no guard at all.* `Msg::Packet`'s
  `Monitor` case computed `accepts` and then ignored it, so a packet from a
  task bound to the previous panel list painted whichever host had moved into
  its slot -- **the exact defect `replace_panels` bumps the epoch to prevent,
  described in that function's own doc comment, reachable through the one arm
  that never consulted it.** It could not have consulted `gen` either: monitor
  tasks are long-lived and stamp every packet `gen: 0`, so once an edit moves
  the generations off zero a `gen` check would reject live stats forever. The
  packet carries the epoch now and the check is made once, before any arm, so
  no arm can be written without one.
- *`Home` scrolled one line the wrong way.* It subtracted one from the offset,
  and the offset counts lines scrolled *back* -- so the key advertised as "top"
  moved one line towards the newest. It is the pane's oldest line now, the same
  bound `scroll_up` clamps to.
- *`End` reset every pane in the grid.* It called the whole-grid reset a view
  switch wants, so returning one pane to the bottom silently threw away where
  the user had scrolled all the others to. Its siblings -- `Home`, the arrows,
  `PageUp`/`PageDown` -- all act on the selected pane alone.

The last two are the fourth pass's class again, one level down: a key acting on
a different set than the keys beside it.

**Sixth pass: the remaining `Msg` arms and the pane windowing. One more (17 in
total), and it is the round's class for the third time.**

- *The scroll offset was bounded in two places by different rules.*
  `App::scroll_up` clamps to `pane_len - 1` because it cannot know a pane's
  height; the view can only go back `pane_len - height`. So the stored offset
  could sit a whole pane-height past anything the view could use, and the next
  dozen presses the other way moved nothing on screen -- a key that reads as
  dead, which is the same complaint the rule about swallowed clicks exists for.
  The fifth pass's `Home` fix made it worse by driving the offset straight to
  the looser bound.

  Fixed where the bound is actually known: `ui::draw` takes `&mut App` and
  writes the offset it used back, so what is stored is always what was shown.
  One clamp, in the one place with the height. **This is the same class as the
  first pass's stale panel count and the fourth pass's hit test** -- one
  quantity, two places, two rules -- for the third time in one round, which is
  the argument for treating it as the round's finding rather than three.

**Checked and correct (3):** `AuxLine`, `AuxBegin` and `AuxDone` are all
guarded, and their `upgrade_gen` path cannot collide with a rebuilt panel --
fresh panels carry `upgrade_gen: 0` and a state that is not `STARTED`, and the
generations only move upward.

**Seventh pass: re-covering ground the earlier passes walked, plus the two CI
gates none of them had run. One more (18 in total).**

- *A local panel's missing agent binary was reported as "ssh command not
  found".* A local panel never runs `ssh` -- it spawns the agent directly -- but
  `connect` mapped every `NotFound` from either spawn path to the `ssh`
  message, sending the operator to check an `ssh` that was installed and
  working. Found by running a gate rather than reading code: the `#[ignore]`d
  local-agent tests fail in a sandbox with no resolvable agent binary, and the
  message they failed with named the wrong program. Class H, fourth sibling.

**What running the gates was actually worth, and it is the finding about the
review rather than the code.** Six passes reported "all gates clean". That
covered `cargo test`, `clippy -D warnings`, `fmt` and the four `tools/*.py`
checks -- and *not* the two CI steps nobody had run:

- `cargo test --workspace -- --ignored` cannot pass in this sandbox: the local
  agent tests need a `multitop-agent` the test binary's own directory can
  reach. Isolated against `be5b83a` first -- it fails identically there, so it
  is the environment, not the round. It was still worth running: it is what
  surfaced the finding above.
- **Corrected by the eighteenth pass: neither of these is a CI step.** Both
  were read out of a stray `ci.yml` at the repository root, which GitHub never
  reads and which does not parse. The real workflow runs neither. Left below as
  written because the *measurements* stand and the lesson about a stale
  `target/` is real -- only the words "CI step" were wrong.
- `cargo llvm-cov --workspace --fail-under-lines 80` **cannot be trusted
  against a stale `target/`**. Run in place it reported 54% and listed
  `multitop/src/sparkline.rs`, a file deleted on 2026-08-04 -- 28,279 regions
  against a clean tree's 19,605. Measured properly, in one fresh worktree per
  commit: `be5b83a` 73.85% lines, this round's HEAD 76.73%. So the round raised
  coverage by three points, and both numbers sit under the gate here because
  this sandbox cannot run the paths that need a real agent binary. **Measure
  coverage in a fresh worktree or not at all.**

**Eighth pass: the composite Configuration actions, the stream boundary the
earlier passes only brushed, and the reconnect loop's repair path. Four more
(22 in total). Every one of them class H.**

- *A server edit that could not be written was reported as a saved password.*
  `ApplyServerEdit` is two operations -- write the server list, then store the
  password typed in the same editor -- and both report through the panel's one
  `notice` line. The second assigned it unconditionally, so when `save_servers`
  failed, "Could not save server configuration: ..." was replaced by "Password
  saved securely in system credential store." before anything drew it. The edit
  was discarded and the screen said success. The same overwrite erased, on the
  path that *succeeds*, the warning naming an upgrade the edit had just
  interrupted and the lock file it may have left -- the one thing this panel is
  required to say out loud. `ImportSshHosts` was the second live instance:
  "Imported 3 hosts" was printed whether or not the file was written.
  Fixed at the class: `write_servers` is the one place that decides whether a
  list write landed and returns the answer, so a follow-up cannot run over a
  failure; and a composite action's second half is joined to the first rather
  than assigned over it.
- *A remote that printed a banner instead of running the agent was reported as
  a closed connection -- or as nothing at all.* `read_handshake` reads the
  offending line to classify it and then **dropped it**, which made its own
  doc comment ("left to the packet reader to fail on, so the text lands in the
  panel") false: the reader failed on the next eight bytes with `invalid magic
  header`, and all three readers of the stream pattern-match
  `while let Ok(Some(payload))`, so the `Err` ended their loop and was dropped
  on the floor. The monitor then said `Connection to <host> closed` about a host
  that was up and talking; fetch and docker said nothing whatsoever. A shell
  profile that prints on a non-interactive session is the ordinary way in.
  The text is carried now, and the reason goes into `errbuf` -- the same bounded
  buffer the stderr lines use, which all three callers already drain -- so it
  arrives by the path that exists rather than by a fourth one each caller has to
  remember. Class H, fifth sibling.

  Folded into the same fix, because it is the round's *other* class one line
  away: that buffer's bound was written twice with two different rules (`>` on
  the stderr path, `>=` on the reason path), so it held nine lines or eight
  depending on which kind of line arrived last. One `note` helper now.

- *An agent that could not be replaced never said so.* A version mismatch ends
  the session on purpose and tries to upload a new agent. Both failure paths
  were swallowed by one `if ... .is_ok()`: `probe_remote_arch` returning `None`,
  and `upload_agent` returning `Err` -- whose message ("No aarch64 agent was
  built into this binary. Rebuild with ./build.sh") is written to be acted on
  and was the message being thrown away. The panel said "replacing..." and then
  nothing, once per backoff interval, for the rest of the session. A *local*
  panel was worse: it has no SSH session to replace anything over, so the repair
  sent `ssh` at `localhost:0` and discarded that failure too. `replace_agent`
  returns a `Result` and the caller has one send, so no arm can be silent.
- *A session this client broke on purpose was reported as the host closing it.*
  The version-mismatch `break` fell straight into the "connection ended" line,
  so `Connection to <host> closed` was printed between "replacing..." and
  "agent replaced" -- the only line in that sequence that is not true, about a
  host that was up and had just been talking.

**The counts so far are 7, 3, 1, 2, 3, 1, 1, 4, 1, 2, 3, 2, 2, 2, 1, 1, 1, 2, 2, 2, 0, 1, 1, 1, 0, 1** -- and they are not
converging. Every pass so far went back up the moment it opened a part of the
loop the earlier ones had not looked at, which is the argument against reading
a falling count as progress toward zero. What is running out is *unexamined
surface*, not defects; the round ends when a pass covers the same ground and
finds nothing, not when the number gets small.

**Checked and correct (2):** the vault message arms under epoch churn --
`VaultUnlocked` forcing `ShowUpgradeModal` is right because `begin_vault_unlock`
has exactly one caller, the `u` path, so there is no way to unlock the vault
that is not already on the way to the confirmation; and `Msg::Status` is
`gen`-guarded rather than epoch-guarded, which is sufficient because only the
short-lived fetch and docker tasks send it and both are stamped with the panel
generation a `replace_panels` moves.

**What the eighth pass says about where to look, and it is the useful part.**
All four findings sit on a *seam between two subsystems* -- Configuration
handing to the credential store, the handshake handing to the packet reader,
the reconnect loop handing to the agent uploader -- rather than inside either
side. Class H survives at seams because each side is locally correct:
`save_servers` returned its error honestly, `read_handshake` classified the line
correctly, `upload_agent` composed a message written to be acted on. Every one
of them was lost in the handover, and three of the four were lost to the same
two Rust shapes: `if ... .is_ok()` with no else, and `while let Ok(Some(x))`,
which are both a discarded `Err` spelled so that nothing looks discarded.
Grepping for those two shapes is the cheapest way to find the next one.

**And it found one immediately, in this pass's own fix.** Sweeping the workspace
for `while let Ok(Some(` and `if ... .is_ok()` turned up three surviving
siblings inside `next_packet`: the framing failure had been given a reason to
report, and the two short header reads and the payload read had not, so a plain
I/O error mid-stream was still silent. That is rule 6 read against this
session -- an instance patch that leaves the class alive. The noting moved out
of `framing_lost` and into `next_packet` itself, which now records *any* `Err`
on one path, so the next `return Err` added below it cannot be silent.

#### Ninth pass -- the panel's own buffers. One more (23 in total)

- *`upgrade_history_lines = 0` silently swallowed the entire Upgrade pane.* The
  pane is composed from the same ring as the history, and `RingLines::push` on a
  zero-capacity ring is a no-op that returns quietly -- so obeying the setting
  meant the pane showed nothing for the whole of a run, and every line this
  round has been arranging to reach the operator went with it: the completion
  note, the sudo-refused warning, the held-lock warning naming the file to
  remove. Nothing said why. `Panel::note` in Upgrade mode went the same way.

  **The state was already known and already fixed once.** `RingLines::from` set
  the capacity to its fixture's length, so an empty fixture built a
  zero-capacity ring; that was closed, and its doc comment describes this exact
  failure. The config file was the *other door into the same state*, and nobody
  had gone looking for it -- rule 6b, unperformed, on a fix that had already
  named its own class. There is a floor in `config::load` now, applied in the
  one place that reads the value, and the substitution is said out loud in the
  panels rather than made behind the user's back: the file still asks for
  something it will not get, and only the user can put that right.

**Checked and correct (1):** `RingLines`'s own arithmetic under wrap --
`get`, `last` and `slice` are all safe at length zero (`get` short-circuits
before the modulo, and `slice` clamps `start` before subtracting), so the floor
is about what the pane can show, not about a panic.

#### Tenth pass -- persistence, from the loop's side. Two more (25 in total)

Both on the same seam, and the shape is now familiar enough to name in advance:
**the writer was careful and the reader was not.**

- *A run started by "save the password and resume" never recorded that it
  started.* There are two ways to begin an upgrade and they disagreed. The
  confirmation modal writes each host's `started_at` with no `finished_at`,
  which is exactly how an interrupted run is detected next time. The resume
  path set only the *global* `upgrade_started_at` and hand-rolled its own
  `AppState` write, cloning `host_updates` through unchanged -- so a resumed run
  cut short by a crash or a power loss left that host's record showing whatever
  was there before it: a previous success, reported afterwards as `Ok`, or
  nothing at all, reported as "never upgraded". The one record the feature is
  built on was the one record that path did not write. `App::mark_upgrades_started`
  is the single place now, and the duplicated `AppState` construction went with
  it -- the round's *other* class, one quantity assembled in two places by
  different rules, for the fourth time.
- *A corrupt `state.toml` read as a first run, and was then overwritten.*
  `load_state` returned `AppState::default()` for any parse failure, which is
  the same value a fresh install produces -- so the two were indistinguishable
  on screen, and the next `persist_state` wrote a new file straight over the old
  one. The history was not merely ignored, it was destroyed. `write_atomic`
  exists precisely so an interrupted write cannot lose `upgrade_started_at`;
  the loader threw it away anyway. It is moved to `state.toml.unreadable` now,
  out of the way of the next write, and said out loud. A read failure that is
  not `NotFound` -- a permission change, an I/O error -- was the same lie and
  gets the same treatment; a genuinely missing file stays silent, because a
  notice on every fresh install trains the user to ignore the line that matters.

#### Eleventh pass -- the Upgrade header, and the harness that was hiding it. Two more (27 in total)

- *A run in flight was reported as an interrupted one, in the line under the one
  saying it was running.* **A run in flight has exactly the shape of an
  interrupted run** -- `started_at` set, `finished_at` absent -- because that is
  the shape written when it starts, deliberately, so a crash leaves it behind.
  `badge`, `badge_color` and `next_action` each checked `running` before
  consulting the record. `last_run_text` did not. So during a genuine upgrade
  the header read:

  ```text
  Status    running
  Last run  just now - interrupted
            -> running - do not quit
  ```

  Three lines, two of them true. "interrupted" is the word that sends an
  operator to go and check a host that is perfectly fine. Fixed by folding the
  two views of one fact into `Status::state()`, so the record cannot be read
  without first learning whether it describes the present -- a fifth consumer
  cannot repeat this.

- *And the reason it survived ten passes is that **both** instruments modelled a
  state the app cannot produce.* The unit test for a running host used
  `HostUpdate::default()` -- no `started_at` -- and `render_views.rs`'s
  `upgrade-running` screen set `upgrade_state` by hand and left `host_updates`
  empty, so the rendered frame said "Last run never" for a host that was
  running. The harness built specifically to catch visual defects was showing a
  frame the product never draws. It is driven through `confirm_upgrade` now, the
  real entry, and the frame at 80x24 reads `Last run  just now - in progress`.
  Same lesson this file learned on its own first run, when it fed the agent the
  terminal size instead of the per-panel size: **a harness that misrepresents
  the product is worse than none.**

- *`persist_state` discarded its own failure.* Found by the harness fix, which
  made the render path write state for the first time. `save_state` goes to some
  trouble to be atomic precisely because these records are what make an
  interrupted run detectable -- and a write that never happened defeats that as
  completely as a torn one. A read-only or full disk cost the user their upgrade
  history with nothing on screen to say so: the same shape as the corrupt-file
  case the loader had one pass earlier, on the other side of the same seam.

#### Twelfth pass -- the Configuration keys, and every destructive confirmation. Two more (29 in total)

One class, two live instances, and the fix for it was already in this round.

The rule was written during Round A's follow-up work: **a destructive
confirmation acts on the keys it names and nothing else**, because "`Enter` is
what an operator hits to dismiss something they have not read". It was applied
to the quit row. Neither of the other two confirmations got it.

- *The server-removal question accepted `Enter`, which it does not offer.* The
  prompt reads `Remove <host>? ... [y] confirm  [Esc] cancel`. `Enter` confirmed
  as well -- and `Enter` is **this panel's own key for opening a row to edit
  it**. So `d` then `Enter` (press `d` to read the question, then reach for the
  key you use to work on a row) removed the host and, through `write_servers`,
  aborted any upgrade running on it: a `dpkg` transaction interrupted on a real
  machine by two keystrokes that never meant to.
- *The upgrade confirmation accepted `y`, `Y` and `Enter`, and names none of
  them.* The row reads `[U] go  [Esc] cancel`. Three unnamed keys that start
  `apt upgrade` on every visible host. Extra *cancel* keys stay, deliberately --
  a stray key that cancels can only ever be the safe answer.

**What the gate could not see, and it is the useful part.**
`tools/check_key_hints.py` enforces *named -> bound*: a key a string tells the
operator to press must be a live match arm. Both of these are the converse --
*bound but not named* -- and the gate is blind to it by construction. It cannot
simply be inverted, because legitimate aliases (`j`/`k`, `q`/`Q`) are bound and
named nowhere. The rule only bites on destructive confirmations, so it is
enforced by a test per confirmation instead: each renders its own row, asserts
what it offers, and then asserts that the keys it does not offer do nothing.
The quit row already had one; the other two have one now.

#### Thirteenth pass -- `upload_agent`'s framing, and `config_ui`. Two more (31 in total)

Both in the one place the round had left untouched, and the second is the
worst-consequence finding of the whole round.

- *A refused upload was reported as `Broken pipe`.* The agent is several
  megabytes. If the remote had already given up -- `mkdir` refused on a
  read-only home, no space in `~/.cache`, a quota -- `write_all(...)?` returned
  the local pipe error and never reached `wait_with_output`, so the child's
  stderr, where the actual reason is, was discarded. The operator was told
  "upload: Broken pipe" about a disk that was full. Same class as the eighth
  pass's stderr finding in `spawn_upgrade`: stderr is where the reason lives and
  the pipe closing is the *symptom* of it. The write failure is remembered
  rather than returned, the child is reaped either way, and `upload_failure`
  decides -- the remote's complaint wins whenever it made one.
- ***A truncated agent could be installed, and the install reported as a
  success.*** `cat` cannot tell a finished stream from an interrupted one: both
  end in EOF. A connection that dropped partway through left `cat` succeeding on
  a short file, then `chmod`, then `mv`, and the whole command **exiting 0 with
  a truncated binary in place as the agent**. The local side said the install
  worked; the next connection failed to exec it, and the panel blamed the
  architecture or the bootstrap for a file this program had put there itself.
  The remote cannot detect this on its own -- the expected length is the one
  thing only the local side knows -- so it is passed in and checked before the
  `mv`, the staging file is removed rather than left to accumulate, and the
  command exits non-zero. Pinned by a test that runs the real script under `sh`
  against a short stream and asserts the agent did **not** land.

#### Fourteenth pass -- the bootstrap script's quoting and the `-tt` pty path. Two more (33 in total)

- *The SSH control socket lived in `/tmp`.* Every session multitop opens is
  multiplexed over it -- including the upgrade, whose stdin carries the host's
  sudo password. The path was `/tmp/multitop-ssh-%u-%C`, and both halves are
  predictable from outside: `%u` is the local username, and `%C` hashes the
  user, host and port, which `ssh` itself puts in argv where `/proc` publishes
  them to every account on the machine. The sticky bit stops another user
  *replacing* that socket; nothing stops them **creating** it first, and
  `ControlMaster=auto` joins a socket that is already there rather than becoming
  the master.

  **This is the same threat model, on the same kind of shared machine, that
  moved the sudo password off the command line** -- "argv is not secret, and
  `/proc/<pid>/cmdline` is world-readable". Taking the password out of argv and
  then handing the whole session to a socket anyone could have created is
  defending one half of a path. It is `~/.ssh/multitop-%C` now, reachable only
  by its owner; if `~/.ssh` does not exist the bind fails and `auto` degrades to
  an unmultiplexed connection, which is the right way round. Pinned by a test
  that rejects any world-writable prefix, plus one for the `sockaddr_un` length
  limit, since `%C` alone expands to forty characters.

- *The held-lock sentinel was checked on the one stream it cannot arrive on.*
  `SUDO_FAILED` was scanned on stdout and `LOCK_HELD` on stderr, which describes
  the **local** shape, where the two pipes stay separate. A remote upgrade runs
  under `ssh -tt`, and **a pty has one stream**: sshd merges the remote's stderr
  into it, so `echo "__multitop_lock_held__" >&2` arrives on the local client's
  *stdout*. Two consequences, on every remote host. The sentinel never fired, so
  detection fell back to the distinct exit code alone -- which is precisely what
  the sentinel exists to survive without, in a login shell noisy enough to lose
  a status. And the stdout branch had no reason to skip it, so the raw
  `__multitop_lock_held__` was printed into the operator's upgrade log as if it
  were output. One `marker()` scanner for both sentinels on both streams now,
  under the same rule `is_sudo_help` already carries: two streams disagreeing
  about what counts is how one of them stops recognising it.

#### Fifteenth pass -- this session's own fourteen passes. One more (34 in total)

**Scope, and it is deliberately not the codebase.** Every named area of the
round now has a pass behind it, so the least-reviewed code in the tree is the
code the last seven passes *wrote*. Round B's rule, read the other way round:
run the panel on the work you are pleased with.

- *The failed-state-write notice was erased by the path that triggers it.*
  `confirm_upgrade` marked each host started -- which is where the write happens
  and where the notice is pushed -- and *then* called `run_upgrade`, which
  clears each started panel's `last_upgrade` ring before streaming into it. The
  panels are already in Upgrade mode by then, so `Panel::note` put the notice in
  exactly the buffer the next line empties. The resume path in
  `password_actions` had the same shape, with `replace_with` doing the clearing.
  The eleventh pass's fix therefore worked everywhere except where it mattered.

  **And its test passed the whole time**, because it drove
  `mark_upgrades_started` directly, where nothing clears afterwards. The defect
  lives in the *order of the real path*, which is the only thing worth pinning --
  the same lesson as the eleventh pass's `HostUpdate::default()` running host,
  one week of code later and by the same hand.

#### Sixteenth pass -- the same diffs, read again. One more (35 in total)

- *Every notice the app writes was erased by the next agent frame.*
  `Panel::note`'s own doc comment says it exists so a message is never "built,
  stored, and never drawn". In every non-Upgrade mode it wrote into `view` --
  and `view` is **derived state**: `show_last_frame` and all three arms of
  `Msg::Packet` rebuild it from the frame. The first frame arrives about a
  second after startup, which is exactly when every startup notice is written:
  the plaintext-password migration (which predates this session), a clamped
  `upgrade_history_lines`, an unreadable `state.toml`, a failed state write.
  Each appeared for one second and was gone. The doc comment's own failure,
  reached by the other door -- not written to the wrong buffer, written to one
  that is rebuilt.

  Notices live in `Panel::notes` now, outside derived state, and `show_frame` is
  the single place that replaces `view` with a frame. It was four places, each
  assigning `view` directly, so re-appending in one would have been dropped by
  the other three -- one quantity, four places, which is the shape this round
  has found more often than any other.

- **The harness was drawing a frame the app does not draw, again.**
  `render_views.rs` assigned `p.view` directly in `with_stats`, and no screen
  had ever rendered a pane carrying a notice. Both closed: `with_stats` goes
  through `show_frame`, and `monitor-with-notice` renders one at all four sizes.
  Confirmed by reading the frame rather than the assertion -- the notice appears
  in both panes at 80x24.

#### Twenty-sixth pass -- `ui::draw`'s composition. One more (46 in total)

- *A status line was eaten by the banner on the frame it arrived.* `ui::draw`
  replaces `lines[0]` with the host banner every frame -- which is why
  `Panel::new` reserves row 0 with a placeholder, after Round B found that "a
  one-line body is eaten, so the connecting state is a blank box".

  `Msg::Status` assigned `vec![text]`. One line, which *is* row 0. So every
  status a fetch or docker task sends -- "installing agent...", and the error
  path's `error_line(e)` carrying `ssh command not found` or a refused key --
  was written into the pane and destroyed before anything drew it. Driven
  through `ui::draw` at 60x8, the screen was the banner and nothing else.
  `Msg::AuxBegin` had the same shape: a `Some(header)` is one line, and a
  `None` left the body empty, which `draw` skips entirely.

  **Round B's fix reached `Panel::new` and `show_last_frame` and stopped there.**
  `Panel::show_body` reserves the row in one place now, and the two message arms
  that build a body from scratch go through it. Rendered agent frames keep
  `show_frame` -- they carry their own row 0 and must not get a second.

  Three tests asserted `view` equalled the text exactly, which encoded the shape
  that ate the body; they assert presence now.

#### Twenty-fifth pass -- the vault modals, fixed. (45 in total; no new finding)

The twenty-fourth measured this and stopped, because wrapping alone converted
horizontal clipping into vertical loss and the honest answer was a design call.
Here it is: **a password prompt cannot become a keybar row -- it needs a field
-- so it does the other half of Kare's ruling instead. It sheds.**

The parts are ranked, exactly as `fit_row` ranks a keybar's chunks:

1. the explanation of what the password is for -- useful, never essential
2. the blank lines that space the block out -- decoration

The headline, the field, the error and the footer naming both keys are never
shed. An operator who cannot read what the password protects can still act; one
who cannot see `Esc` is stuck. The box height is derived from the wrapped
content at the width it will actually be drawn at, rather than the literal whose
own comment recorded it had already been bumped once for this.

At 40x12 the create prompt now sheds its explanation and keeps headline, field,
error and footer; at 80x24 everything is present and nothing is clipped. Pinned
by a test that drives `ui::draw` for both prompts, with and without an error, at
four sizes, and asserts the footer's `Enter`, `Esc` and `cancel` all reach the
screen.

**One process note worth keeping.** The first version of that test failed for a
reason that was not the code: `begin_vault_creation` refuses when another prompt
could be up, so the modal was never drawn and the assertion read as "the footer
lost Enter". The harness has to reach the state the product reaches -- the same
lesson as the eleventh pass's `HostUpdate::default()` and the sixteenth's
`upgrade-running` screen, for the third time.

#### Twenty-fourth pass -- modals and the no-matches screen. One found, **fixed by the twenty-fifth** (45 in total)

- ***The vault modals clip their own text, including the footer that names the
  way out.*** At 40x12:

  ```text
  │  Enter vault master password to unl│
  │  Press Enter to unlock, Esc to canc│
  ```

  This is the defect the detection record already lists as user-reported -- "a
  modal clipping its own footer". **Kare's ruling fixed it for the upgrade
  confirmation** by replacing the box with a keybar row, because "the box was 38
  cells wide at 40 columns and clipped its own cancel line to `Esc t`". The
  vault modals are still boxes and still clip. `settings-delete-confirm`, which
  wraps, shows the fixed shape one screen over.

  **Attempted and reverted, and the reason is the finding's real shape.** Adding
  `Wrap` fixed the unlock prompt outright -- full text, and moving the indent
  into the block's padding aligned the continuation lines. But it turned the
  create-vault modal's horizontal clipping into *vertical* loss: the box height
  is a literal (`12` when creating, `10` when unlocking) whose own comment
  records it was already bumped once because it "clipped the `Press Enter to
  create` footer off the bottom" -- a guessed line count, correct only at the
  width it was guessed for. Deriving the height from the wrapped content fixes
  that, and still does not fit: the create modal is nested *inside* the Settings
  box, so at twelve rows it has eight for nine lines of content.
  
  So the honest fix is not "wrap it" -- it is Kare's, again: **this box wants to
  stop being a box**, or the create-vault explanation has to become one short
  sentence. That is a design call on a screen the operator meets when they
  cannot get in, and it is not one to make and ship unvalidated at the end of a
  session. The work is reverted; the measurement stands.

**Checked and correct (1):** a filter query is user-typed and unbounded, and is
interpolated into both the no-matches headline and the keybar echo, neither of
which wraps. `draw_no_matches` exists because "the way out -- `Esc` -- is not
guessable from a blank terminal", so the question is whether a long enough query
can push that instruction off the screen. Driven through `ui::draw` at 1, 20,
60, 200 and 1000 characters across three terminal sizes: `Esc` survives every
one, because the instruction is its own short `Line`. Pinned.

#### Twenty-third pass -- the render harness used as a review instrument. One more (44 in total)

The roadmap has said since Round B that `render_views.rs` "has never been used
as the instrument of a review round". This pass used it as one: render every
screen at 40x12 and mechanically flag any line that fills the pane and ends
mid-word. Fifteen candidates, twelve of them data or complete sentences that
merely end on a word -- and one real signal, appearing in four screens.

- *Three of the five credential states overflowed the pane and lost their
  endings.* Measured rather than eyeballed: the pane gives **38 cells**, and the
  `Sudo` row costs `label(9) + 2 + text`. `Session` needed 40, `MissingNoVault`
  40, `VaultLocked` 42. They reached the operator as "password set for this
  sessio", "will prompt - no vault set" and "from the vault - unlocks on r".

  **The rule is stated in this file, and the lines beside it obey it.**
  `next_action`'s doc comment: "Kept under ~40 visible columns per line: with
  four panels the grid is two columns wide, and `ui::visible` hard-truncates
  rather than wrapping, so a longer sentence loses exactly the part that tells
  the user what to do." Its own lines run to 26 cells. The `Sudo` row, in the
  same header, built by the same function, was measured by nobody.

  Shortened to fit -- `Session` drops the word "password", which the row's own
  label already says -- and pinned by a test that measures **every** variant
  against the pane, so a sixth cannot be added over-width. That is the class
  fix: the previous state of affairs was three strings that happened to be
  short enough and two that happened not to be.

**What the sweep is worth keeping.** Twelve of fifteen flags were noise -- a
clipped `upgrade_cmd`, apt's own output, sentences ending on a word. The
signal-to-noise is poor enough that this belongs as a *review technique* rather
than a gate, but it found in one command something twenty-two passes of reading
had walked past.

#### Twenty-second pass -- `ui.rs`'s draw path and `layout.rs`. One more (43 in total)

- *Every notice this session added was hard-truncated mid-word.* Found by
  rendering the smallest pane and reading it, not by any assertion. At 40x12 --
  a four-panel grid, which is the ordinary case -- the sixteenth pass's own
  notice reached the operator as:

  ```text
  config: upgrade_history_lines = 0 woul
  ```

  **The rule was already written down, twice.** `upgrade_view::next_action`
  keeps its sentences "under ~40 visible columns ... `ui::visible`
  hard-truncates rather than wrapping, so a longer sentence loses exactly the
  part that tells the user what to do". `config_ui` learned the same thing when
  a delete confirmation lost its own `[Esc] cancel` at 40 columns, and grew
  `wrap_words` for it. The panel notices got neither: four of them, added in
  passes 9, 10, 11 and 16, every one a long sentence whose ending is the
  actionable half.

  Fixed by moving the notices out of `view` altogether. They live in
  `Panel::notes` and are appended by `ui::pane_lines` -- "the single entry point
  to what is in that pane", and the only place that knows the pane's width and
  can therefore wrap them. `wrap_words` moved to `layout` as the shared rule.
  That is also a better answer to the sixteenth pass's finding than the
  sixteenth pass's: `view` goes back to being purely derived state, so there is
  nothing for a frame to erase and no re-append for a fourth writer to forget.

  Five tests were reading `panel.view` for a notice. They read `ui::pane_lines`
  now, which `pane_lines`' own doc comment already told them to do: *"a test
  reading `panel.view` directly is reading a buffer the Upgrade pane does not
  draw."*

**Checked and correct (3):** `fit_row` shrugs off out-of-range shed indices
(`kept.get_mut`) and returns everything when nothing fits, as documented;
`share_width`'s surplus loop cannot spin, because a hungry cell always takes at
least one column from a finite pool; and `pane_window`'s pinning arithmetic is
saturating throughout, with `height = 1` correctly yielding no pinned row rather
than an empty pane.

**Noted, not reachable (1):** `share_width` indexes `flex_want[i]` over
`flex_min.len()`, so a caller passing a shorter `flex_want` panics. Its one
production caller passes matching literals. Left alone rather than guarded,
because a `debug_assert` that only fires in tests is not protection and a
silent `.get()` would hide the caller's mistake.

#### Twenty-first pass -- `refit.rs` and `ansi.rs`. **Nothing found (42 in total)**

The first pass in this round to end with no findings. It does **not** meet the
bar, and saying why is the point: the bar is a *full* round that produces
nothing, and this covered two files and a question, not the rendering area.
`ui.rs`'s draw path -- some nine hundred lines -- and `layout.rs` were not
opened. A partial pass coming back empty is what stopping too early looks like
from the inside, which is exactly the mistake this item exists to stop repeating.

What was asked, and answered clean. Every answer is now a test, so none of it
has to be re-derived:

- *What an unterminated escape sequence does* -- one of the two questions the
  roadmap set for this area. The CSI parameter loop runs off the end of the
  line, `final_byte` stays `None`, the sequence is discarded and the text before
  it is kept. No hang, no panic, nothing swallowed.
- *What a private-parameter sequence does.* `\x1b[?25l`, which any progress
  display emits, skips its `?`, reads `25`, terminates on `l`, and is dropped
  whole because it is not SGR. No literal reaches the pane.
- *What the embedded newline the control filter exempts does.* The filter drops
  `\r`, BEL and backspace "so they don't corrupt Ratatui's cursor/layout" and
  lets `\n` through, which reads like an oversight. Rendered into a real buffer
  rather than reasoned about: ratatui swallows it exactly as it swallows the
  `\r` the filter does drop -- text joined onto one row, row count unchanged.
  Harmless, and now pinned.
- *Whether any pane gets negative or wrapping arithmetic at extreme sizes* --
  the other question set for this area. `regions`, `agent_dims` and `visible`
  driven across every combination of width and height in `{0,1,2,3,5,10}` with
  0 to 8 panels and four scroll offsets: no pane escapes its screen, no agent is
  told to render into zero, no window exceeds its height. `render_views.rs`
  claims to be a gate for sizes "smaller than the content" and its smallest is
  40x12, which is small but is not the case that finds a subtraction.
- *`refit_header` rebuilds its label by filtering to fullwidth characters and
  spaces, dropping everything else* -- which would silently delete the `…` from
  a truncated wide banner. **Checked for reachability rather than reported:**
  `ui::draw` composes the banner over row 0 *after* `visible` has refitted, so
  the banner never passes through it. Not a finding.

**The keychain gate fired on this pass's own new test and was obeyed.** It
checks *reaching* per file and *diverting* per test -- asymmetric on purpose,
because the reaching call is usually one helper away and text matching cannot
follow a call graph. The test touches no credential; it holds the guard anyway.
Arguing with a structural gate is how the gate gets switched off, and this file
warns about that twice in its own comments.

#### Twentieth pass -- the two persistence questions. Two more (42 in total)

The nineteenth pass narrowed the persistence entry to two questions it had never
asked. This pass asked the second one -- *can any write path lose a key the user
set by hand* -- and it answered twice.

- *Cycling the theme stripped every comment from the config file.* `save_theme`
  parsed to `toml::Table` and re-serialised, which rebuilds the file from its
  values; comments and blank lines are not values. One press of `t` reduced a
  hand-maintained `config.toml` to a bare key list.

  **`save_banner_style` sits immediately below it and carries the doc comment
  describing exactly this defect as fixed** -- "this runs on a keystroke, which
  means one press of a display toggle was enough to strip a hand-written
  config". Two toggles, identical job, identical file, one fixed. The test file
  for this class, `config_preserves_comments_test.rs`, opens by saying *every*
  writer used to have it. `save_theme` was the one nobody re-checked, and it is
  the one bound to the key a user presses most idly.

- *Every config write could truncate the file.* `state.rs` grew `write_atomic`
  because a truncating write does not merely fail to record the new value, it
  destroys the old one -- and all four config writers kept `std::fs::write`.
  That is strictly the worse file to lose: `config.toml` holds the server list,
  is maintained by hand, and is written **on a keystroke**. The atomic writer is
  shared now. Pinned by the observable consequence rather than the mechanism: a
  write that cannot happen leaves the previous contents byte-for-byte and no
  scratch file behind.

**The first question, answered without a fix.** *What two multitops sharing one
config do:* both write `state.toml` through `write_atomic`, so neither ever sees
a torn file -- but each holds the whole `host_updates` map in memory and writes
all of it, so the later writer's map wins and the other instance's upgrade
history is silently dropped. `config.toml` has the same read-modify-write shape.
Not fixed: the honest repair is per-key merging or a lock file, both of which are
real design decisions rather than review findings, and two instances against one
config is not a documented use. **Recorded so the next person does not have to
rediscover it**, which is the whole reason this log exists.

#### Nineteenth pass -- an audit of this log's own claims. Two more (40 in total)

The eighteenth pass found two defects in the record rather than the code, so
this one checked the rest of the record against the tree. Most of it held: the
keychain probe's five call sites are exactly where it says, the gate does sweep
`crates/vault/src`, all six test-only baseline entries still exist, item 5's
description of `ui::draw` splitting by `shown.len()` while `refresh` is handed
`app.panels.len()` is still true line for line, and `sparkline.rs` was indeed
deleted in `6aaf8bb`. Two claims did not hold, and chasing the second one found
a defect in the code.

- *This log said persistence had never been reviewed, in the same document that
  records reviewing it.* The "Where it has never been" table still listed
  `config.rs` and `state.rs` after the ninth and tenth passes read both and
  found three defects there -- and the "next round" heading below still opened
  with "Round C is done", written when the round was seven passes old and never
  revisited. Anyone reading the summary would have planned a round that had
  already happened. Both corrected, with the two persistence questions that
  genuinely remain unanswered spelled out rather than left implied.

- ***`save_servers` merged two entries on one machine and destroyed one of
  them.*** Found by testing a claim rather than asserting it: the correction
  above needed a sentence about whether a write path can lose a hand-set key,
  and the sentence turned out to be wrong in the interesting direction. The
  writer reuses an existing `[[servers]]` table so the comment above it survives
  -- and found that table **by host alone**. This project is explicit everywhere
  else that two entries on one machine with different users or ports are
  different things; `replace_panels` was fixed for precisely this, because
  handing the first entry's password to the rest would send one account's sudo
  password to another's session. The writer kept the host-only match, so on
  every save -- adding a server, editing any row, importing from
  `~/.ssh/config` -- both entries cloned the *first* matching table: one
  silently acquired the other's hand-written keys, and the other's were
  destroyed. Matched on the full identity now, with each table handed out at
  most once and an in-order fallback so an edited row keeps its comment.

**One noted, not fixed:** `fuzz/fuzz_targets/fuzz_target_1.rs` is the cargo-fuzz
scaffold -- empty body, `// fuzzed code goes here` -- and is not registered in
`fuzz/Cargo.toml`, so it builds and runs never. It is harmless where it sits and
misleading to anyone reading the directory as a list of what is fuzzed. The
roadmap's "6 fuzz targets" counts the registered ones and is correct.

#### Eighteenth pass -- the harness again, and the record itself. Two more (38 in total)

- *A test was skipped for a precondition that was met.* `local_agent_test`'s two
  tests were `#[ignore]`d as **"requires ssh binary in PATH"**. A local panel
  does not run `ssh` at all -- `spawn_local_agent` execs the agent directly,
  which is the whole distinction -- and `ssh` is at `/usr/bin/ssh` on every
  machine that has ever run them. So the stated precondition was satisfied and
  the tests still failed, on `NotFound` for `multitop-agent`.

  **This is the seventh pass's own finding, in the metadata of the tests for the
  code it fixed.** That pass corrected `stream.rs` so a local panel's missing
  agent binary stopped being reported as "ssh command not found"; the label on
  these tests said exactly that and was left alone. The reasons now name the
  agent, and the `expect`s say what `NotFound` means.

  With `multitop-agent` on `PATH`, **nine of the ignored tests pass here** and
  only four fail -- all `test_remote_upgrade_*`, which genuinely need
  `MULTITOP_TEST_SSH_HOST`. So the seventh pass's conclusion that the *local
  agent tests* are why `--ignored` cannot pass in this sandbox is wrong twice
  over: the blocker is `PATH`, not the test binary's directory, and the local
  tests are not what fails.

- ***There is no coverage gate in CI, and there never was.*** The seventh pass
  recorded `cargo llvm-cov --workspace --fail-under-lines 80` and
  `cargo test --workspace -- --ignored` as "the two CI steps nobody had run".
  Neither is in CI. They came from a stray `ci.yml` **at the repository root** --
  not `.github/workflows/`, so GitHub never reads it -- which is an orphan from
  `e7dab58`, is referenced by nothing, and **does not parse**: a YAML block-
  mapping error at its `test:` job, three spaces where two belong. A previous
  pass read a file that is not CI, cannot be CI, and is not valid YAML, and the
  record here has said "the gate" ever since.

  What is actually true: `.github/workflows/ci.yml` runs fmt, clippy
  `-D warnings`, all four `tools/*.py` gates *with their self-tests*, the
  workspace tests and a bench build. It deliberately does not run the ignored
  set, and says why -- CI has no reachable host. The 80% line threshold exists
  only as `make coverage-check`, which nothing enforces. The stray file is
  deleted; whether the Makefile's threshold should become a CI job is a policy
  call, not a review finding, and is left to the owner.

**What this says about the round.** Two of this pass's findings are the review's
own record being wrong rather than the code -- one pass's conclusion repeated
forward as fact by every pass after it. "Re-run the gates" is not the same
check as "confirm the gates are the ones that run".

#### Seventeenth pass -- the unreproduced failure the sixteenth wrote down. One more (36 in total)

Started by identifying the binary: `vault_upgrade_e2e` is the only one with
fifteen tests. Six isolated runs of it and four full workspace runs were all
green, so reproduction was abandoned in favour of reading it -- and the reading
found something better than the flake.

- *The e2e harness swallowed task panics and reported them as something else.*
  Fourteen sites across four files did `let _ = handle.await;` on a
  production spawn awaited to completion. A panic inside `spawn_upgrade` --
  which has `expect("stdout piped")` in it -- arrives there as a join error, and
  `let _ =` discards it. In `vault_upgrade_e2e` the *only* remaining symptom
  was the `timeout(10s, rx.recv())` two lines below elapsing and reporting
  "task must emit a message": true, ten seconds late, and silent about why.
  That is class H inside the instrument, and it is a complete explanation for a
  failure that appears once under load and says nothing useful.

  Proven by injecting a `panic!` at the top of `spawn_upgrade`: with the fix the
  test fails at the join with "the spawned task must not panic"; without it, the
  panic never reaches the test at all.

  Whether it is *the* failure the sixteenth pass saw is not established, and is
  not claimed. What is established is that if it happens again, the suite will
  now say what happened.

**Also found, not a defect (1):** `upgrade_view_live_e2e`'s four tests are all
`#[ignore]`d, so the twelfth pass's edits to them -- swapping the unnamed `y`
confirm for the named `u` -- compile here but have never run. They are in the
`--ignored` set the seventh pass established this sandbox cannot execute.

**Not diagnosed, and recorded rather than forgotten (1).** One workspace run
during this pass reported `14 passed; 1 failed` in a fifteen-test binary -- one
of the timing-sensitive tokio e2e ones. It did not reproduce in four subsequent
full runs and the failing test's name was not captured. Nothing was concluded
from that: it is written down here because a suite that fails once has either a
flaky test or a real race, and both are worth the next pass's attention.

**Verified rather than remembered (1).** The fourteenth pass's `-tt` finding
rests on a claim about pty semantics that was asserted from memory. Probed on
this machine: a child whose stdout and stderr are both a pty slave produces one
stream carrying both, with CRLF endings. Both halves confirmed -- the merge the
fix depends on, and the CRLF that `painted_states` already had a test for.

**Gates re-run, including the two that need care.** Coverage measured in a fresh
worktree per the seventh pass's rule: **78.00% lines**, against the 76.73% that
pass recorded. Still under the 80% gate, for the reason it documented -- this
sandbox cannot run the `--ignored` local-agent tests.

**Checked and acceptable (2):** `state.rs` renames a corrupt file over any
previous `state.toml.unreadable`, so a second corruption loses the first
preserved copy -- the point is keeping it out of the way of the next *write*,
and a chain of timestamped copies is more machinery than the case earns. And
`persist_state` notes to every panel rather than one, which is right for a
global condition, and bounded: once per upgrade completion, against a ring whose
floor is now fifty lines.

**The sixteenth did exactly that and found another**, in a notice path that
predates this session. Two consecutive passes over the same diffs, two
findings: re-reading is not exhausted by one attempt.

**Checked and correct (2):** `sh_quote` is correct POSIX single-quoting, and the
double layer in `spawn_command` (`sh_quote`, then `'\''`-escaping the result for
the inner `zsh -c '...'`) composes correctly; and the `-tt` pty's CRLF is
already handled -- `tokio`'s `Lines` strips `\r\n`, and `painted_states` has a
test for the CRLF case.

**Checked and correct (1):** `config_ui`'s notice rendering. The delete
confirmation's `[y] confirm  [Esc] cancel` reaches the screen -- the notice is
placed above the hints and in the height budget before them, and wrapped rather
than clipped, both from earlier fixes. `[D] Delete` in the hint row promises
more than `d` does (it asks first), which is the safe direction and left alone.

**Checked and deliberately unchanged (1):** the remote upgrade lock's `ts` stamp
is written just after the directory rather than with it, so a crash in that
window leaves a lock no later run can time and therefore none will break. The
window is a few instructions wide, the panel's held-lock message already names
the exact path to remove, and both alternatives are worse -- a `find -mmin`
fallback puts a GNU-ism in a script that otherwise runs on any POSIX `sh`, and
breaking an unstamped lock on sight lets a second client stamp over a run that
has just acquired it. The doc comment claimed the six-hour break covered every
crash; that claim is corrected rather than the script.

**Checked and correct (2):** `HostUpdate::duration_secs` guards `f >= s`, so a
clock that went backwards yields `None` rather than underflowing; and
`insert_opt_u64`'s `expect("u64 fits in i64")` is genuinely unreachable --
every value that reaches it is either a `SystemTime` epoch second or a value
that already came back through `u64::try_from(i64)`.

**Checked and deliberately unchanged (1):** the Secure Enclave re-bind in
`vault::api` is silent by design and stays that way. It runs inside
`unlock_with_password`, repairs something the user did not ask to have repaired,
and has no UI surface to report through; if it fails the password path is
untouched and the vault still opens, so the user is no worse off than before it
ran. The comment above it already says so.

**Checked and deliberately unchanged (1):** `SIGTSTP` is still fatal, and that
is the right call for now. Raw mode clears `ISIG`, so Ctrl-Z never becomes a
signal -- it arrives as an unbound key. Only an external `kill -TSTP` can stop
the app, and `SIGCONT` already repairs the terminal on resume. Catching it
properly means restoring the terminal and then re-raising with the default
disposition, which needs `raise` -- unsafe, denied workspace-wide -- or a
spawned `kill`, whose restore-then-stop sequence has a race of its own. The
trade is not worth it while the only route in is a deliberate signal from
another terminal.

### The next round: rendering, and what persistence still has not been asked

**Round C is not done.** It stood at twenty passes and forty-two findings on
2026-08-04, and this heading claimed it was finished for most of them -- written
when the round was seven passes old and never revisited. That is the same defect
as the CI file the eighteenth pass deleted: a stale claim in the record, read
forward as fact.

- **Rendering beyond the Configuration panel** -- `ui.rs`, `refit.rs`,
  `ansi.rs`. `render_views.rs` covers screens at four sizes and is a gate, but
  it has never been used as the instrument of a review round. `ui.rs` has been
  read only where the twelfth pass needed the two confirm rows; `refit.rs` and
  `ansi.rs` have not been opened at all. What it should ask: what a zero-width
  or one-column pane does; what an unterminated escape sequence spanning a refit
  boundary does; whether any pane can be given a negative or wrapping arithmetic
  result at extreme sizes.
- **Persistence -- both remaining questions were asked by the twentieth pass**
  and are written up in the review log below. Nothing here is outstanding except
  the concurrent-instance behaviour, which is recorded there as a design
  decision rather than a defect. The entry that used to sit here read: The ninth and tenth
  passes read `config.rs` and `state.rs` and answered the first of the three
  this entry used to pose -- *what a partially written or corrupt state file
  does on load* -- with three findings. Still unasked, and neither is idle:
  **what two multitops sharing one config do** (both write `state.toml` through
  `write_atomic`, so neither tears, but the later writer's whole map wins and
  one instance's upgrade history is lost), and **whether any write path can lose
  a key the user set by hand** (`save_servers` reuses the existing table per
  host to keep comments, which is why comments survive -- but nothing has
  checked what happens to a key it does not know about).

## 2. Clear the test-only baseline

`tools/test_only_baseline.txt` lists functions exercised by tests and by no
production path. The gate (`tools/check_test_only_code.py`) stops new ones
appearing; the existing list has to be worked down by hand, and the file can only
shrink — a stale entry fails the gate too.

Six entries remain:

| Entry | Shape |
|-------|-------|
| `crates/vault/src/crypto.rs:from_config` | Reached only when `argon2_params` is `Some`, which now happens only under the test-mock flag (`vault::config_for`) |
| `crates/agent/src/render.rs:frame_height` | Accessor |
| `crates/multitop/src/app.rs:had_upgrade` | Accessor |
| `crates/multitop/src/app.rs:vault_unlocked` | Accessor |
| `crates/multitop/src/panel.rs:set_sudo_password` | Accessor |
| `crates/vault/src/lockout.rs:uses_keychain` | Accessor |

The reason this is worth the effort is not the dead function itself: it is that
a *duplicate* of its logic is what production calls, the tests guard the dead
copy, and the live copy drifts unwatched. `rollback::parse_stored_counter` and
`LockoutState::on_failure` were both exactly that.
## 3. The other half of in-place progress output

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
## 4. `G` — per-pane CPU / memory / network graphs

Requested 2026-08-03. Start only once items 1-3 are closed; item 5 follows it.

A new view alongside the existing ones, bound to `G` and placed immediately to
the right of `F` (Fetch) in the keybar, drawing CPU, memory and network history
as graphs inside each pane. Use btop's graph glyphs — the braille-style
sub-cell blocks that give several vertical steps per character row.

Points to settle before writing any of it:

- The agent currently ships point-in-time samples; a graph needs history. Decide
  whether the ring buffer lives in the agent (more data over the wire, survives
  a view switch) or in `Panel` (cheap, but empty for the first N seconds after
  a panel is rebuilt by `replace_panels`).
- There is no per-panel history renderer any more: `sparkline.rs` was deleted
  on 2026-08-04 (`6aaf8bb`). `G` starts from nothing, which is the right
  footing — one renderer, built for the job, rather than a second one beside a
  drifting first.
- `s` is free in Settings now, if the graph view wants a preference there.

## 5. A filtered grid tells the agents how big its panes are

Requested 2026-08-04. **After item 4** — the owner's ordering, not a dependency.

Filtering already hides the panes that do not match and re-splits the grid over
the ones that remain: `ui::draw` calls `regions(f.area(), shown.len())`, so one
match on an eight-host grid is drawn full-screen. What has not been told is the
*agent*, which is still rendering for a pane one quarter of that size, so the
pane grows and its contents do not.

The infrastructure is all here, and it arrived with Round C. `AgentDims` takes
the pane count as one half of the signature it diffs, and `refresh` already
runs on a resize and on a server-list edit. What is missing is small and named:

- the count handed to `refresh` is `app.panels.len()`, and it has to be
  `app.filtered_indices().len()` — the same count `ui::draw` lays out from, so
  the two cannot disagree;
- the filter has to trigger a refresh. Typing into the query changes the
  visible count on almost every keystroke, so this wants the resize debounce
  rather than a refresh per character, and `app.rerender_all` on the way out;
- `filtered_indices()` can be empty while a query matches nothing. `regions`
  returns no rects for zero panes and `agent_dims` returns its minimum, which
  is the right floor — but the screen wants to say *why* it is empty rather
  than showing nothing.

Worth settling first: whether clearing the filter should re-render every pane
at the restored size immediately, or let the next agent frame do it. The first
costs a burst of work on a keystroke; the second leaves the grid showing
stale-sized content for up to a tick.

## Deferred

| Item | Why |
|------|-----|
| TPM2 wrapper | Would make Linux fingerprint unlock actually release a key. Until it exists, `fprintd` cannot unlock anything, so `try_unlock_biometric` does not prompt for it. |
| Post-quantum KEM | Not warranted for a device-local file threat model. |
