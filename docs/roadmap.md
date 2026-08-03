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

## 2. What the UX panel found, by class

Round B of the review (see item 4). Every claim below was verified against the
source or against a rendered frame before it was written down; persona claims
that did not survive checking are not here.

### How disagreements are settled  (owner decree, 2026-08-03)

**Two experts disagreeing is not resolved by the orchestrator. Bring in a
third.** Whoever is running the review does not get to average two opinions, or
to pick the one they already agreed with. Find a third expert whose bar bears on
the question, put the disagreement to them as posed, and take their answer.

**For UX and UI questions the third is Kare, and her call is final.** Layout,
legibility, labels, glyphs, what a screen shows and how it degrades: no further
arbitration.

**The boundary, and it matters.** Correctness, safety, gates and cost are not UI
questions and do not go to Kare:

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

**Resolved by the decree, immediately:** the keybar. Rams wants the theme badge
*deleted* outright to buy back columns; Kare wants all three badges kept and
**shed whole, in priority order** (Sort, then Theme, then Settings) as the width
runs out, with initials below ~44 columns. That is a UI question and Kare has
it: shed in order, never delete a badge that is affordable at the width in
front of you, and never slice one. Rams' `{:<11}` complaint survives untouched
-- both of them raised it, so it is not a tension at all.

**Settled by the third expert:** the confirmation modal. Rams and Hashimoto
disagreed, it went to Kare, and she ruled. See "The confirmation, as ruled"
below -- it is a decision, not an open question, and it needs nothing from the
owner.

The safety half turned out not to exist. It was recorded here as "whether a tool
that runs `apt upgrade` on production may drop its confirmation step at all",
and that was a misreading of Rams: he proposed *moving* the confirmation to the
keybar row, never removing the gate. Both experts wanted a confirmation
throughout; they disagreed only about its form, which is entirely Kare's. The
splitting rule was right and simply had nothing to split here.

Grouped by **class**, not by persona, because the value of the round was that
four independent bars kept landing on the same few root causes. Fixing per
finding would fix each symptom once and leave the cause alive -- which is
exactly what happened earlier the same day, recorded under class B.

### A. Row 0 has two owners  (high)

`ui::visible` composes the scroll badge into `lines[0]` (`ui.rs:137`);
`ui::draw` then overwrites `lines[0]` with the host banner unconditionally,
every frame (`ui.rs:536`). Two pieces of code believe they own that row and the
later one silently wins. Two symptoms, found independently from opposite ends:

- **A one-line body is eaten.** `Panel::new` sets `view: ["connecting..."]`, so
  a host coming up renders an **empty box** -- indistinguishable from a hung SSH
  session or a dead app. `grep -rl connect target/views` matches none of the 88
  rendered frames.
- **The scroll badge can never appear.** `[↑ -N lines]` is built and destroyed
  on the same frame, so the scroll-position indicator has never once been on
  screen.

The test that should have caught the badge (`upgrade_ux_test.rs:634`) calls
`visible()` with `target_cols = 0`, which skips the whole badge path. It passes
against code that renders nothing.

**Fix (structural, not two patches):** make the ownership unrepresentable.
`visible()` returns body rows plus the `badge_offset` as a value; `draw`
composes row 0 exactly once, from banner + sparklines + badge together. Then pin
it with a test that renders through `ui::draw` into a `TestBackend` and asserts
the badge is in the buffer.

### B. Fixed-width layout with no budget  (high)

Every one of these is the same defect: content laid out to a constant, a
terminal narrower than the constant, and `Paragraph` clipping mid-word.

| Where | At 80 columns | At 40 |
|-------|---------------|-------|
| Keybar (`ui.rs:324`) | `[Sort: Cpu/Mem]` -- the only place the sort order is stated -- falls off; `[Theme: Kare` cut mid-word | dies at `Upgrad` |
| Settings row (`config_ui.rs:85`, 75 fixed columns) | Password column reads `✓ S` / `· U`; header reads `Pas` | column gone entirely |
| Modal copy (`modals.rs:114`, no `.wrap()`) | fits | `Press U or Enter to confirm, Esc t` -- a destructive dialog amputating its own cancel instruction |
| Settings hint row (`config_ui.rs:144`) | fits | orphaned `[`, and `[Esc/Q] Return` is shed -- the only exit signage on a screen that covers the keybar |

**This class was already open and got fixed at the instance level.** Earlier the
same day the settings row's *command* column was given a `clip` helper, which
cured the misalignment and left the row 75 fixed columns wide -- so the
credential state still falls off. Rule 6, unlearned, by the person who wrote
rule 6 into this file.

**Fix:** one width-budget helper shared by all four. Fixed cells (marker, state,
port) never give; flexible cells (host, user, command) share the remainder; a
chunk is emitted whole or dropped whole, never sliced. Shed in a declared
priority order, and put the thing the user cannot guess -- the way *out* --
first, so it is never what gets dropped.

Kare's rule, binding: **a label is whole or it is absent; there is no third
state.**

**Build this helper first.** Her ruling on the confirmation (below) *deletes*
the modal-clipping row of the table above rather than fixing it -- a single
keybar row cannot clip if it is assembled from whole chunks -- and the same
mechanism serves the keybar badges and the settings hint row. Three defects and
one new feature, one helper. Anything built before it will be rebuilt by it.

### C. The banner is not in the family, and it clips the identifying tail  (high)

`fmt::fullwidth` maps the host name into U+FF01-FF5E for the panel banner
(`ui.rs:524`). Two problems, the second serious:

- Those codepoints are absent from Menlo, SF Mono, JetBrains Mono and Berkeley
  Mono, so the banner falls back to a CJK face: a different typeface, weight and
  baseline from every line beneath it.
- It doubles the cell cost, so `ztomer@web-01` needs 26 cells in a 20-cell pane
  and clips to `ｚｔｏｍｅｒ＠ｗｅ`. **The digits are what fall off** -- `web-01`
  and `web-02` render identically at four panels on a small terminal, on a tool
  where the selected panel is the machine `u` runs `apt upgrade` against.

**Fix:** plain ASCII in the accent colour, bold -- already the only bold accent
line in the pane. When it still will not fit, drop the `user@` prefix first (it
is identical in every panel, so it carries no information), then ellipsize from
the **left**, `…b-01`, so the distinguishing end survives.

### D. Cost paid per frame for nothing  (med)

- `app.rs:1056` and `:1066`: `last_upgrade.drain(..len - cap)` with
  `cap = 5000`, twice per line, on the event-loop thread. Once the buffer fills,
  every further line memmoves ~5000 `String` headers. `apt upgrade` on a
  neglected box emits tens of thousands of lines, and this is the app's headline
  feature. Fix: `VecDeque` + `pop_front`, or amortize (drain only at
  `cap + 512`), and stop the `line.clone()`.
- `run.rs:347`: a `TIOCGWINSZ` syscall plus two `Layout::split` allocations on
  every mouse event -- including motion, which crossterm's `EnableMouseCapture`
  requests with `?1003h` (any-event tracking) -- then discarded by `_ => {}` for
  all but three event kinds. Fix: compute inside the three arms that use it, and
  track terminal size from the `Resize` event instead of asking the kernel.
- `run.rs:384`: `dirty = true` after every message. A `Monitor` packet for a
  panel showing Docker changes nothing visible and still forces a full redraw of
  every panel. Fix: `apply` returns whether anything visible changed;
  `dirty |= app.apply(msg)`.

### E. Names and labels  (med)

- **Three names for one screen:** README says *Configuration* (`README.md:63`),
  the keybar badge says *SEttings*, the panel title says *Server Settings*
  (`config_ui.rs:182`). The third is wrong on its own terms -- that screen holds
  the vault master password and a sparklines toggle, neither of which is a
  server. Pick **Settings** and use it three times.
- `[SEttings]` highlights the **second** letter (`ui.rs:219`) while every other
  key in the bar highlights the first. A mnemonic that has to be explained is
  not one. Print `[E] Settings`.
- `format!("{:<11}", theme.name)` (`ui.rs:216`) pads a four-letter word with
  seven dead columns, at exactly the width where the bar overflows.
- `"/ "` renders the sort badge as `Cpu/ Mem` (`ui.rs:236`).
- **The app tells the user to do by hand what the app does.** Both
  `upgrade_view.rs:163` and `modals.rs:85` say to set `upgrade_cmd` in
  `config.toml`, while the settings row editor has that field and writes it back
  to that file. Replace with the in-app path.

### F. The blast radius is not what the screen says it is  (high)

The operator bar found the two worst defects of the round, and neither is
cosmetic.

- **The filter narrows the screen but not the run.** `ui::draw` honours
  `filtered_indices()` (`ui.rs:435`); `run_upgrade` iterates
  `0..self.panels.len()` (`app.rs:794`). `filtered_indices` has exactly two
  callers -- the renderer and the selection clamp -- and the upgrade path is not
  one of them. So `/db` Enter `u` `u` Enter runs `apt upgrade` on **every host in
  config.toml** while one host is on screen, and the output and failures of the
  hidden ones never render, because the grid is still filtered when they come
  back. The modal's "all servers" is technically true and practically a trap:
  the filter's whole purpose is that "all servers" is now a set the operator
  cannot see. **Decide which it is** -- scope the run to the filter, or state in
  the modal, in words, that the filter does not scope it. Silently disagreeing
  with the screen is the one option not available.
- **Esc quits, kills a live `apt upgrade`, and asks nothing.**
  `upgrade_view.rs:151` prints "→ running -- do not quit" and nothing enforces
  it: `run.rs:619` calls `app.quit()` unconditionally, `abort_all` drops every
  handle, children are `kill_on_drop`, and dpkg dies mid-transaction on a
  production box. Deleting one *row* in Settings takes two keys; killing a
  package transaction on N production servers takes one, and it is the key an
  operator presses to back out of a screen. **Fix:** when `upgrades_in_flight()`,
  Esc/q/Ctrl-C confirms first, naming the hosts; and on quit, print to stderr
  after terminal restore which hosts were killed and that the remote lock may
  need removing.

### G. Three help lines name two keys that do not exist  (med)

`upgrade_view.rs:236` says `will prompt · p to save`. `tasks.rs:462` says
`Set a password for this host with o in Settings`. `password_actions.rs:324`
says `set those again with p`. Neither `p` nor `o` is bound anywhere:
`grep -rn "Char('p'\|Char('o'" crates/multitop/src` returns nothing. The key is
`e` -- which `tasks.rs:427` gets right, so the codebase names three different
keys for one action and two of them do nothing. This is the worst shape a doc
lie can take: it appears at the exact moment the operator is stuck and needs the
instruction to work first time.

**Fix:** one constant for the settings key and its human label, referenced by
all four sites, plus a test that scans user-facing strings for key references
and asserts each is a live arm of `run::handle_key`. Structural, not
disciplinary -- this is the third naming defect in the same file set.

### H. Two failures that are reported as something else  (med)

The project already fixed this class once, for a refused sudo password (exit 111
plus a marker line, so it is no longer indistinguishable from a failing
command). Two siblings were never given the same treatment:

- **A held remote lock reads as a failing command.**
  `ssh.rs:287` ends `echo "Upgrade already in progress"; exit 1`, which lands in
  the generic arm, so the panel says `⚠ upgrade command exited 1 -- host
  reachable, command failed`. It did not fail; it never ran. A lock left by a
  killed run is broken only after six hours, so for six hours every attempt
  sends the operator to debug an apt command that is fine. Fix: a
  `LOCK_HELD_CODE` and sentinel matched ahead of the generic arm, naming the
  lock file to remove.
- **The shipped example cannot work for the user it is written for.**
  `config.example.toml` offers `upgrade_cmd = "apt update && apt upgrade -y"`
  next to `user = ""`. Uncommented by a normal login, apt exits 100 with "are
  you root?" -- the per-host sudo machinery does not save it, because the
  preamble runs `sudo -v` and then the command runs *without* sudo. `is_sudo_help`
  does not fire either, because apt's message contains no "sudo". Meanwhile
  every rendered frame shows `sudo apt update && sudo apt upgrade -y`: the
  product's own screenshots disagree with the file it ships. Fix: put `sudo` in
  the example and say the command runs as the SSH user; teach `is_sudo_help` the
  root-permission shapes.

### The confirmation, as ruled  (Kare, third expert, 2026-08-03)

**Rams takes the placement, Hashimoto takes the content, neither gets what he
asked for.** Binding.

**1. A keybar row, not a box.** The frames decided it: at 40x12 the box is 38
cells wide and clips its own cancel line to `Esc t`, while at the same size the
filter prompt renders every word whole. One of those two patterns survives the
smallest size in the product and it is not the box. A box that has to be taught
to wrap, to compute its own height, and to shed lines in priority order is a box
being rebuilt into a row the hard way.

And the covering is real: at 40x12 the box sits on top of db-02. You do not
print a summary over the thing it summarises.

**2. Hashimoto's hidden set gets SHOWN, not listed.** He was right that "all
servers" conceals a set the operator cannot see, and wrong about the remedy: a
list of eight hostnames typeset over the eight panes that already name those
hostnames is the same information twice, once badly. The Upgrade view *is* the
enumeration. So -- **arming the confirmation clears the visual filter for its
duration.** The grid springs from one pane to eight and the operator sees the
six he was about to touch blind, in full detail, in the layout he already knows.
Disclosure by uncovering rather than by overprinting.

**3. The row, matching the filter prompt's grammar exactly** -- state left, keys
right, two spaces between:

```
Upgrade 8 hosts · 2 skipped  [U] go  [Esc] cancel      (>= 56 cols)
Upgrade 8 hosts  [U] go  [Esc] cancel                  (40 cols, 37 used)
```

Shed order, whole chunks only: `· 2 skipped` goes first, because the ⚠ is
already in those panes. `[Esc] cancel` goes last and in practice never -- it is
the only thing on the line the operator cannot guess, since he pressed `U` to
get here.

**The count is the alarm.** If the grid was showing one host and the row says 8,
that discrepancy is louder than any sentence that would fit in the remaining
cells.

**Build-order consequence, and it is the good kind.** This ruling *deletes* the
modal-clipping finding rather than fixing it -- a single row cannot clip if it
is built from chunks emitted whole or not at all. That is the same mechanism the
keybar badges and the settings hint row need. Three defects, one small helper,
one rule: **a label is whole or it is absent.** Build the helper first and the
confirmation row falls out of it for free.

### Still open, and not a UI question

Kare's ruling settles *disclosure* -- the operator sees every host he is about
to touch. It does not settle **scope**: `run_upgrade` still iterates every panel
while `ui::draw` honours the filter (class F). Un-filtering at the confirmation
makes the disagreement visible instead of silent, which is the minimum bar, but
"a filter narrows the screen and not the run" remains a semantics decision. The
alternative -- scoping the run to `filtered_indices()` so `/db` then `u` upgrades
db-02 alone -- is defensible and is the owner's call, not a reviewer's.

**What each argued, for the record.** Rams: the modal says *less* than the view
it covers -- "all servers" instead of naming them, one aggregate `Last update`
instead of the per-host truth, a skipped list duplicating the ⚠ already in the
panes, and in the rendered frame it obliterates db-02; delete
`draw_upgrade_modal` and put the confirm in the keybar row, where the filter
prompt already proves the pattern (`run.rs:691`, `ui.rs:191`). Hashimoto: keep
the box and make it enumerate, because "all servers" is the phrase that hides a
filtered-away set (class F) -- and separately the box drops its own "Esc to
cancel" line whenever a skipped host and an interrupted previous run co-occur,
which are both ordinary states.

## 3. Decide the fate of two unused vault API functions

`UnlockedVault::remove_password` and `Vault::get_unlocked` are implemented and
tested with no production callers. `remove_password` would be per-host removal
*from the vault*, which is distinct from the credential-store deletion that an
emptied password field already performs — if that distinction is not wanted,
delete it. Both are listed in `tools/test_only_baseline.txt`.

## 4. The adversarial review is not finished

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
being written down. They are item 2, grouped by class rather than by persona.

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

## 5. Clear the test-only baseline

`tools/test_only_baseline.txt` lists functions exercised by tests and by no
production path. The gate (`tools/check_test_only_code.py`) stops new ones
appearing; the existing list has to be worked down by hand, and the file can only
shrink — a stale entry fails the gate too.

Nine entries remain:

| Entry | Shape |
|-------|-------|
| `crates/vault/src/api.rs:remove_password` | Item 3 above |
| `crates/vault/src/api.rs:get_unlocked` | Item 3 above |
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

## 6. Finish keychain isolation for tests

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

## 7. The other half of in-place progress output

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

## 8. Rotate the sudo password used during live verification

The sudo password for the three test hosts was pasted into a Claude Code session
transcript on 2026-08-02 in order to verify the stdin handshake. It is therefore
on disk in `~/.claude/projects/`. Change it on all three machines.

## 9. `G` — per-pane CPU / memory / network graphs

Requested 2026-08-03. **Last** — start only once items 1-8 are closed.

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
