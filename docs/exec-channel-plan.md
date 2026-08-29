# Plan — move the upgrade channel onto the agent's binary protocol

> Per rule 13 this file is **deleted when the work lands**. What survives goes
> into `roadmap.md`'s detection record and into the skills. Do not let it become
> a graveyard entry.

---

## 1. The class

**One reader, N stream shapes, and the shape is chosen by something outside the
program.**

multitop has four SSH data channels. Three of them — Monitor, Docker, Fetch —
carry length-prefixed `MTOP` packets over `ssh -T`. The fourth, the upgrade,
carries **raw text over `ssh -tt`** and asks a hand-written reader to work out
where one record ends and the next begins. `ssh_command_tty` has exactly one
production caller (`ssh/spawn.rs:274`); every other channel goes through
`ssh_command`.

Seven of the twenty-seven rows in `roadmap.md`'s detection record are defects in
that one channel: the sudo-handshake deadlock, the `\r` progress bar logging a
line per tick, the `ESC[nA` block logging a copy per tick, the flooded-channel
wedge, the `select!` race that lost the session's reason at random, the
`__multitop_lock_held__` marker printed to the operator verbatim, and the
post-upgrade freeze. **Zero** are in the three framed channels. The defect rate
is not distributed across the program; it is concentrated in the one place we
do not own the framing.

### 1.1 Evidence — the shape is decided by a file in `~/.ssh`

Probed live against `192.168.0.33` (OpenSSH_10.3p1 client), command
`echo OUT; echo ERR >&2; tty`, stdout piped through `cat -A`:

| ControlMaster state | pty? | stdout bytes | stderr |
|---|---|---|---|
| multiplexed (socket bound) | `/dev/pts/1` | `OUT␊ ERR␊` | merged |
| unmultiplexed (`ControlMaster=no`) | `/dev/pts/1` | `OUT␍␊ ERR␍␊` | merged |
| stale file at `ControlPath` | `/dev/pts/1` | `OUT␍␊` + `disabling multiplexing` on stderr | merged |
| local panel (`is_local`, `$SHELL -c`) | none | `OUT␊` | **separate pipe** |

Warm vs cold mux is *not* the variable — both multiplexed runs gave LF. The
variable is **multiplexed vs not**, and a leftover file at
`~/.ssh/multitop-%C` is enough to flip it. `stdin` being a tty is irrelevant
(verified with `</dev/null`).

So the upgrade reader is handed **three different byte streams** for the same
command, and which one arrives depends on a file it does not control.

### 1.2 The house already solved this class once

`ssh/command.rs::upload_command` carries the note:

> `cat` cannot tell a finished stream from an interrupted one — both end in EOF.

The fix was to **put an explicit length on the wire** (`expected`, checked with
`wc -c` before the `mv`). That is the same class and the same answer. This plan
applies it to the one channel that still guesses.

---

## 2. Siblings found (fix with the class, not after it)

| # | Sibling | Where | Status |
|---|---|---|---|
| S1 | Upgrade output framing — the reported bug | `tasks/upgrade.rs` | this plan |
| S2 | `ControlMaster=auto` is documented as degrading gracefully when the `ControlPath` cannot be bound. **It does not** — `ssh` exits **255** with `unix_listener: cannot bind to path …` and no connection at all. On a host with no `~/.ssh`, *every* channel fails, not just the upgrade. | `ssh_opts.rs:41` comment vs. reality | independent defect, fix in this round |
| S3 | "a pty has one stream … everything the remote writes arrives on stdout" is true **only unmultiplexed**. Multiplexed, stderr stays a separate channel. The one-scanner fix it justifies is right; the recorded reason is wrong for the common case. | `tasks/painted.rs:25` | doc correction + test |
| S4 | The stderr timeout arm is `Err(_) => {}` — partial stderr is never flushed until EOF, so a reason written without a trailing newline is lost. | `tasks/upgrade.rs:266` | folded into the rewrite |
| S5 | Text sentinels (`===NEEDAGENT===`, `__multitop_pw_ready__`, `__multitop_sudo_failed__`, `__multitop_lock_held__`) are line-shaped markers in a stream whose line shape is exactly what varies. One of them has already reached the operator's log verbatim. | `ssh/command.rs`, `tasks/spawn.rs` | become protocol `Marker` frames |
| S6 | The remote and local upgrade locks have **drifted**: `wrap_with_upgrade_lock` breaks a stale lock on a 6-hour timestamp only; `wrap_with_local_upgrade_lock` also checks PID liveness. A killed remote run blocks upgrades for six hours; a killed local one recovers immediately. Same logic, two quoted shell strings, one behind. | `ssh/spawn.rs` | one implementation in the agent |
| S7 | `deliver_sudo_password` consumes through `Lines`, then `into_inner()` discards whatever partial line `Lines` had buffered. | `tasks/spawn.rs:28` | deleted with the sentinel hunt |

---

## 3. The defects in the reported bug

| | | |
|---|---|---|
| **F1** | Framing decided by the mux — §1.1 | root |
| **F2** | The timeout branch feeds `stdout_str` to the painter and **never clears it**. Every 100 ms of mid-line silence re-emits the same text and re-scans it for markers. | `upgrade.rs:189` — **the duplication** |
| **F3** | The char loop flushes on `\r` *and* on `\n`. On the CRLF shape each line yields one text feed plus one empty feed, and the empty feed decrements `Painter::up` a second time, so `ESC[nA` block repaints drift and append copies instead of overwriting. | `painted.rs:135` — **the other duplication** |
| **F4** | `if tx.send(msg).await.is_err() { return; }` returns **without `AuxDone`**, and there is no stall or overall deadline. The file's own header says the cost: panel pinned in `STARTED`, `upgrades_in_flight()` never clears, quit needs a confirm, no further upgrade can start. | `upgrade.rs:192,209` — **the stuck system** |
| **F5** | Both `select!` arms rearm a 100 ms timeout, so an idle upgrade wakes the runtime 20×/s per host. | `upgrade.rs:150` |
| **F6** | = S7. | `spawn.rs:28` |

F2–F6 are all consequences of F1: they are what guessing record boundaries out
of an externally-shaped stream looks like in code.

---

## 4. Design — `ProtoMode::Exec`

Extend `MTOP`; `PROTO_VERSION` 4 → 5, `ProtoMode::Exec = 3`.

```
Payload::Exec(ExecEvent)
  Begin  { host, agent_version, pid }
  Out    { stream: u8 (0=stdout, 1=stderr), seq: u32, bytes }  // raw, as the child wrote them
  Marker { kind: PwReady | SudoFailed | LockHeld }             // never reaches the operator's log
  Alive  { elapsed_ms }                                        // ~1 Hz heartbeat
  Exit   { code: i32, signalled: bool }                        // the ONLY source of AuxDone
```

**Agent side** — `multitop-agent exec`:

* takes the command as a **framed request on stdin**, never argv (argv is
  world-readable; this is the same reason the sudo password already is not
  there);
* runs it under a pty **it** allocates (`libc::forkpty`), so the shape is
  identical whether ssh multiplexed, did not, or there is no ssh at all;
* owns the upgrade lock as real Rust with real tests, replacing the two drifted
  quoted shell strings (S6) with one implementation that keeps the PID check;
* emits `Exit` on **every** path, including panic and signal.

**Client side** — `spawn_upgrade` becomes a packet reader. No timeouts, no `\r`
guessing, no chunk buffer, no sentinel hunt. `Out.bytes` go through one line
assembler (the existing `Painter`, fed whole lines, with the CRLF double-feed
closed). `Exit` produces `AuxDone`. A missing `Alive` for N seconds is a bounded
stall that is reported and ended — never a panel pinned in `STARTED`.

**Transport** — plain `ssh -T`, the same path the three working channels use.
No `-tt` anywhere. The local panel runs the same binary via `--agent exec`, so
local and remote collapse to one code path and one stream shape.

**Fallback** — only when the remote architecture has no embedded agent. Stated
in the panel, never silent. `ssh_command_tty` and the raw-text reader are
deleted otherwise; keeping both alive means two framing models forever, which is
how we got here.

**Compatibility** — agents are content-addressed per build
(`agent-<hash>`), so a `PROTO_VERSION` bump re-uploads on first connect. The
`===NEEDAGENT===` handshake already covers the miss.

### 4.1 Decision — the pty (settled 2026-08-29)

`forkpty` is real `unsafe` in the agent crate (the crate does not opt into the
workspace's `unsafe_code = "deny"`, so it is permitted). It is chosen over
pipes-only on user experience, which is the deciding axis here:

* interactive prompts (`Continue? [Y/n]`) reach the panel and can be answered;
* `apt`, `docker` and friends keep colour and their `\r` progress displays —
  without a tty they silently switch to a different, duller output;
* `sudo` can prompt rather than failing with "no tty present";
* the local panel and every remote host produce the **same bytes**, so what the
  operator sees does not depend on which host they are looking at.

Pipes-only would be less code and a worse product: it changes what the remote
tool prints, which is the one thing an upgrade log exists to show.

---

## 5. E2E test plan

Every test at L2/L4/L5 is **proven red against current `HEAD`** before the fix
lands (rule 2). L2 and the L4 mux matrix are expected to fail today; the actual
output is reported either way.

### L0 — protocol
* Round-trip every `ExecEvent` variant.
* Oversize and truncation, against the `u16` length field — the class that
  already bit the Docker rows once.
* Version-mismatch: a v4 reader must refuse a v5 `Exec` frame legibly, not
  desynchronise.
* Fuzz target for `decode_packet` over exec frames (`fuzz/` already exists).

### L1 — agent exec, no ssh
Real child processes, asserting exact event sequences:
clean exit · non-zero exit · killed by signal · writes and never emits a newline
· 1 MB burst · `\r` progress · `ESC[3A` block repaint · lock already held ·
child that outlives its parent.

### L2 — client reader oracle
Recorded packet streams → **exact `Msg` sequence**. The assertion that would
have caught F2 and F3: *no line is emitted more times than the input contains
it.* This is the layer the current suite has no equivalent of.

### L3 — live, single host, no TUI
`upgrade_cmd = "ls -la /etc"` against `127.0.0.1`, `192.168.0.33` (x86_64),
`192.168.0.90`, `192.168.0.158` (aarch64 — covers the second embedded arch, and
the only Raspberry Pi in the fleet). Oracle is `ssh <host> ls -la /etc` run
directly. The panel log must equal it line for line.

`ls` rather than an update command on purpose: its output is knowable in
advance, it is idempotent, and it is safe to run a hundred times.

### L4 — live TUI over the unix socket
Promote `tests/test_tmux_e2e.py` into `tests/harness/tmux_session.py`:
wait-for-condition instead of `time.sleep`, guaranteed teardown of strays,
`multitop-agent` resolved from the build tree and never from `PATH` (see the
stale-agent note for `192.168.0.33`).

**The assertion surface is the SIGUSR2 state-tier diag dump, not
`capture-pane`.** `capture-pane` sees an 80×24 viewport and cannot tell
"printed twice" from "scrolled"; the dump gives the panel's `last_upgrade` ring
verbatim. `diag.rs` already writes it — this reuses it rather than building a
second channel.

**The mux matrix** — the F1 variable, run for every host:

1. socket absent (`rm ~/.ssh/multitop-*`)
2. socket warm (a stats stream has already connected)
3. a **stale file** planted at the `ControlPath` — the case that flips ssh to
   unmultiplexed and produces CRLF today

All three must produce byte-identical panel content.

### L5 — stuck
* After `Exit`: `upgrades_in_flight()` clears and `q` resolves the loop within a
  deadline.
* A child that stalls (`sleep 600`, no output) hits the `Alive` deadline, is
  reported, and ends — the panel never stays in `STARTED`.
* The `tx.send` failure path (F4) reaches `AuxDone`.

### L6 — gates and siblings
* `check_gate_parity.py` covers the new python harness.
* Live layers **skip with a stated reason** when `MULTITOP_LIVE_HOST` is unset
  (the env seams already exist) — never silently pass.
* **S2**: a test that points `ControlPath` at an unbindable directory and
  asserts what actually happens (`exit 255`), plus a corrected comment. Whether
  to pre-create `~/.ssh` or to detect and report is decided by what that test
  shows.
* **S6**: the agent-side lock keeps the PID check for both local and remote; a
  test kills a run mid-flight and asserts the next one acquires immediately
  rather than in six hours.

---

## 5a. Progress — agent side complete (2026-08-29)

**Landed.** `ProtoMode::Exec` (protocol 5), `crates/agent/src/exec/` (frame
types, codec, pty, sieve, script, lock, runner), `multitop-agent exec`.
L0 (11 tests) and L1 (22 tests) green; whole agent suite 267 green; clippy
clean. L3 lives in `tests/test_exec_live.py` -- 9 tests, 36 subtests, green
against all three hosts in 3m37s -- with a second, independent wire
implementation in `tests/exec_wire.py`. That second implementation is written
from the documented layout rather than from the Rust, so a disagreement between
them is a disagreement between the code and its own specification; round-tripping
a codec against itself proves only self-consistency.

**Both live assertions were proven red before being trusted** (rule 2):

* breaking the `Done` marker put `\x1b]111\x07` back on the end of the log and
  `test_07` failed with exactly that byte sequence -- while `test_04`
  (the three transports agree) correctly stayed green, because the noise was
  present identically on all three. The two tests measure different things and
  demonstrably do not stand in for each other;
* doubling the output in `stash` produced
  `'zsh' appeared more often than the host printed it` from `test_01`. The
  duplication detector -- the reported symptom -- fires, and names the line.

**The headline measurement.** Three live hosts (`192.168.0.33` x86_64,
`192.168.0.90` x86_64, `192.168.0.158` aarch64) x three multiplexer states
(socket cold, socket warm, plain file planted at the `ControlPath` so `ssh`
disables multiplexing) = nine runs, every one **byte-identical** to
`ssh <host> /bin/ls -1 /etc`. The variable that used to change the stream's
shape no longer changes anything.

### What the work found that the plan did not predict

| # | Finding | Where it came from |
|---|---|---|
| N1 | **The sieve lost ordering inside one read.** Its first version returned `{out, markers}` -- two lists -- and a single 8 KiB read routinely holds startup noise, the `Started` marker, and the first real output. Handled out of order, the real output was dropped with the noise. Now an ordered `Vec<Piece>`. Order is load-bearing twice: `Started` says which side of it a byte falls on, and `PwReady` says the far side has turned echo off, so anything written before it is echoed into the operator's log. | L1 went red |
| N2 | **A login shell is not quiet at either end.** `zsh -l -i` emits `OSC 111` on the way in *and* on the way out; a host with a banner in `.bashrc` adds more. All of it used to land in the upgrade log. The command is now bracketed between `Started` and `Done` markers and the log is the command's output and nothing else. The interactive login shell is not optional -- it is what makes an alias like `ud` resolve. | L1, then the live run (the trailing one only showed up over SSH) |
| N3 | **`execvp` after `fork` is not async-signal-safe** -- a PATH search allocates, and a `malloc` lock held by another thread at the instant of the fork is held forever in the child. Now `execv` on an absolute path. Kept on its own merits: a red parallel run initially looked like evidence for this, and a deliberate re-test with `execvp` restored **passed**, so the change is correct but is *not* the fix for that flake. | reading the code; the attribution was disproven on purpose |
| N4 | **A spawn failure reported `Unknown error: -6 (os error -6)`** -- not an errno. Some call returned failure without setting one. It named neither which of the three syscalls failed nor anything an operator could act on. Now each call is named at the point it fails, a rc that is not an errno says so, and a transient failure is retried three times: a pty is a finite resource and a host can refuse one for a moment. **The original -6 remains unexplained**; it has not recurred in any run since. | one flaky parallel run |
| N5 | **A test pinned a value the protocol was about to gain.** `an_unknown_mode_byte_is_rejected_rather_than_guessed` asserted `try_from(3) == Err(3)`; 3 is now `Exec`. This is the roadmap's own "what was testing the step you just deleted" pattern, arriving from the other direction. | the suite went red |
| N6 | **The first oracle was wrong and the code was right.** Comparing against `ssh host ls` failed on all three hosts: through the exec channel `ls` resolves the operator's alias and finds `isatty(1)` true, so it printed three columns of coloured, icon-decorated names, while the oracle had neither and printed 235 plain lines. Two different commands, compared. The oracle is now `/bin/ls -1` on both sides -- absolute path defeats the alias, `-1` defeats the tty layout -- leaving the transport as the only variable. The aliased, coloured output is now asserted separately as a feature, because it is one. | the live run went red |

N6 is worth keeping in view: the instrument was wrong in the direction that
*looked* like a defect in the new code. A failing comparison is not evidence
until the comparison is known to hold everything but one thing constant.

### A timing-fragile client test, investigated

`monitor_packets_use_the_panel_gen_after_a_view_switch`
(`crates/multitop/tests/event_loop_e2e.rs:557`) failed once under
`cargo test --workspace` and passes every time on its own or as its own file.
Clean `HEAD` ran the whole workspace green, so it is not simply pre-existing --
under investigation with repeat runs rather than asserted either way.

The mechanism is legible even before the data: the test drives keys with fixed
sleeps (300 ms, 500 ms, 200 ms) and then requires the loop to resolve inside
5 s. The only client changes in this round are two match arms for
`Payload::Exec`, neither of which is reachable without an Exec frame arriving on
a *stats* stream -- which cannot happen here. What did change is the load: the
L1 suite forks 22 real shells on real ptys, and a wall-clock deadline shares a
machine with whatever else the workspace is running.

**Resolved as far as evidence allows:** three further `cargo test --workspace`
runs with these changes were green, so it is one failure in four, not a
reproducible break -- too thin to attribute in either direction. The test's
shape is fragile regardless of what triggered it, so L5 makes it wait on a
condition rather than on a clock, which removes the question instead of
answering it.

## 5b. Progress — client side complete (2026-08-29)

The client reads frames. `spawn_upgrade` has one exit that always reports
`AuxDone`, a bounded 30-second stall deadline, and install-and-retry for a host
with no agent. `Painter` takes bytes. `ssh::spawn_exec` is one spawn path for
local and remote.

**The old transport is deleted, not bypassed** -- `spawn_command`,
`ssh_command_tty`, `password_preamble`, both lock wrappers, `Spawned`,
`deliver_sudo_password` and the four sentinels. That was the point: two framing
models kept alive is how this happened.

### What the client side found

| # | Finding | How |
|---|---|---|
| N7 | **A regression already at HEAD.** `painted_states("a\r")` is `["a", ""]`, whose last element is empty -- and a pty ends every line `\r\n`. On the unmultiplexed transport the whole upgrade log collapsed to blank lines. The line-based reader before commit `95661fe` was safe by accident: `tokio`'s `Lines` strips the `\r` for you. Splitting by hand lost that and nothing noticed. | writing the byte-oriented painter |
| N8 | **The bootstrap cannot share stdin with the request.** Script then frame on one stdin returned `no readable exec request on stdin` from all three hosts: `sh` reading a script from a pipe reads ahead and the frame dies in its buffer. The bootstrap moved to argv -- carrying only this build's own constants, never the command or the password. | probe |
| N9 | **And `ssh` does not quote for you.** It concatenates its arguments and hands the string to the remote *login* shell, so `["sh", "-c", script]` lost every quote and `$(uname -m)` was evaluated too early. The quoting is ours. | probe |
| N10 | **S2 confirmed, after a probe that said the opposite.** A fake-`HOME` test showed the connection succeeding -- because **`ssh` expands `~` in `ControlPath` from the passwd entry, not `$HOME`**, so the path under test never changed. Isolated properly: a `ControlPath` that cannot be bound makes `ssh` print `unix_listener: cannot bind` and **the command never runs**, on every channel. The fallback is now performed rather than hoped for. | probe, calibrated |
| N11 | **My own length check was wrong.** It measured the `ControlPath` with `%C` unexpanded -- two characters against forty -- so it was 38 bytes too generous about a limit whose penalty is the whole connection failing. | a test with a deep home |
| N12 | **Six outcome tests failed on stderr handling I had dropped**: the bounded buffer, the `ssh`-chatter filter, the marker scan, the error colouring. All restored, and the agent now sieves stderr too, so a marker cannot cross the wire as text on either stream. | the suite |
| N13 | **The flake, explained and fixed.** Quitting mid-upgrade deliberately takes two presses, so a single-`q` test passed only when the run happened to have finished -- a race between a sleep and a subprocess. It also got slower: local upgrades now run under the interactive login shell the remote path always used. It waits on `finished_at` on disk now, like its sibling. | three failures in seven runs |

N10 and N11 are the same lesson twice in one afternoon: **a probe that shows no
defect has to be shown capable of showing one.** The first was blind because the
variable it changed was not the variable in play; the second was caught only
because a test happened to use a home deep enough to cross the line.

### Honest behaviour changes

* `signalled` now means *the agent's own child* was signalled. A signal that
  kills something nested arrives as an ordinary 128+N status, so that convention
  is decoded separately and the note names the signal -- "exited 137" alone sends
  an operator hunting a bug the OOM killer caused.
* An interactive login shell ignores `SIGTERM`, so `kill -TERM $$` in an
  `upgrade_cmd` no longer kills anything. That is what an interactive shell does,
  and it is the shell that makes an alias like `ud` resolve.
* Local upgrades pay rc-file startup they did not before. Milliseconds against an
  upgrade that takes minutes, and it buys local and remote behaving alike.

### Verified

Clippy clean, full workspace green, and the live suite green: 9 tests, 36
subtests, three hosts, three multiplexer states.

## 5c. Progress — L4 complete (2026-08-29)

`tests/tmux_harness.py` drives a real multitop in a real terminal over a private
tmux socket. Nine tests, green three runs consecutively at ~8s. Every wait is on
a condition; there is not a `time.sleep` in the assertions.

### One design correction to this plan

§5's L4 said the diag dump would give the panel's log "verbatim". It does not,
and it should not: `PanelDigest` is deliberately free of every field that could
hold a secret, because an upgrade log holds whatever the command printed. That
is a good property, so the harness uses two surfaces instead of weakening it --
`capture-pane` for *what is on screen*, the dump's `ring` count for *how many
lines the log holds*. The second is the duplication oracle and it is exact;
`capture-pane` sees an 80x24 viewport and cannot tell a line printed twice from
one that scrolled.

### What L4 found

| # | Finding | Kind |
|---|---|---|
| N14 | **The diagnostic handler wrote over the frame.** Every SIGUSR1/USR2 did `eprintln!("diag: wrote <path>")`, and stderr is the terminal the TUI holds in raw mode inside the alternate screen -- so each signal scribbled a wrapped path across the operator's panel. A tool whose purpose is to help when the display has stopped being readable was making it unreadable. **Five sites, and fixing one left four**: the loudest was the state tier the loop itself writes. All five now go through `diag::report`, which writes only when stderr is not a terminal. | product |
| N15 | The harness signalled **the user's own multitop** -- it resolved the pid by scanning the process table for the string "multitop" and found a release build that had been running 41 hours. It read that process's dumps and reported them as the test's, which is how a freshly started test saw a panel called `mockhost` with 41 hours of uptime. Now resolved by parentage from its own pane. | harness |
| N16 | **`ps` on `PATH` is not `ps`** on this machine: a replacement that rejects `-p` and answers with a usage error, which read as "the process is dead" and sent an afternoon chasing a signal handler that was working perfectly. Now `/bin/ps`, absolutely. | instrument |
| N17 | **A pid is not readiness.** The default disposition for SIGUSR2 is *terminate*, so a dump requested between the process existing and its handler being installed killed the app. That is what made the suite fail in bursts -- twelve, then none, then twelve -- and it read as a stale binary twice before it read as a race. `start()` now waits for the app to draw its first frame. | harness |
| N18 | **A stale binary tested code nobody wrote.** Three separate sessions today lost time to this. The harness now compares the binary's mtime against the newest source and **fails** rather than skipping -- skipping would let the suite go green while testing nothing. | harness |
| N19 | **Two assertions passed vacuously.** "not in_flight" and "state is not STARTED" are both true of a panel that never ran, and an earlier version of this file was green against exactly that: `state=NIL`, `ring=0`. Every such assertion now proves the run happened first. | test |
| N20 | **And the duplication ceiling was too loose to bite.** It was 8, "safely" above the four lines the command produces -- so injecting an exact duplicate of every line took the log to 8 and the test passed. Both bounds now come from a measured run, and the injection makes it fail. | test |

N14 is the one worth keeping: it is a defect no unit test could have found, in the
very tool built for the failure this whole change is about.

N16, N17, N18 and N19 are one lesson four times: **an instrument that shows no
defect has to be shown capable of showing one.** Twice today a probe was blind
because the variable it changed was not the variable in play.

### Proven red

* breaking the carriage-return collapse fails `test_05`;
* logging every line twice fails `test_04` with
  `8 not less than or equal to 5`;
* a run that does not start fails the floor with
  `only 0 line(s) logged`.

### Still to do

The gate-parity entry for the python tests, and the commit.

---

## 6. Order of work

1. L0 + L1 — protocol and agent exec, self-contained, no ssh.
2. Red-proof of L2 against `HEAD`; then the client reader.
3. L3, then L4 with the mux matrix.
4. L5, then L6 including S2 and S6.
5. Delete `ssh_command_tty`, the chunk reader, and the sentinel hunt.
6. Fold the findings into `roadmap.md`'s detection record; record the class in
   the relevant skill (`trace-the-boundary` — "a summary metric cannot
   distinguish nothing-to-do from silently-broken" generalises to "an unframed
   stream cannot distinguish end-of-record from end-of-nothing"); delete this
   file.
