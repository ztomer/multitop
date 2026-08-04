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
| Terminal / process lifecycle | Round C, 2026-08-04, seven passes so far, with `tests/event_loop_e2e.rs` built for it | **On findings, seven times.** 7, 3, 1, 2, 3, 1, 1. What is running out is unexamined surface, not defects. Two classes account for seven of them: *one quantity derived in two places by different rules*, and class H, *a failure reported as something else*. An eighth pass is owed. |
| SSH + upgrade transport | Covered by Round C where the lifecycle reaches it -- child process groups, the two output streams, the session handshake, the packet-decode boundary | Partial. `read_handshake` and `interpret_packet` now have seams and tests; the bootstrap retry was read and found correct; the rest of the framing has not been reviewed. |

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

The streak stops at eight. Round C's eighteen findings were all found by review,
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
- `cargo llvm-cov --workspace --fail-under-lines 80` **cannot be trusted
  against a stale `target/`**. Run in place it reported 54% and listed
  `multitop/src/sparkline.rs`, a file deleted on 2026-08-04 -- 28,279 regions
  against a clean tree's 19,605. Measured properly, in one fresh worktree per
  commit: `be5b83a` 73.85% lines, this round's HEAD 76.73%. So the round raised
  coverage by three points, and both numbers sit under the gate here because
  this sandbox cannot run the paths that need a real agent binary. **Measure
  coverage in a fresh worktree or not at all.**

**An eighth pass is owed. The counts so far are 7, 3, 1, 2, 3, 1, 1** -- and they are not
converging. Every pass so far went back up the moment it opened a part of the
loop the earlier ones had not looked at, which is the argument against reading
a falling count as progress toward zero. What is running out is *unexamined
surface*, not defects; the round ends when a pass covers the same ground and
finds nothing, not when the number gets small.

**Checked and deliberately unchanged (1):** `SIGTSTP` is still fatal, and that
is the right call for now. Raw mode clears `ISIG`, so Ctrl-Z never becomes a
signal -- it arrives as an unbound key. Only an external `kill -TSTP` can stop
the app, and `SIGCONT` already repairs the terminal on resume. Catching it
properly means restoring the terminal and then re-raising with the default
disposition, which needs `raise` -- unsafe, denied workspace-wide -- or a
spawned `kill`, whose restore-then-stop sequence has a race of its own. The
trade is not worth it while the only route in is a deliberate signal from
another terminal.

### The next round: rendering, and the persistence path

Round C is done (above). Two areas remain that no round has ever touched:

- **Rendering beyond the Configuration panel** -- `ui.rs`, `refit.rs`,
  `ansi.rs`. `render_views.rs` covers screens at four sizes and is a gate, but
  it has never been used as the instrument of a review round. What it should
  ask: what a zero-width or one-column pane does; what an unterminated escape
  sequence spanning a refit boundary does; whether any pane can be given a
  negative or wrapping arithmetic result at extreme sizes.
- **Persistence** -- `config.rs`, `state.rs`. Both defects found here so far
  (non-atomic writes, comments destroyed on every write) were found ad hoc. What
  it should ask: what a partially written or corrupt state file does on load;
  what two multitops sharing one config do; whether any write path can lose a
  key the user set by hand.

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
