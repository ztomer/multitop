# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history (`git log --oneline`) and in `docs/detection-record.md`. When an item
here is finished, delete it rather than ticking it off.

State on 2026-10-14: `main` at `7a316b3`, **one commit ahead of `origin/main` and
unpushed** — P0.1 is committed and green locally but has never left the machine;
`fuzz/Cargo.lock` is modified in the tree because adding `getrandom` and `subtle`
as direct deps changed it, and the commit that carries it has not been made. The
exact recovery is under *P0.1* below; read it before touching the tree. Everything
else in Phase 0 is landed and released (v0.55.0, `2a5121c`).
CI was last green on `f5be29f` (2026-10-12). The goh 0.27.0 hooks could not be
committed as installed: goh refuses the older stock pre-commit, and this repo's
`tools/check_gate_parity.py` refused the new one (it read the hook's text for
`tools/gate.sh`); `2492bdd` taught it the house hand-off, then `8a5a105`
committed the hooks. Narrative of the 2026-10-12 pass:
`git show f5be29f:docs/roadmap.md`.

Every item names its proof: the test or gate that goes red until it is done.

## Phase 0 — wire in goh 0.27.0, then the shipped defects (size S-M, one or two sessions)

Goal: `tools/gate.sh --full` green at HEAD and pushed, and nothing the binary
ships today is wrong. Exit: P0.3-P0.7 closed and pushed first (nothing pushes
until they land), then P0.1 and P0.2 closed, gate green, released.

Measured 2026-10-14 on `2492bdd`'s tree: the full gate was stopped at 15 min
(the brief's bound), at load 138-164 from other sessions, while on step 8 of
12. Steps 2-7 were green; steps 8-12 were then run one by one: fuzz 489 s,
bench 233 s, coverage 255 s (96.13%, floor 95), installed-freshness 30 s, all
green, and `tools/lint_linux.sh` red. Structural (step 1) was read with the
new hook in place and each red masked in a scratch tree in turn.

### ~~P0.3 — bare `python3` in the gate script~~ CLOSED `3a43ffe`: the lock
helper runs on goh's pinned interpreter (`bash "$GOH/gates/python.sh"`), both
arms of `tools/gate.sh:21,27` and the two trap releases.

### ~~P0.5 — the Linux lint step needs a container runtime nobody started~~
CLOSED by starting it, not by changing the step: `colima start` and
`tools/lint_linux.sh` runs green — "linux clippy clean" on the whole workspace
at `linux/arm64`, which is the only place an OS-only `#[expect]` is visible.

Whether the GATE should start its own runtime is still the open question, and it
is deliberately not answered here. `DEVELOPMENT.md` states the prerequisite and
the step fails closed naming the command, which is right: a pre-push gate that
boots a VM on its own is a gate that can surprise the machine it runs on. What
this round added is the evidence that the step is not a formality — it is the
only Linux-side check, and it passed on a tree whose every other gate had
already passed.

### ~~P0.4 — 39 child processes inherit stdin undeclared~~ CLOSED: every child
process names its stdin across `tools/`, `tests/` and `scripts/`, with
`GOH_SUBPROCESS_STDIN_MIN_CALLS=42` set at its measured count and proved red at
43. No seed: the sweep declared the calls, it did not suppress them.

### ~~P0.6 — scanner floors are unset~~ CLOSED: all 13 floors are in `.gatesrc`
at their measured counts, and `GOH_SUBPROCESS_STDIN_MIN_CALLS` one past its
count turns the step red naming the key.

### ~~P0.7 — the latency bench reads the machine's load, not the code~~ CLOSED.
The bench gates INSTRUCTIONS RETIRED, not nanoseconds: `client_bench
--only=decode|render --iters=N` runs one loop under `/usr/bin/time -l`, and the
count is taken at N and 2N so process start-up and set-up cancel. Measured
across a 4x busy-loop load the per-frame count moved 0.07%, against tens of
percent for wall time. Budgets are ~1.7x the measurement; a deliberate 2x
slowdown inside `render` is over both, and was caught by exactly that while the
gate was being finished.

### P0.1 — the `--serve` token is guessable — CLOSED locally at `7a316b3`, NOT PUSHED
The token was a `DefaultHasher` over `"<pid>-<unix nanos>"` — SipHash with FIXED keys, so
a deterministic function of two values an attacker bounds, and 64 bits. Its own comment
claimed a rule it did not implement ("only when binding to loopback ... For now, generate
if not supplied"). Three defects, fixed together in one place:

- **`crates/multitop/src/serve_token.rs` owns both halves now.** `generate()` is 256 bits
  from the OS CSPRNG (`getrandom`, promoted from a transitive dep to a declared one);
  `matches()` compares in constant time (`subtle`). Never derived from time or pid.
- **`AppState::token` is a `String`, not an `Option`.** `check_auth` returned `Ok(())` on
  `None`, so "no token configured" silently meant "no authentication" — a path the binary
  never took, which is exactly why it read as dead code while being an open door with a
  test-shaped handle. `GET /` still needs no token (static HTML, no data); everything else
  does. The CLI path collapses `Option` to `String` at the point of generation, which also
  deleted a `(no token)` branch that was already unreachable.
- **Three tests were reading the trap as a feature.** `test_api_snapshot_and_history` passed
  `None` and sent no `Authorization`, as did two in `server_tests.rs`; they were asserting
  200/503/404 through a router that authenticated nobody. They authenticate now, and their
  subjects are handlers and decoders again.

Calibration found a hole in the new test worth recording: an empty presented token is spelled
`"Bearer "` or `""`, **not** a bare `"Bearer"` — that one does not strip, so `got` stays the
literal `"Bearer"` and a mutation accepting an empty credential is never reached. The arm
sends both.

**Getting `7a316b3` off this machine.** The commit is green but unpushed; `fuzz/Cargo.lock` is
modified in the tree because promoting `getrandom` and `subtle` to direct deps changed it.
Exactly this, and nothing else:

```bash
cd ~/Projects/monitor
cargo check --manifest-path fuzz/Cargo.toml --all-targets
git add fuzz/Cargo.lock
git commit -m "lock: fuzz manifest follows the new getrandom/subtle deps"
git push origin main
```

Two gotchas on that push, both paid for already:

* Run `./install.sh` **first**, or `installed_freshness` goes red on the binary the commit
  changed and the push is refused for the wrong reason.
* Run `bash tools/gate.sh --full` on the working tree first, so step 11 is recorded as proven.
  Skipped, the gate does a cold release build inside its snapshot and blows the 900 s timeout —
  which reads as a hang, not as a missing step.

### P0.2 — docs that describe a different server

* `README.md`'s documentation table describes this file as "light-terminal
  palettes, the Android companion, post-quantum KEM"; light-terminal palettes
  shipped in v0.50.0 and were dropped from here. Change the row.
* The previous roadmap listed an `/api/mtp` route and said a bare `--serve` has
  no auth. Neither is true (`router()` has five `/api/*` routes, no `mtp`; the
  binary always sets a token). Corrected below in this rewrite.

Proof: `grep -n "light-terminal" README.md` returns nothing;
`tools/check_doc_paths.py` stays green.

## Phase 1 — gate drift (advisory today, size S)

Goal: the gate costs what it must and runs nothing twice. Depends on Phase 0.

1. **G1 — every commit runs the push gate's slow half.** The pre-commit hook
   (`tools/gate.sh --staged` -> `tools/commit_gates.sh`) runs clippy, the
   end-to-end suites, the fuzz build and workspace coverage: the hooks commit
   `8a5a105` took 4 min 42 s, `2492bdd` over 10 min, at load ~140. A commit
   gate that slow is one people learn to batch around. Decide what must block
   a COMMIT (fast, staged-scope) versus a PUSH, and measure both at a stated
   load. Proof: `tools/check_gate_parity.py` still green, and the staged gate
   under a minute on a quiet box.
2. **G2 — a plain `cargo test --workspace` beside the gate's.** goh Downstream
   (monitor): `.gatesrc` step 5 and `.github/workflows/ci.yml:132` run
   `cargo test --workspace --all-features` while coverage already runs every
   test target. (The same bullet's retired `GOH_NO_NATIVE` is already gone,
   since `cc2906a`.) Proof: one test run per gate, a failing test still red.
3. **G3 — the tap bump goes around the tap's gate.** goh 6.8 (`bfac28b`):
   `release.sh` PUTs `Formula/multitop.rb` through the GitHub API, so the
   tap's gate never runs on it. The house `tap_bump.sh` rewrites ONE url and
   sha256; this formula carries three (the source tarball and two agent
   binaries, `tools/bump_formula.py`). Blocked on goh: `tap_bump.sh` taking
   several resources (cross-repo ask), then `release.sh` calls it. Proof: a
   release dry run shows the tap gate step.

## Phase 2 — Android companion (size L, three to five sessions)

Goal: a phone shows which hosts are up, what the last upgrade did, and what is
degraded, without a terminal. Exit: the app reads a LAN `--serve` over an
authenticated, hardened API, and a transition push reaches the phone. Depends
on P0.1 (a second client must not be built on a guessable token).

The serving half ships and is tested; this is a client. Transport is **HTTP**
over the existing API, not a `WebSocket`:

* `multitop --serve [<ADDR>]` and `--serve-token <TOKEN>`
  (`crates/multitop/src/main.rs`); `--serve` binds **loopback by default**
  (`serve_socket_addr`: `:8080` means `127.0.0.1:8080`). Keep it that way.
* `crates/multitop/src/server.rs`: bearer `check_auth`, static `index`, and
  `/api/hosts`, `/api/health`, `/api/snapshot/{host}`, `/api/history/{host}`,
  `/api/mtop`; `spawn_collectors` reuses the agent's `stream::connect` and
  `Payload::Hello` validation.

In the order it blocks:

1. **A1 — LAN-bind hardening, after P0.1.** The questions to settle before a
   second client exists to lose the token, each answered by a test:
   * The token is printed to stderr at startup (`token=<t>` and a ready `curl`
     line). Right for an interactive start, wrong under a service manager whose
     stderr is a log. Decide: print once to the tty only, or write it to a
     `0600` file. Proof: a test that a non-tty stderr carries no token.
   * Whether the token reaches an access log or an error body. Proof: grep the
     axum error paths; a test that a 401 body does not echo the header.
   * Whether a non-loopback bind should require an explicit `--serve-token`
     (no generated token for a LAN bind) or TLS. Decide on UX grounds
     (house tiebreaker: best user experience), state the reason here.
2. **A2 — a read-only Android app** over `/api/*`. A glance surface, not a
   terminal emulator and not the TUI's key handling in a smaller window.
   Open decision before code: same repo or its own. A second language in this
   repo inherits none of its gates (`polyglot-gate-parity`): it needs its own
   lint, tests, coverage floor and a `check_gate_parity.py` entry from the
   first commit. Proof: the app's own gate runs in CI; an end-to-end test
   against a `--serve` started by the test.
3. **A3 — push.** Android push is **FCM** (or UnifiedPush/ntfy to avoid a
   Google dependency), not APNs as an older entry of this file said; APNs is
   Apple's and would only apply to an iOS client. Needs the app, so it is third.
   The trigger is the server noticing a *transition* (host down, upgrade failed,
   health degraded), which the current request model does not have, so this is
   new server code. Proof: a server test that a synthetic transition emits
   exactly one notification and a steady state emits none.

The `repack`/`convert` logo pipeline (`scripts/repack_logos.py`,
`scripts/convert_logos.py`, over `crates/multitop/data/logos.bin.zst`) already
renders app icons, so an icon is not a blocker.

## Phase 3 — parked on a trigger: post-quantum KEM (size M when it fires)

Sequenced against its trigger, not a date. For a device-local vault file read by
a process that already holds the key, harvest-now-decrypt-later is not the
binding risk. It becomes one when:

* the vault leaves the device (a synced file, a backup, a shared host), or
* the KEM protects something with a long confidentiality life.

Neither has happened (2026-10-12). Exit criterion when it fires: a fourth
wrapper opens a vault in a round-trip test, and a vault written before it still
opens.

**The shape, already prepared.** A vault key is wrapped once per authenticator,
tagged by a discriminant (`crates/vault/src/crypto/wrapper.rs`:
`SecureEnclave = 0x01`, `Tpm2 = 0x02`, `Argon2id = 0x03`). A KEM is a fourth
`WrapperType` variant whose wrapping key comes from a KEM shared secret, added
beside the others; not a migration. The content cipher (AES-256-GCM with an
HKDF-derived sub-key, `crates/vault/src/crypto/primitives.rs`) is untouched.

It must not:

* touch the hardware-wrapped variants — a Secure Enclave or TPM key is
  non-exportable by construction, and a KEM in front buys nothing;
* bump `min_proto_version` — negotiation is a range check (`is_compatible`,
  `crates/agent/src/proto/mod.rs`), and raising the floor breaks every
  deployed client at once;
* be hand-rolled — a maintained crate, as with `argon2` and `aes-gcm`.

The agent transport is a separate, weaker case (a local-network control
channel); bundle it only if the vault trigger fires.

## Standing rulings (Do NOT)

* **Do not chase the remaining duplicate crates to zero.** All transitive;
  `secret-service 5.1.0` (via `keyring`) holds the 0.10 crypto generation.
* **Do not land dependency currency as a plain gate.** It is a ratchet
  (`GOH_DEPS_RATCHET=tools/deps_ratchet.txt`). Its one entry, `signal-hook` 0.3,
  waits for `crossterm` (0.29.0 is still the newest release, measured
  2026-10-12); delete the entry the commit `crossterm` moves.
* **Do not put back a minimum Rust version without a job that builds on it.**
  `tools/check_msrv_claim.py` fails a claim nothing exercises.
* **Do not bundle a lockfile refresh with a major move.** Its own commit.
* **Do not "fix" the empty coverage part by deleting the agent's bin target.**
  The `⚠ ... valid-but-EMPTY lcov part` line is the coverage gate saying
  `main.rs` is regex-excluded; the `mcp` mode lives in the library and is
  driven for real by `ops_view_e2e.rs`.
* **Do not move Android's auth hardening behind the app** (P0.1 and A1 before
  A2).
* **Do not read terminal escapes anywhere but `multitop_agent::escape`.**
  `tools/check_one_escape_reader.py` fails a fifth reader.
* **Do not draw ratatui chrome in anything but a `Chrome` role, nor app text in
  anything but the current palette.** `chrome_contrast_test` and
  `tools/check_no_hard_coded_colour.py` fail the commit otherwise.

## Dropped, and why

| Dropped | Why |
|---------|-----|
| **Plugin SDK (WASM)** | Custom `exec` panels (`custom_command` in `crates/multitop/src/config/types.rs`) cover it without a sandbox to get wrong. Revisit if panels ever arrive from somewhere other than the user's own config. |
| **A light variant per theme** | Superseded by background detection: one mechanism for all eight themes, correct on an image background. Shipped 2026-09-28. |
| **Adapting the agent's tty path** | The seam exists (`emit_fetch`/`emit_docker` take a `&PaletteView`), but `crates/agent/src/lib.rs` builds it with `for_theme`, the identity, because a process on the remote host cannot measure the user's terminal background. Worth doing when a second consumer that knows the client's background appears. |

## Sources consolidated (2026-10-12)

Every agent store for this repo was re-checked on 2026-10-12; nothing in them
is open beyond the items above. The table is `git show f5be29f:docs/roadmap.md`.

## Where the invariants live

The hand-over notes — `Hello` first/valid/single, the embedded agent matching the
workspace version, the stable ad-hoc signature, the vault's fallback, "build
with `./build.sh`" — are in
[DEVELOPMENT.md](../DEVELOPMENT.md#hello--agent-embedding). The per-defect
history is [`detection-record.md`](detection-record.md); general process rules
are in the house skills.
