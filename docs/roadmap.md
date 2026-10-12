# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history (`git log --oneline`) and in `docs/detection-record.md`. When an item
here is finished, delete it rather than ticking it off.

State on 2026-10-12: `main` at v0.54.0, CI green on the release commit, tree
clean, no stale branches or worktrees. One shipped defect is open (P0.1), found
while verifying this file's claims against the code. The version of this file
before the 2026-10-12 rephase is `git show ed9e685:docs/roadmap.md`.

Every item names its proof: the test or gate that goes red until it is done.

## Phase 0 — shipped defects (size S, one session)

Goal: nothing the binary ships today is wrong. Exit: P0.1 and P0.2 closed, gate
green, released.

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

## Phase 1 — Android companion (size L, three to five sessions)

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

## Phase 2 — parked on a trigger: post-quantum KEM (size M when it fires)

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

The 2026-10-10 consolidation's table (opencode loop goals, two Antigravity
conversations, branch `ci-persist`, old items 3 and 4) is at
`git show ed9e685:docs/roadmap.md`. This pass re-checked every store:

| Source | Disposition |
|--------|-------------|
| Claude Code CLI, slug `-Users-ztomer-Projects-monitor` | No transcripts. One `*.desktop-released.json` marker (session deleted 2026-10-11): residue, removed. `memory/` kept: one project fact (stale `multitop-agent` on the .33 test box) and two feedback entries that read as procedures, flagged for a skill. |
| Claude desktop sessions | None for this repo (listed with archived, 2026-10-12). |
| opencode db | Zero sessions; the `project` row for this path remains (coordinator's to delete). |
| opencode loop dir `.opencode/opencode-loop/` | Gone since 2026-10-10; `.opencode/` is git-ignored, nothing tracked. |
| Antigravity (app + IDE + backup) | No conversation names this repo or `multitop` any more. |
| Gemini CLI `~/.gemini/tmp/` | No dir claims this repo. |
| The Projects-root cross-project indexes (ROADMAP-consolidated, RUSTIFICATION_ROADMAP) | No monitor or multitop item in either. |
| git branches and worktrees | `main` only, one worktree, clean. |
| Verification of this file's own claims | Found P0.1 (guessable token), P0.2 (doc drift), the APNs/FCM error in A3, a stale `lib.rs` line number; all fixed or carried above. |

## Where the invariants live

The hand-over notes — `Hello` first/valid/single, the embedded agent matching the
workspace version, the stable ad-hoc signature, the vault's fallback, "build
with `./build.sh`" — are in
[DEVELOPMENT.md](../DEVELOPMENT.md#hello--agent-embedding). The per-defect
history is [`detection-record.md`](detection-record.md); general process rules
are in the house skills.
