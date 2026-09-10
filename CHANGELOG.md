# Changelog

All notable changes to `monitor`. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

This file starts at v0.47.0 — earlier history is in git (`git log`), and the
per-defect record is `docs/detection-record.md`, which is the more useful
document for anything before this point.

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
