# Changelog

All notable changes to `monitor`. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

This file starts at v0.47.0 — earlier history is in git (`git log`), and the
per-defect record is `docs/detection-record.md`, which is the more useful
document for anything before this point.

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
