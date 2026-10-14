# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history (`git log --oneline`) and in `docs/detection-record.md`. When an item
here is finished, delete it rather than ticking it off.

State on 2026-10-14: `main` at `8a5a105`, two commits ahead of `origin/main`
and unpushed -- the pre-push gate is red on goh 0.27.0's fallout (Phase 0).
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

### P0.3 — bare `python3` in the gate script

Structural step `no bare python in a hook or gate` (goh 6.2, `1c4be61`) is
red: `tools/gate.sh:21` and `:27` (`python3 tools/gate_lock.py acquire`, both
arms). It is the first structural red, so it hides every step after it. Fix:
`bash "$GOH/gates/python.sh" tools/gate_lock.py ...`, including the two
`trap` lines' release calls. Proof: the step is green; size XS.

### P0.4 — 39 child processes inherit stdin undeclared

Structural step `every child process names its stdin` (goh 3.4, `0c93f4d`) is
red: 39 calls (40 before `2492bdd` declared the one it touched), 22 in
`tools/`, 16 in `tests/`, 1 in `scripts/` -- e.g. `tools/ratchet_check.py:100`,
`tools/installed_freshness.py:174`, `tests/test_comprehensive_e2e.py:225`,
`scripts/repack_logos.py:74`. A tmux or exec e2e child that must read the
caller's terminal says `stdin=None`; the rest `stdin=subprocess.DEVNULL`.
Proof: the step is green with no seed (or a seed whose every entry carries a
reason); size S.

### P0.5 — the Linux lint step needs a container runtime nobody started

`tools/lint_linux.sh` (step 12) is red: `no container runtime answering
(colima start)` -- `colima` was not running. It fails closed and names the
fix, which is right; the gap is that pre-push depends on a machine service
the gate does not own. Proof: `colima start`, the step green; then decide
whether the step should start the runtime itself (it already names the
command) or stay a stated prerequisite in `DEVELOPMENT.md`. Size XS.

### P0.6 — scanner floors are unset

Structural's last step, `every scanner read its declared subject` (goh 4.3,
`831d686`), printed on 2026-10-14: `GOH_EMOJI_MIN_FILES=434
GOH_CONFLICT_MARKERS_MIN_FILES=435 GOH_FILE_LENGTH_MIN_FILES=385
GOH_SECRETS_MIN_FILES=434 GOH_SHELL_LINT_MIN_FILES=10
GOH_EARLY_EXIT_PIPE_MIN_FILES=17 GOH_DEAD_AFTER_EXEC_MIN_FILES=17
GOH_BARE_HOOK_INDEX_MIN_FILES=17 GOH_BARE_PYTHON_MIN_FILES=6
GOH_CHECKOUT_CREDENTIALS_MIN_FILES=2 GOH_MD_LINKS_MIN_LINKS=11
GOH_UNREAPED_SPAWN_MIN_FILES=249`. Paste each at or a little under its re-read
count, plus `GOH_SUBPROCESS_STDIN_MIN_CALLS` after P0.4. Proof: the
`[subject]` line is gone, and one floor raised one past its count turns the
step red naming the key (then put back). Size XS.

### P0.7 — the latency bench reads the machine's load, not the code

The pre-push run on `b60f25d` (2026-10-14, load ~35-65) failed step 9,
`tools/bench_check.py`, on all 3 tries: frame render 54976 ns against a
50000 ns limit; run alone 15 minutes earlier it read 33919 ns. Pre-push was
red on steps 1, 9 and 12. A threshold on wall-clock latency is a gate on
whoever else is running, and on this machine somebody always is. Calibrate
against a control measured in the same run (goh 5.3's `gate_profile.sh`
divides by a pinned control for this reason) or gate on a ratio. Proof: the
step stays green under a 2x synthetic load and goes red on a deliberate 2x
slowdown in the renderer. Size S.

### P0.1 — the `--serve` token is guessable

Measured 2026-10-12, not inferred. `serve_headless`
(`crates/multitop/src/main.rs`) always fills a missing `--serve-token`, but
with `DefaultHasher::new()` over `"<pid>-<unix nanos>"`: `DefaultHasher::new()`
is SipHash with FIXED keys, so the token is a deterministic function of two
values an attacker can bound (pid is small, start time is roughly known), and
it is 64 bits at most. On the default loopback bind this is moot; on the LAN
bind the phone needs (`--serve 0.0.0.0:8080`) it is the only thing between the
network and every host's metrics. Its own comment ("only when binding to
loopback ... For now, generate if not supplied") states a rule the code does not
implement.

Three defects in one place, fix together:

* **Token source.** At least 128 bits from the OS CSPRNG (`getrandom`, already
  in `Cargo.lock` transitively; make it a direct dependency). Never derived from
  time or pid.
* **Unauthenticated state unrepresentable.** `AppState.token` is
  `Option<String>` and `check_auth` returns `Ok(())` on `None`; the binary never
  reaches that path, so it exists only for tests and as a trap for the next
  caller. Make the token non-optional in `AppState` (a seam, house rule: the
  right state is the default); `index_ok_without_token` keeps its meaning
  because `/` serves static HTML with no data.
* **Constant-time compare.** `got == expected` in `check_auth`
  (`crates/multitop/src/server.rs`) is not. One crate (`subtle`) or a
  length-checked fold; not worth a hand-rolled oracle analysis.

Proof: a unit test that a generated token is at least 32 hex characters (today's
is 16, so it is red now) and comes through an RNG seam whose fake can be
injected, so the test proves the source, not just the length; a test that
`AppState` cannot be built without a token (compile-level, via the type);
`server_tests.rs` keeps `hosts_requires_token_when_set` green. Calibrate: revert
the generator once, watch the test go red. Risk: a CHANGELOG line must say the
old tokens were weak, so a user who scripted `--serve` on a LAN rotates.

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
