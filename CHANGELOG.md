# Changelog

All notable changes to `monitor`. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

This file starts at v0.47.0 — earlier history is in git (`git log`), and the
per-defect record is `docs/detection-record.md`, which is the more useful
document for anything before this point.

## v0.53.0 — four majors moved, each proven where it touches the system _(2026-10-08)_

- **axum 0.7 → 0.8.** Route captures are `/{host}` now. The old test asserted a
  404, which an unmatched route also returns, so a broken route passed it; the
  new one asserts each handler's own error body (red with a route broken on
  purpose). The history route had no test at all.
- **dirs 5 → 7.** Both builds resolve the same path. The real vault is
  `~/.config/multitop/vault.bin`, derived from the config file and never from
  dirs, so no vault can move; a test pins the default path against the
  platform rule built from `$HOME`, not against dirs.
- **mach2 0.4 → 0.7.** It provides `mach_host_self` and `mach_port_deallocate`,
  so the agent's hand-written declarations are gone; a new test requires real
  CPU and memory figures from the macOS sampler, which nothing tested before.
- **zstd 0.13 → 0.14** (licence MIT → BSD-3-Clause, like zstd-safe/zstd-sys
  already were). 0.14 decoded all 9 history files the 0.13 build wrote.
- **signal-hook stays 0.3**: crossterm 0.29 (its newest) needs 0.3, and two
  versions in one graph make one type two types. `diag.rs` keeps its own
  thread so a dump works with the runtime stuck.
- `cargo update` in the workspace and `fuzz/`: 10 duplicate crate pairs gone.
- Hooks: the stock gates_of_heck delegates (O39); `check_codesign` builds the
  binary it checks instead of reading `target/`; CI checkouts no longer leave
  the job token in git config; early-exit consumers on a pipe read to
  completion.

## v0.52.0 — the gate is green, and it can now prove the binary is this build _(2026-10-05)_

**Nothing you use changes.** This is the currency and freshness pass: 115 test
assertions rewritten through one helper, a test leak fixed, three dependency
majors moved, and a new gate that fails when the installed `multitop` is not the
binary this source builds. Every vault on disk still opens -- there are tests
that pin the exact bytes now, which is the only reason that sentence is a
measurement rather than a hope.

### Fixed
- **A test could leak an `sshd`.** `ssh_tests.rs` reaped its child *after* a
  `write_all` that could panic first, so a failing write skipped the reap and
  left the upload script -- which starts an `sshd` and holds a port -- running on
  the machine. The reap is now a guard's `Drop`, which unwinding runs. It has its
  own test, because the house gate that caught the original cannot tell a guard
  from a wrapper (see `docs/roadmap.md`).
- **`painted_states` carried a `#[must_use]` that duplicated the return type's.**
  `Iterator` is already `#[must_use]`, so the attribute was a second copy of a
  property `impl DoubleEndedIterator` already had. Removed, not renamed.

### Changed
- **115 `assert!(x.is_empty())` sites are now one helper.** `clippy 1.99.0`
  (2026-10-01) added `assert_is_empty`, and the house added
  `check_no_empty_assert`, which also counts the message-carrying form clippy is
  silent about. Rather than 115 type ascriptions, `crates/testassert` is one
  `assert_empty!` / `assert_not_empty!` pair shared by all three crates. The
  failure line names the expression, which is what identifies which of a dozen
  assertions failed -- and it requires no trait of the value, because
  `mlock::LockedMemory` has no `Debug` on purpose (it holds a vault key) and
  `ratatui`'s `Modifier` has no `len()`.
- **`tower` 0.4 -> 0.5, `sha2` 0.10 -> 0.11 (with `hkdf` and `argon2`), `zbus`
  4 -> 5.** Each was pinned below a version the graph already resolved, so the
  tree carried two majors of one crate and the house gate calls that FATAL. No
  type crossed any of the three boundaries, and none does now. The lockfile's
  duplicated crates: **31 -> 16**, and every one of the sixteen is transitive --
  `secret-service 5.1.0` alone holds what is left of the 0.10 crypto generation.
- **`tools/check_doc_paths.py` no longer fails on a shallow clone.** CI checks
  out with `fetch-depth: 1`, so the checker could not see any deletion and
  reported six docs that correctly record a removal as broken. `main` had been
  red on CI since 2026-10-01 for that reason alone. It now refuses with the
  three commands that fix it, and CI fetches the history it needs.

### Added
- **`tools/installed_freshness.py`** — fails when the installed binary is not
  what this source builds, comparing **bytes**. The installed copy was 64,608
  bytes away from a fresh build of the same commit, with the same version string,
  because the stable toolchain was replaced after it was installed; no gate could
  see it, and every end-to-end suite was evidence about the wrong artifact. Runs
  on the pre-push list, because its subject is the machine's install.
- **Known-answer tests for the vault's key derivation.** Argon2id, the HKDF
  signing sub-key, the Ed25519 verifying key and SHA-256 are pinned to bytes
  computed from the *previous* dependency generation. Every other vault test is a
  roundtrip, which cannot detect a KDF change that makes existing vaults
  unopenable -- which is the failure a major bump actually risks.

## v0.51.0 — the remote MCP command is not a login shell _(2026-09-30)_

**If you configured an `mcp` command that needs a binary from `~/.local/bin`,
it may have been failing with `command not found`.** The docs, the example config
and the doc comment all said the remote runs the command "in the host's login
shell". It does not.

Measured on `.33`, with a positive and a negative control:

```
zsh -l            -> PATH carries /home/ztomer/.local/bin
ssh host '<cmd>'  -> ~/.local/bin absent
```

`mcp::spawn::open` passes the command as a single `ssh` argument, so the remote
runs `$SHELL -c <command>` — not interactive, not a login shell. `~/.zprofile`,
where pipx adds its directory, is never read. Corrected in all three places the
claim appeared (`config.example.toml`, `README.md`, and the `types.rs` doc
comment) rather than the one the failing run happened to hit.

Also: `.158` is not Pi-hole — it is AdGuard Home with unbound, no `pihole` CLI
and no `pihole-FTL`. Its hostname is still `pihole`, which is what keeps the
label alive. Nothing in this repo called it Pi-hole, so nothing changed here.

## v0.50.0 — every theme was illegible on a light terminal _(2026-09-28)_

**If you run multitop in a light-coloured terminal, your colours have changed**,
and the alert banner has changed most. Dark terminals are unaffected: the theme
values you had are the values you still have.

### Added
- **Background detection.** The client asks the terminal what colour it is
  (OSC 11), then `COLORFGBG`, then falls back to the theme's own keybar
  background, and adapts all eight palettes for what it finds.
  `multitop --print-background` reports which, and from which source.
- **`tools/light_terminal_check.py`** — runs the built binary in a pty and
  measures every colour it actually drew, on a light and a dark background, with
  a negative control that fails if the two arms draw identical palettes.

### Fixed
- **The alert banner was unreadable on a light terminal, and 1380 tests could not
  have told you.** The breach and warning branches hard-coded colour escapes.
  On a dark terminal those literals *are* the theme's values, so nothing looked
  wrong, and a literal does not pass through the adaptation layer, so nothing
  could fix it. Measured on the real app, the alert red came out at **3.14:1**
  on white — the only one of eight rendered colours under the 4.5:1 bar. Both
  branches now ask the palette.
- **The detected background reached nothing.** For one commit the client probed
  the terminal, stored the answer, adapted a palette for it, and then rendered
  with a view built *before* the probe ran. Everything was green and the app was
  unchanged. `tools/check_no_hard_coded_colour.py` now fails any colour literal
  in either render crate, and the wiring test drives `boot_app_with` and reads
  the `App`'s own view — a test that calls the rebuild itself would have proved
  the function works and not that anything calls it.
- **Four themes were illegible on their own background, not just on light.**
  Nord's gray, purple and red; Gruvbox's red; Monokai's red. Corrected, and the
  contrast table is in the detection record.
- **The OSC 11 probe accepted its own echo as a reply.** Terminal.app echoes what
  the app writes, so the "response" was the query. The reply is now distinguished
  from the echo.

## v0.49.1 — the release script and the docs could not be trusted _(2026-09-28)_

Housekeeping only. No behaviour in the shipped binary changed; the pane-width
fix, the mux-noise fix and everything in v0.49.0 are what this builds on. What
changed is that two of the things that certify the build were themselves wrong.

### Fixed
- **`RELEASE.md` step one named a script deleted on 2026-09-14.** "Run the gates
  first -- `python3 scripts/local-ci.py`" was correct the day it was written and
  wrong from the day after, with no signal: the file was eleven days past the
  script's death and every release in that window would have started with
  `No such file or directory`. Nothing in the repo could see it, because no gate
  looked. A doc that no longer applies is worse than no doc, because it is
  trusted.
- **`DEVELOPMENT.md` had the same defect, worse.** It told a reader to run
  `python3 scripts/release.py v0.23.0 --cut` in a copy-pasteable block --
  thirteen minor versions stale, and naming the second of two release tools.
- **A second release tool was still shipped, carrying the bug it was written to
  prevent.** `scripts/release.py` (429 lines) rewrote the Homebrew formula with
  `re.sub(r'sha256 "[^"]*"', ..., no count)`, which sets **every** `sha256` line
  in the file -- tarball and both agent binaries -- to the tarball's digest, and
  it had no concept of the agent resources at all. Used, it would produce a
  formula that installs and then fails every agent checksum. Deleted rather than
  fixed: the last release was cut with `release.sh`, this predates the change
  that made the formula carry agents, and the previous release tool
  (`local-ci.py`, retired 2026-09-14) was deleted for exactly the same reason --
  a second copy of a list, drifting.
- **`tools/check_no_emoji.py` no longer exists** and the development docs still
  named it. It moved to the shared house gates; the table now says so and where.

### Added
- **`tools/check_doc_paths.py`** -- a gate that fails when a doc names a path
  that does not exist. It found the two defects above on the real tree, which is
  the only reason to believe it works; the `--self-test` cases were all
  insufficient on their own, and calibrating against the real defect caught three
  design flaws in the checker itself (a code-fence-only scope that missed the
  prose the defect was actually in; a deletion exemption so permissive it
  blessed the very sentence it was written to catch; and a per-line marker test
  that could not see a marker three lines up a wrapped sentence).
  Scope is deliberately a floor: a path in a command, a "Key Files" label, or
  multi-segment prose in an instructing doc must exist, and a path git records
  as deleted is accepted **in prose only** -- naming a removed file in a command
  is wrong however true the removal is. The oracle is `git log
  --diff-filter=D`, not a hand-kept allowlist, so a new deletion needs nobody to
  remember the docs.
- **The roadmap is a backlog again.** It had accumulated a hand-over notes
  section that was 24 days stale and carried two wrong file:line pointers; those
  invariants live in `DEVELOPMENT.md`, which is where a developer looks. The
  mobile companion entry claimed greenfield while the serving half shipped
  (`--serve`, `--serve-token`, 404 lines of `/api/*`), and named a WebSocket
  transport for work that is an HTTP client.

### Removed
- **`scripts/release.py`** and **`scripts/clean_slskd_history.py`** (see above,
  and: the latter is a tool for a different project's slskd music server and has
  no referent in this repository at all).

## v0.49.0 — the upgrade stops being drawn 80 columns wide _(2026-09-27)_

### Fixed
- **The upgrade is rendered at the wrong width.** Every remote channel was
  handed the live pane size — Monitor, Docker, Fetch, Ops — and the two exec
  callers that draw into that same pane passed `cols: 80, rows: 24`. Measured
  on a live host over a multiplexed connection, `stty size` asked *inside* a run
  on a 199-column terminal answered `24 80`: `apt` laid out its progress bar
  and its "After this operation" table for eighty columns and the pane drew the
  result into two hundred. `journalctl -f` and `tail -F` were following a
  screen nobody was looking at. The pane's size is now a required argument of
  every exec spawner, so the next one cannot be added without deciding.
- **`ssh`'s own multiplexer chatter could be quoted as a diagnosis.** The rule
  that recognises it lived in the upgrade reader alone. A plain file at the
  `ControlPath` — measured on OpenSSH 10.3p1 — makes `ssh` print two lines
  about the socket and carry on unmultiplexed, and the other three readers of
  that pipe had never seen them: a monitor panel could be told
  `ControlSocket … already exists, disabling multiplexing` was the reason it
  went dark, those two lines could clear a genuine missing-agent report, and a
  failed upload quoted a full local path. One definition, beside the socket it
  describes, asked by all four.
- **The remote host's own verdict is no longer swallowed.** The old rule also
  matched `Connection to %s closed by remote host.` on a bare
  `contains("connection to") && contains("closed")` — the host telling us it
  hung up, which is exactly the diagnosis the rule exists to protect. The
  vocabulary is now built from the format strings in the `ssh` binary itself,
  so every phrase matched is one only `ssh` prints.
- **Three timing suites that fired under load, which is what the coverage gate
  caught.** A `sleep` standing in for "the loop has read the key" passes when
  the machine is quiet and fails when it is not — 6 of 20 warm runs on
  unmodified `main` for one of them — and one collector gave each message 5
  seconds and stopped collecting *silently* when the window closed, so a
  truncated list failed a later assertion about output that was never missing.
  Every wait is now on a condition: the key is re-sent until the loop's own
  durable record shows it acted, which is correct rather than a retry because
  the app defers a confirm until the credential lookup lands, so an early press
  is queued rather than lost.
- **`tools/lint_linux.sh` had never run here.** `cp -r` cannot stat git's
  fsmonitor socket through a bind mount, and the three commands shared one `&&`
  line, so the failure skipped only the `cd` and cargo then reported
  `could not find Cargo.toml in /tmp` — a message naming neither the cause nor
  the directory it was in. Now copies with `tar --exclude`, and says so when
  the copy produces no manifest. `✓ linux clippy clean` for the first time on
  a macOS checkout.
- **`brew upgrade` was broken by v0.49.0, for about ten minutes, and the
  release script is why.** The Homebrew transform paired a digest with a url by
  LINE ORDER — it armed `pending` on a url line and wrote the sha onto the next
  sha line. That works only while every url in the formula names the same tag,
  and two agent urls had been left at v0.47.3 by a release four versions
  earlier. So on v0.49.0 it matched nothing, changed nothing, and every
  assertion it had still passed: `pending` had simply never been armed. The
  formula shipped 0.49.0 digests beside v0.47.3 urls, and `brew fetch` refused
  it. The final check that should have caught it only looked for the *tarball's*
  old tag, so a different old tag walked straight past. Fixed in
  `tools/bump_formula.py`, which rewrites one resource block at a time by name
  with every count asserted, and requires every tag anywhere in the formula to
  be the tag being released. `tests/test_bump_formula.py` runs the v0.49.0
  input through both transforms: the old one ships the breakage silently, the
  new one repairs it.
- **Two constants claimed to be the resize debounce.** `consts::RESIZE_DEBOUNCE`
  said 250 ms; `run::RESIZE_DEBOUNCE`, which is the one the loop reads, says
  30 ms. Nothing referenced the first, and a reader asking "how long until a
  resize takes effect" in the constants file would have been wrong by 8×. Both
  dead copies removed with their duplicate `RECONNECT_BACKOFF`.

### Added
- `exec_window_test`: the child's own `stty size` has to report the pane it is
  drawn into, at a wide pane, a narrow one, and two at once.
- `ui::layout` pins the geometry for 1–9 panels: the published size is never
  larger than any pane it is drawn into, and for a single panel it is exactly
  the pane. Includes the one case where the render floor publishes more rows
  than the shortest pane has, which is a trade rather than an oversight and is
  now named as one.
- `check_magic_numbers.py` gained a `window` rule. The gate could not see this
  class at all: a struct field initialised to a literal is not a `let`
  binding, and none of its six rules matched one.

### Verified
- The upgrade's pty tracks the pane: 198×29, 118×39, 78×29, 58×29 on
  terminals of 200, 120, 80 and 60 columns, over a shared `ControlMaster`
  connection to a real host — against a hard `24 80` at every size.
- The multiplexer is not the cause of anything else here, and that was checked
  before anything was changed: cold, warm and deliberately-unmultiplexed
  transports deliver identical bytes on all three hosts (`test_exec_live.py`,
  9 tests / 36 subtests), and `ssh` writes nothing to stderr in normal
  operation. Recorded in `docs/detection-record.md` along with one
  measurement that was wrong and the reason it was.

## v0.48.1 — "unchecked", never read as unhealthy _(2026-09-23)_

### Fixed
- **The Ops view's containers line.** ".33: 44 running, 33 healthy" read as
  eleven unhealthy containers; none were - eleven declared no Docker health
  check. The line now reads "N running, all healthy", or warns "N running ·
  H healthy · U unchecked" (a container nothing checks is a gap). Unhealthy
  containers are still named, each on its own line.

### Changed
- **Hooks.** The commit hook runs coverage and the ratchet through the
  gates_of_heck proven-step cache, so the push gate skips a step already
  proven on the identical tree; `tools/gate_lock.py` works in a git worktree.

## v0.48.0 — the Ops view _(2026-09-23)_

### Added
- **`p`: the Ops view** (servers ROADMAP 12.17b). Per host, what its
  `mcp_host` says that the agent does not stream: the last verdict of each
  health check (`health://latest`, never a suite run), cron jobs, container
  health (`docker.containers`), and the alerts its cron wrapper logged in the
  last 24 hours. Each section says why it is absent (a host with no Docker) or
  unreadable, and one failing never hides the rest. A container outside any
  compose project that exited 0 is a finished one-shot, not trouble - the
  routines follower's rule. `/` searches check, job and container names,
  alert subjects and a failed session's reason.
- **A stateless MCP client** (`mcp::client`, protocol 2026-07-28):
  `server/discover` first, every request carrying the version in `_meta`, no
  handshake; an older server is refused with the versions it offers. Each
  request has a deadline, and after a timeout or a closed pipe the session is
  not trusted again. The server is started by the host's login shell over the
  panel's ssh connection (`mcp` in `[[servers]]`); nothing listens.
- The poll holds one session per host, reopens a broken one on the next poll,
  and shows why under the last answer it had. It runs only while its panel is
  in the Ops view (`Tasks::retire_ops` after every key).

### Changed
- **No source file is over 500 lines any more.** `handle_key.rs`, `diag.rs`,
  `state.rs`, `keybar.rs` and `server.rs` were grandfathered over the cap by
  the LOC ratchet; each is now split by responsibility (`run/commands.rs`,
  `run/confirm_keys.rs`, `run/palette.rs`, `ui/keybar_confirm.rs`) or has its
  tests in a sibling `*_tests.rs`, and their ceilings are gone from
  `tools/loc_baseline.txt`.
- The palette's view words go through `execute_cmds` like the keys; its
  Docker and Fetch branches were copies of it.
- The narrow keybar sheds keys by NAME: its index list, coupled to the row's
  order by a comment, overflowed at 8 columns the moment a key was added.
- `Mode` lives in `src/mode.rs`.

## v0.47.3 — no lint suppression, unsafe scope as a gate, keyring 4 _(2026-09-21)_

### Changed
- **Every `#[expect]` and `#[allow]` is gone from the workspace; the house
  gate (`gates_of_heck` `check_no_allow.py`, policy of 2026-09-20) now
  refuses both.** About 150 sites across the three crates, each fixed at the
  finding rather than annotated: lossy `as` casts became `try_from` +
  `unwrap_or`, or go through the new `agent::conv` helpers (`unsigned`,
  `count`, `signed`, `single`, `whole_*`) which convert exactly via
  `num-traits`; exact float asserts compare with `partial_cmp`; functions
  whose body differs per platform are defined once per `cfg` (a synchronous
  Secure Enclave path returns `std::future::ready`, the Linux path stays
  `async`), so neither `unused_async` nor `manual_async_fn` needs quieting;
  `cfg_attr(..., allow(unused_mut))` became a per-`cfg` `let`; oversized
  functions were split into named stages (`run/handle_key.rs`,
  `run/event_loop.rs`, `app/apply.rs`, `config/load.rs`, `config_ui.rs`,
  `exec/pump.rs`, `render_layout.rs`, `password_actions.rs`, `run/spawn.rs`);
  bool clusters became small enums and structs (`Overlay`,
  `CredentialLookup`, `QuitFlags`, `SudoSigns`, `Sections`, `Handshake`);
  `vault::initialize` and `rebind_biometric` are synchronous (they never
  awaited). Every integration-test and bench crate root carries
  `#![cfg(test)]`, so clippy's `allow-expect-in-tests` applies to them as
  it always did to unit tests; test helpers that could fail return `Result`.
- **`unsafe_code` is no longer a workspace lint; its scope is a checker.**
  The agent and the vault are FFI by nature and carried a per-module
  `#[expect(unsafe_code)]` under a workspace-wide deny. `multitop` (no FFI)
  keeps `#![deny(unsafe_code)]` at both crate roots; for the other two,
  `tools/check_unsafe_scope.py` lists the eleven files that may contain
  `unsafe` with the reason each needs it, fails on any other file, and fails
  on a listed file that no longer has any (a ratchet both ways). Wired into
  pre-commit and CI; `check_gate_parity` keeps the two in step.
- **The host-wide `~/.cache/cargo-target` is no longer a root anything
  looks in.** It was the shared target dir until the 2026-09-20 build-dir
  layout; `build.rs` still searched it for an agent to embed (after the
  checkout's own `target/`, before auto-compilation), the `./multitop`
  wrapper and the tmux harness would run a binary from it, and
  `check_agent_version.py` compared against one -- which is how the
  empty-scope sweep found the checker failing on an empty tree with a
  0.47.2 agent still parked there. Every site now resolves only this
  checkout's target dir (`cargo metadata`, `CARGO_TARGET_DIR`, `target/`).
- **`fuzz/Cargo.lock` is checked `--locked`.** The pre-commit fuzz check
  ran an unlocked `cargo check`, which rewrote the lockfile during the
  0.47.3 commit and left the tree dirty behind a green hook. A stale fuzz
  lockfile is now a red step naming its fix (proven red, then green).
- **The bench gate is best-of-three.** One sample under load (57 us for a
  28 us frame render, right after the fuzz ASan builds) refused a push;
  a latency gate measures what the code can do, and a real regression is
  slow every time.
- **The enclave rebind is macOS-only at its call site.** The pre-push
  container lint (`tools/lint_linux.sh`) caught the non-macOS stub as
  `unused_self`; the repair now takes the vault by value and the one call
  is `cfg`'d, so there is no stub and no `mut` a platform never uses.
- **`release.sh`'s tap bump ran for the first time since it was written
  and failed twice on its environment, not the formula.** The Python
  transform sat inside `python3 -c "..."`, so the `sha256 "..."` regexes
  lost their double quotes to bash and the agent-sha line was never
  matched; it is a quoted heredoc now. Then `ruby -c` rejected the
  formula's em dash as an invalid multibyte character because the tool
  shell had no locale; it runs under `LC_ALL=C.UTF-8`. The tag, release
  and assets had already landed, so the bump was finished by hand-running
  the fixed tail; the next release runs end to end.
- **Vault moves from `keyring` 3 to 4, matching `multitop`.**
  The workspace carried two majors of one crate: `multitop` on 4 since the
  August dependency refresh, `multitop-vault` still on 3 with the old
  `apple-native` / `sync-secret-service` feature flags (names that no longer
  exist upstream; the platform stores are built in and Linux secret-service
  selection is the default). The vault now declares
  `keyring = { version = "4", default-features = true }`, identical to
  `multitop`, and both lockfiles drop the second graph: `keyring` 3.6.3,
  `dbus` / `dbus-secret-service` / `libdbus-sys`, `security-framework`
  2.11.1 and `core-foundation` 0.9.4. The keychain calls in use
  (`Entry::new`, `get_password`, `set_password`) are identical across the
  bump and the vault file format is untouched — existing vaults open as-is.

### Fixed
- **Tests name this build's agent through a seam instead of guessing where
  it lives.** `spawn_exec` / `spawn_local_agent` used to look for a
  `multitop-agent` beside or above the running executable — a guess that
  held for exactly one build layout (`target/debug/deps/<test>` under
  `target/debug/<bin>`) and broke silently on 2026-09-20 when cargo's
  intermediates moved to a separate build-dir: ten integration-test crates
  spawned nothing (`NotFound`, reported as if the agent had not been built)
  and one of them parked forever, which is what a "10-minute commit" was.
  The resolver now takes `MULTITOP_AGENT_EXE`, then this process when it is
  a `multitop` build (`--agent`), then `multitop-agent` on `PATH`; the tests
  set the seam to `CARGO_BIN_EXE_multitop` (`tests/common`), which cargo
  hands every integration test in every layout. `local_agent_test`'s two
  tests are un-ignored — they had never run and had rotted past the `Hello`
  frame the stream opens with.
- **`test_concurrent_upgrade_generations_isolated` cannot hang.** It looped
  on the channel until both generations reported *success*, holding its own
  sender, so a failed upgrade left it waiting forever. It records every
  outcome and asserts on both (proven: fails in 0.00s when the agent cannot
  spawn).

## v0.47.2 — upgrade stderr through the painter, finish routing shared _(2026-09-13)_

### Fixed
- **Update screen showed progress text in duplicate.**
  `tasks/upgrade.rs` sent stdout through the `Painter` (append vs repaint by
  cursor movement) but buffered stderr per line in `Report::errbuf` and flushed
  it as plain appends at end of run. `apt` writes its progress display to
  stderr, so every tick appended one more near-identical line — and out of
  order, after all stdout. Stderr now feeds the same shared `Painter` (one
  cursor: the remote pty has only one), styled red and routed with `paint_msg`
  exactly like `exec_runner`. `keep_stderr`, `Report::errbuf` and
  `MAX_UPGRADE_ERR_LINES` are gone; the ring cap bounds memory. `sudo_help`
  detection, connection-noise filtering and blank-stderr dropping moved with
  the bytes, pinned by e2e tests that print progress to stderr.
- **An unterminated final line was appended twice on the exec path.**
  `exec_runner::drain_stdout` sent the painter's `finish()` as a bare
  `AuxLine`, ignoring `back`/`erase_below` — a tool killed mid-write left its
  last line on screen via the open-line paint and got a second copy at close.
  The tail now routes through `paint_msg` like every other paint, and
  `drain_stdout` is generic over the byte stream so the case is tested with
  synthetic framed packets instead of a live run.

## v0.47.1 — unbreak Linux CI, keep chunked markers, fix upgrade hangs & PTY sizing _(2026-09-09)_

### Fixed
- **Lingering agent processes and stuck upgrade view on remote hosts.**
  Commands like `us;ud` that launch Docker containers or subshells leave open
  descriptors (PTY slave, stderr) inherited across forks. The agent's `pump()`
  loop previously waited indefinitely for descriptor EOF without checking if
  the parent shell had exited. `pump()` now actively polls `try_wait(child.pid)`,
  drains queued bytes, closes descriptors, and emits `ExecFrame::Exit`
  promptly when the shell process completes.
- **Double lines on `update-local` and live-rendered tables.**
  `run_upgrade` passed `cols: 0, rows: 0` in `ExecFrame::Request`, which clamped
  to `ws_col: 1, ws_row: 1` in the PTY. Terminal tools like `rich.live.Live`
  detected a 1-column terminal and wrapped character-by-character, breaking
  ANSI cursor rewinds (`\x1b[1A`) and duplicating every row in the log. PTY
  dimensions now default to standard 80×24 when 0, and `run_upgrade` sets 80×24
  explicitly.
- **Interactive pager hangs and unframed reads during upgradable package check.**
  On Ubuntu 24.04/26.04, `apt list --upgradable` run inside a PTY spawned
  `/usr/bin/pager` (`less`), hanging forever waiting for input. The command wrapper
  now exports `PAGER=cat`, and `spawn_upgradable_check` sets `DEBIAN_FRONTEND=noninteractive`
  and properly decodes binary framed packets rather than attempting raw stdout reads.
- **Linux CI compiled nothing since the lint opt-in.** Opting the agent into
  the workspace lints tripped `unsafe_code = "deny"` on the pre-existing
  Linux-only `malloc_trim` block, which had never been named in the
  module-attribute policy — so clippy, test, coverage and fuzz all failed to
  build. `monitor` (and the already-attributed but unlisted `proc_disk`) join
  the FFI module list.
- **A marker split across two reads after a progress bar was lost.** The
  sieve's hold-back length budget measured the whole partial instead of the
  post-`\r` state, so `…\r__multitop_sudo_failed__` arriving in pieces was
  flushed as output. The budget now measures the carriage-return state, with
  deterministic `sieve_test.rs` regression tests.

## v0.47.0 — two advisories, a lint policy, and a crate that was linting nothing _(2026-09-07)_

### Security
- **RUSTSEC-2026-0253: use-after-free in `lru`'s `LruCache::pop()`.** Fixed by
  moving off the affected version. This was sitting in the lockfile.
- **The yanked `chacha20` 0.10.1 → 0.10.2.**

Both were found by wiring `cargo audit` as a gate rather than running it by
memory, and by configuring it to actually bite: `cargo audit` exits 0 on
`unmaintained` and `unsound` findings, so a gate without
`[output] deny = ["warnings", "unmaintained", "unsound", "yanked"]` passes over
precisely the class it was added to police.

### Fixed
- **`crates/agent` was subject to NO lint policy and nobody could tell.** The
  workspace had declared `pedantic` and `nursery` from the start; a member crate
  applies that with `[lints] workspace = true`, and this one — the biggest, the
  one uploaded to every monitored host — never said so. It had accumulated **254
  findings** that every green gate had agreed were absent. There is no warning
  for this anywhere: cargo does not mention the omission and `clippy -D warnings`
  passes, because the crate genuinely has no findings at the levels it is subject
  to. Now gated by `gates_of_heck/checks/check_lints_optin.py`.
- **`make clippy-targets`: every SHIPPED target is linted, not just this
  machine's.** Clippy reports on the cfg it compiled for and is silent about
  every other, so a crate that is half `cfg(target_os = ...)` had a Mac checking
  one half and the ubuntu runner checking the other — neither failing on the
  other's code, and nothing comparing them. That found ten findings in `src` the
  host cannot see, plus two `#[expect]`s that were UNFULFILLED for linux-musl,
  which under `-D warnings` is an ERROR: **the agent's build was broken for the
  platform it ships to while every gate was green.** A missing target std is now
  a hard failure naming the `rustup target add`, never a skip.
- **A 619-line test file was bounded by nothing.**
  `crates/multitop/tests/event_loop_e2e.rs` was named in `.gatesrc`'s
  `GOH_LINE_EXCLUDE` (so the 500-line cap did not apply) and absent from
  `tools/loc_baseline.txt` (whose sweep is `crates/*/src` only), so neither the
  cap nor the ratchet applied and it could grow without limit. Worse, the
  baseline's own header asserted the opposite, so the document read as though the
  hole did not exist. Split into a directory target; the exemption is gone; the
  pairing is now enforced by `check_exclusion_has_ceiling.py`, run from
  `tools/ratchet_check.py` so the hook, CI and local-ci all get it.

### Changed
- **`clippy::pedantic` + `nursery` adopted across the workspace**, in staged
  passes with the count recorded at each: the machine-applicable fixes, then the
  judgement calls, then the libc FFI casts in `sys.rs` scoped narrowly rather
  than blanket-allowed. `lib.rs` was split because the ratchet was right about it.
- **Two forbidden `#[allow]`s deleted** and the wire encoder tightened. `#[allow]`
  is banned here in favour of `#[expect]`, which errors when its lint stops
  firing and therefore cannot rot.

### Removed
- **Three unused dependencies**: `tower-http` (multitop), `rpassword` and
  `core-foundation` (multitop-vault). Each verified by REMOVAL and re-checked
  against all three shipped targets, not by grepping for the crate ident — that
  grep has exactly the blind spot that makes the tool wrong on `serde_bytes`,
  which is reached only through `#[serde(with = "...")]` and is now recorded as
  an ignore with its reason.
