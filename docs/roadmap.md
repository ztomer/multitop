# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history (`git log --oneline`) and in `docs/detection-record.md`. When an item
here is finished, delete it rather than ticking it off.

## Now

**The currency and freshness pass, assessed 2026-10-03 and not yet started.** The
light-terminal palette finished on 2026-09-28 and the section that described it
has been deleted rather than left as a graveyard; the commits `e556d63`,
`a86aad3`, `f7cfb0a` and `95b8168` are the record, and
`docs/detection-record.md` carries the measurements.

### The shape of it, in one paragraph

This repo is in good shape and the assessment says so with numbers, not
politeness: all eleven `tools/check_*.py` checkers pass behind their own
self-tests, `cargo audit --deny warnings --deny unmaintained --deny unsound
--deny yanked` is clean, coverage measured 96.26% against a 95% floor this
afternoon, the line ratchet is green, and the whole 1380-test suite runs green
under the current stable. There is no rot in the source tree. The two things
below are the only *red*, and **both surfaced today rather than having been
hiding**: one is a pair of lints that did not exist in the compiler before
2026-10-01, and the other is a house gate that was itself written today. So this
is a two-item unblock plus a short dependency pass, not a rescue.

### Item 1 — the gate is red, and nothing can be committed or pushed until it is not

Two independent reds, both verified by running the exact gate command rather than
by reading the logs.

**1a. `clippy::assert_is_empty` × 41 and `clippy::double_must_use` × 1.**
`cargo clippy --workspace --all-targets --all-features -- -D warnings` fails on
eleven targets. The findings are `crates/multitop/src/tasks/painted.rs:29` (a
`#[must_use]` on a function returning `impl DoubleEndedIterator`, which is
already `#[must_use]`) and 41 `assert!(x.is_empty())` / `assert!(!x.is_empty())`
call sites across `crates/agent/tests/` and `crates/multitop/tests/`. Both lints
are new in clippy 1.99.0, and the stable toolchain was replaced on 2026-10-01
18:50 — so the gate went red on a clean tree with no commit to blame.

* *Major move?* No.
* *Red first:* it is already red — 49 error lines, named per target. To prove the
  fix bites, put one `assert!(x.is_empty())` back and watch the same step go red.
* *What could go wrong:* clippy's own suggestion is `assert_eq!(rows, [] as
  [multitop_agent::docker::Row; 0])`, which is worse to read than what it
  replaces, at 41 sites. Taking the machine's advice verbatim would trade a lint
  for a wall of type ascriptions. The shape worth building instead is one
  `assert_empty` / `assert_not_empty` pair in the test support module — which is
  also the fix for the class rather than the 41 instances, and leaves the next
  occurrence of the pattern correct by construction. `#[allow]` and `#[expect] are
  not available here (operator policy, and `check_no_allow.py` would go red).

**1b. `check_no_unreaped_spawn`: `crates/multitop/src/ssh/ssh_tests.rs:88`.**
The reap is `child.wait()` at line 91; line 90 is
`child.stdin.take().unwrap().write_all(payload).unwrap()`. A failing write panics
before the reap, and the leaked `sh` is the upload script itself — which starts
an `sshd` and holds a port. That is the shape the house gate was written for
today after media_server left nine live orphans, each holding a cargo build lock.

* *Major move?* No.
* *Red first:* already red, and the house check is calibrated (`--probe`, 19
  shapes measured against rustc 1.99.0 on 2026-10-03).
* *What could go wrong:* almost nothing. The fix is the guard the gate's own
  message names — wrap the `Child` so `Drop` kills and waits, which is strictly
  more robust than moving line 91 above line 90 (that fixes this panic and leaves
  the next one). The gate reported exactly one site, so there are no siblings to
  sweep — but grep for `Command::new` in `src/**/*tests*.rs` once, because a gate
  reports what it recognises, not what exists.

### Item 2 — prove the installed binary is current, and let the gate say so

The installed `multitop` is `/opt/homebrew/bin/multitop`: a regular file, **not** a
Cellar symlink, so it was placed by `./install.sh` (which `cp`s into
`$(brew --prefix)/bin`) rather than by `brew install`. No app bundle, no launchd
agent, nothing else on PATH.

Comparing the artifact rather than its version string: the installed binary is
5,412,624 bytes, sha256 `48b4b556…`, built 2026-09-30 22:17 — eighteen minutes
after the `v0.51.0` commit, so it is from the right *source*. But
`cargo build --release -p multitop` from the same HEAD under the current
compiler produces 5,477,232 bytes, sha256 `3108b887…` (+64,608 bytes, +1.19%),
and cargo rebuilds the entire dependency graph to do it. The cause is visible in
the toolchain directory: `~/.rustup/toolchains/stable-aarch64-apple-darwin` was
replaced on 2026-10-01 18:50, after that build.

So: the installed binary is source-current and **toolchain-stale**, and nothing in
this repo can detect that. Every end-to-end suite that drives
`target/release/multitop` has been evidence about a binary the current toolchain
would not produce.

* *Major move?* No.
* *What:* re-run `./install.sh` so the artifact matches the toolchain, and add a
  gate step that fails when the installed copy's bytes differ from
  `$(cargo metadata --format-version 1 | jq -r .target_directory)/release/multitop`.
* *Red first:* `cp` any byte-different binary over the installed path and watch the
  step go red. It has never been able to go red, which is the finding.
* *What could go wrong:* on a machine that deliberately runs an older pinned build
  this is noise. Scope it to "installed == current build", and let it skip with a
  stated reason when no install exists, in the shape
  `tools/empty_scope_allow.json` already documents for exactly this situation.

### Item 3 — three direct dependencies sit BELOW what a parent already resolves

This is the class the house calls a bug rather than a preference. The lockfile
holds **31 crates at more than one version**. Three of the duplicates are
direct-below-transparent, and all three were confirmed with `cargo tree -i`:

| Direct dep | Resolves to | Parent already resolves | Used at |
|---|---|---|---|
| `tower` 0.4.13 | 0.5.3 | `axum 0.7.9` | `crates/multitop/src/server_tests.rs:4`, `crates/multitop/tests/server_test.rs:13` |
| `sha2` 0.10.9 | 0.11.0 | `ed25519-dalek 3.0.0` | `crates/vault/src/crypto/keys.rs:9`, `lockout/state.rs:49`, `rollback.rs:1` |
| `zbus` 4.4.0 | 5.19.0 | `keyring → zbus-secret-service-keyring-store → secret-service 5.1.0` | `crates/vault/src/fprintd.rs` (Linux) |

**No type crosses any of the three boundaries today, and that was checked rather
than assumed.** `ServiceExt::oneshot` is a blanket impl over
`tower_service::Service`, and there is exactly one `tower-service` (0.3.3) in the
tree — so tower 0.4's extension trait satisfies an axum 0.7 Router that axum
itself built against tower 0.5. And no `sha2` type is ever handed to or received
from `ed25519-dalek`, which keeps its 0.11 hash private. So this is latent, not
broken — which is precisely why it is cheap to fix now and expensive later.

* *Major move?* Yes, all three, and each in its own commit:
  1. `tower` 0.4 → 0.5. One manifest line. Expected to be a no-op in code
     precisely because of the blanket impl — and that expectation is the thing to
     test, not assume.
  2. `zbus` 4 → 5. Real API work in `fprintd.rs`, which is regex-excluded from
     coverage and therefore only CI-verified. Removes **8 of the 31**
     duplicates: `zbus`, `zbus_macros`, `zbus_names`, `zvariant`,
     `zvariant_derive`, `zvariant_utils`, and `rand` 0.8 (whose only parent in the
     tree is `zbus` 4.4.0, verified).
  3. The crypto generation — `sha2` 0.10 → 0.11 together with `hkdf` 0.12 → 0.13
     and `argon2` 0.5 → 0.6, as ONE move, because half of it does not compile.
     Expected (not yet measured) to remove `sha2` 0.10.9, `digest` 0.10.7,
     `crypto-common` 0.1.7, `block-buffer` 0.10.4 and `block-padding` 0.3.3,
     collapsing the vault from two simultaneous RustCrypto generations to one.
     Do **not** bundle this with `zbus`: one is a serialisation transport, the
     other is the key derivation, and a failure in either should be attributable.
* *Red first:* before each bump, `cargo tree -d` lists two of the crate and
  `cargo tree -i <crate>@<old>` shows a direct-only edge; after, one version and
  the direct edge follows the parent. For the type claim specifically, add the
  assertion to a test: name the trait from the version axum resolves and compile
  it, so the blanket impl is proven rather than inferred.
* *What could go wrong:* `zbus` 5 is where this bites. `fprintd.rs` is excluded
  from coverage *and* from the macOS build, so a mistake there is invisible
  locally and only CI's Linux job sees it. Prove it with `tools/lint_linux.sh`
  and a real Linux run before pushing, not with the coverage number.

### Item 4 — nothing in the gate reports dependency currency

`cargo update --verbose --dry-run` moves **71 packages** inside their existing
semver ranges, so the lockfile itself is behind: `Cargo.lock` was last written
2026-09-30 and `cc 1.4.0→1.6.0`, `syn 3.0.3→3.0.6` and `tokio 1.53.1→1.53.2` are
the visible ones. Eleven **direct** deps are a major or more behind, taken from
the `Unchanged … (available: …)` lines:

| Crate | Have | Available | Gap |
|---|---|---|---|
| `dirs` | 5.0.1 | 7.0.0 | two majors |
| `mach2` | 0.4.3 | 0.7.0 | three minors / a major line |
| `axum` | 0.7.9 | 0.8.9 | one major |
| `tower` | 0.4.13 | 0.5.3 | one major |
| `zbus` | 4.4.0 | 5.19.0 | one major |
| `argon2` | 0.5.3 | 0.6.0 | one major |
| `hkdf` | 0.12.4 | 0.13.0 | one major |
| `sha2` | 0.10.9 | 0.11.0 | one major |
| `signal-hook` | 0.3.18 | 0.4.4 | one major |
| `zstd` | 0.13.3 | 0.14.0 | one major |
| `generic-array` (transitive) | 0.14.7 | 0.14.9 | patch line |

The other fourteen direct deps — `crossterm`, `ratatui`, `tokio`, `tokio-stream`,
`toml`, `toml_edit`, `secrecy`, `regex`, `serde`, `serde_json`, `libc`,
`num-traits`, `aes-gcm`, `ed25519-dalek`, `rand`, `zeroize`, `keyring` — are
current. No deprecation warnings anywhere in the workspace.

And **no gate says any of this.** `~/Projects/gates_of_heck/checks/check_dep_currency.py`
exists, its own docstring is *"a dependency pinned below the graph is a bug"* —
the exact class of item 3 — and it is run as `goh.sh deps`, which appears in
neither `.gatesrc`'s `GOH_CI_STEPS` nor `structural.sh --full` (whose nine steps
are emoji, conflict markers, file length, cap ceilings, shell lint, secrets,
markdown links, `Cargo.lock` vs manifests, unreaped spawns).

* *Major move?* No for landing the gate; yes for each bump it then forces.
* *Red first:* pin one direct dep to `=0.4.13` and watch `goh.sh deps` go red;
  then run its `--probe` so the gate is calibrated before it is trusted.
* *What could go wrong, and this one is real:* the gate is **red on arrival** —
  71 packages behind. Landing it as a plain gate produces a gate everyone learns
  to `--no-verify` past, which is worse than not having it. Land it as a
  **ratchet**: record today's state as the ceiling, fail only on *new* drift, and
  delete entries as each move above lands. `tools/ratchet_check.py` is already
  the shape for this in this repo; do not invent a second one.

### Item 5 — `rust-toolchain.toml` does not do what its own comment says

The header reads *"Pin the toolchain so the local gate and CI compile the same
code with the same compiler."* The body is `channel = "stable"`, which floats, and
`dtolnay/rust-toolchain@stable` in all three CI jobs floats with it. The two
sides agree by coincidence, not by construction — and item 1a is the receipt: the
estate's `rustup update` on 2026-10-01 turned this gate red with no commit in the
repository.

The same shape appears in the MSRV. `rust-version = "1.85"` in `Cargo.toml` and
"Rust 1.85+" in `README.md:278` are claims **nothing enforces** — CI has never
built anything but current stable (1.99.0). One of the two is false, and the
repo's own recent history says which way that goes: v0.51.0 shipped because a
docstring claimed the remote MCP command ran in a login shell, and it does not.

* *Major move?* No.
* *Red first:* `rustup update stable` and re-run the clippy step — it changes
  under you with a clean tree, which is the whole point.
* *What could go wrong:* pinning `rust-toolchain.toml` to a dated channel trades
  this bug for the "never build on an EOL toolchain" one, and creates a file
  whose owner nobody named. The honest middle is: keep `stable` (so CI keeps
  moving), and make the *artifact* carry its compiler — item 2's check is where
  that belongs. For the MSRV, pick one: add a 1.85 job to CI, or delete the claim
  from `Cargo.toml` and the README. Do not leave the third option, where a number
  is written down and nothing checks it.

### Do NOT

These are the tempting moves, and each is worse than not making it.

* **Do not take clippy's `assert_eq!(x, [] as [T; 0])` at 41 sites.** It is worse
  code. Item 1a wants one helper, not 41 type ascriptions.
* **Do not reach for `#[allow]` or `#[expect]` to quiet item 1a.** Operator policy,
  and `check_no_allow.py` goes red regardless.
* **Do not chase the 31 duplicates to zero.** Twenty-three are transitive-only and
  cannot be collapsed without an upstream move — four `windows-sys`, two
  `windows-targets`, `hashbrown` via ratatui's own two crates, `thiserror` 1.0.69
  held by `redox_users`. The three direct-below-transparent ones in item 3 are the
  whole actionable set.
* **Do not bump `dirs` 5 → 7 as part of a general sweep.** Two majors, and it is
  the only thing holding `thiserror` 1.0.69 in the tree, so it is genuinely worth
  doing — but `dirs` resolves the vault's config directory, and a path that
  silently resolves elsewhere is a vault that silently is not the vault. Its own
  commit, and a real-machine check that the right file is opened.
* **Do not treat the 71-package lockfile refresh as one commit.** It is the
  cheapest win in this list (semver-compatible, no code change expected) and the
  easiest to get wrong by bundling. One commit, on its own, before the majors.
* **Do not "fix" the empty coverage part by deleting the agent's bin target.**
  `⚠ multitop-agent (bin multitop-agent) exported a valid-but-EMPTY lcov part` is
  the coverage gate doing its job out loud: the target measures nothing because
  `main.rs` is regex-excluded from coverable lines. The fix is to say so in the
  exclusion's reason. Deleting the target to silence a warning about an
  unmeasured target is how a real hole gets to look tidy.
* **Do not re-order the two items under `## Next`.** The Android client's step 1 —
  `check_auth` compares the bearer token with `==` at
  `crates/multitop/src/server.rs:58`, and no `--serve-token` means no auth at all
  — is a security surface, it is already first, and nothing in this pass competes
  with it. This pass is the two-day unblock that lets work resume; it is not a
  reason to defer that item.
* **Do not cut a release from this pass.** The gate is red; a release now ships a
  binary that its own pre-push refuses.

### Calibration notes, so the next session does not have to re-derive them

* Three of the eleven checkers have no `scope_is_empty` guard:
  `check_doc_paths.py`, `check_gate_parity.py` (both structurally cannot see an
  empty scope) and `check_agent_version.py` (excused with a written reason in
  `tools/empty_scope_allow.json`, whose `known_blind` list is empty). The house
  empty-scope sweep already ran here on 2026-09-14, so this is a completeness
  note, not a live hole.
* `fuzz/Cargo.lock` is a second lockfile and already disagrees with the workspace
  on nine crates. `--locked` in `tools/fuzz_check.sh` does not care (it is
  self-consistent), so nothing is red — but every dependency bump in item 4 will
  need it regenerated, and the gate's error message already names the command.
* `tools/gate_lock.py` waits forever by design, but
  `~/Projects/gates_of_heck/gates/local_ci.sh` caps every step at
  `GOH_LCI_TIMEOUT` (default 900s, exit 124). The house covers this; there is no
  gap here.
* `target/lcov.info` on arrival was a concatenation of per-target exports whose
  naive merge reads 89.77% against a 95% floor. It is wrong, not alarming: the
  parts are per-test-binary and concatenating them double-counts every line. The
  96.26% above came from running `bash tools/coverage_check.sh` properly. Do not
  trust that file without re-running the gate.

### Two things are worth knowing before starting any item, because both cost a day to learn and neither is obvious from the code

- **A palette that measures correct can still be wired to nothing.** For one
  commit this feature detected a light terminal, stored the background, adapted
  a palette for it — and rendered with a view built *before* the probe ran, so
  1380 tests passed and the app was unchanged. The test that caught it drives
  `boot_app_with` and reads the App's own view, because a test that calls the
  rebuild itself proves the function works and not that anything calls it.
  `tools/light_terminal_check.py` then found a second defect the whole suite
  structurally could not: a hard-coded colour literal in the alert banner, which
  is correct on a dark terminal and invisible to the adaptation.
- **The measurement decides the shape of the change.** `PaletteView` reads like
  `Palette`, so none of the 26 render signatures needed editing. The first draft
  of this file assumed all 26 would. Ten minutes of counting saved an afternoon.

## Next

Two items, neither started, both larger than the one that just finished.

### Mobile companion, Android

The serving half already ships and is tested, so this is a client, and the
transport is **HTTP** — the API below — not the `WebSocket` an older entry of
this file named:

* `multitop --serve [<ADDR>]` and `--serve-token <TOKEN>` (`crates/multitop/src/main.rs`)
* `crates/multitop/src/server.rs` — 404 lines: bearer `check_auth`, `index`, and
  `/api/hosts`, `/api/health`, `/api/snapshot/:host`, `/api/history/:host`,
  `/api/mtp`, `/api/mtop`, plus `spawn_collectors`, which reuses the agent's own
  `stream::connect` and `Payload::Hello` validation

In the order it blocks:

1. **Auth hardening, first.** It is the one item here that is a security surface
   rather than a feature, and a client multiplies the exposure: the token is a
   bearer credential in a header, and it travels over a socket. A phone on a home
   network is the realistic deployment, so the questions to settle *before* a
   second client exists to lose the token are these — measured, not assumed:

   * `check_auth` compares with `==` (`crates/multitop/src/server.rs:58`), which is
     not constant-time. A remote timing oracle on a long random token is a poor
     bet, and it costs one dependency or one hand-rolled compare to close.
   * **No token means no auth at all** — `check_auth` returns `Ok(())` when
     `--serve-token` is absent, so a bare `--serve` is an open read API for
     anything that can reach the port. That may be the right default for a
     loopback-only server; it is certainly not the right one for the LAN bind a
     phone needs, and the two cases are currently indistinguishable.
   * The good news, and worth not breaking while fixing the rest: `--serve`
     already binds **loopback by default** (`serve_socket_addr`, `crates/multitop/src/main.rs:210` —
     `:8080` means `127.0.0.1:8080`), so nothing is exposed until someone asks
     for a socket address explicitly. Any change here has to keep it that way.
   * Whether the token ever reaches a log, an access log, or an error message.
2. **A read-only Android app** over the existing `/api/*`. A phone is a glance
   surface: which hosts are up, what the last upgrade did, what is degraded. Not a
   terminal emulator, and not the TUI's key handling in a smaller window.
3. **Push.** APNs needs a device token, which needs the app, so this is step 3
   rather than a parallel track. The trigger has to be the server noticing a
   *transition* — the current model is a client asking, which is correct for a
   glance surface and is why push is a real addition rather than a latency tweak.

The `repack`/`convert` logo pipeline (`scripts/repack_logos.py`,
`scripts/convert_logos.py`, over `crates/multitop/data/logos.bin.zst`) already
renders and repacks app icons, so an icon is not a blocker.

### Post-quantum KEM

Sequenced against its trigger, not a date. For the current threat model — a
device-local vault file read by a process that already holds the key — a
harvest-now-decrypt-later attack is not the binding risk, because an attacker who
can store a copy of the vault can also wait for a build with a classical KEM. It
earns its place when the threat model changes, and these are the changes to watch
for:

* the vault leaves the device — a synced file, a backup, a shared host. A copy is
  then durable, and the argument above stops applying.
* the KEM protects something with a long confidentiality life, where the
  ciphertext is the thing being protected rather than the file it wraps.

**Where it goes, and the part that is already built.** A vault key is wrapped
once per authenticator, tagged by a discriminant
(`crates/vault/src/crypto/wrapper.rs`: `SecureEnclave = 0x01`, `Tpm2 = 0x02`,
`Argon2id = 0x03`), and the file's own comment says it: *a vault carries one per
way it can be opened*. A KEM is a fourth wrapper, not a migration — the three
existing ones keep working and the new one is added beside them. So the shape of
the change is a new `WrapperType` variant whose wrapping key comes from a KEM
shared secret instead of an Argon2id derivation. The content cipher is untouched:
AES-256-GCM with an HKDF-derived sub-key
(`crates/vault/src/crypto/primitives.rs`).

Three things this must not do, all of which the current code already knows:

* **It must not touch the hardware-wrapped variants.** A Secure Enclave or TPM
  key is non-exportable by construction (`secure_enclave.rs`, `tpm2.rs`); a KEM in
  front of one buys nothing and costs the property that made it worth using.
* **It must not bump `min_proto_version`.** A KEM is an *additional* wrapper, and
  the agent↔client negotiation is a range check
  (`is_compatible`, `crates/agent/src/proto/mod.rs:135`): a newer agent stays
  compatible with an older client exactly as long as the old client's
  `PROTO_VERSION` is at or above the agent's `min_proto_version`. Raising that
  floor breaks every deployed client at once.
* **It must not be hand-rolled.** Prefer a crate with a maintained
  implementation — the same reason `hello-negotiation` exists as a skill rather
  than a paragraph. For the same reason, the KDF work here went to Argon2id and
  the AEAD to `aes-gcm` rather than to code in this repo.

The transport is a separate and much weaker case, deliberately not bundled here:
the agent link is a local-network control channel, and harvest-now-decrypt-later
on it is rarely the binding risk. Bundle it only if the vault trigger fires.

## Dropped, and why

Kept so the decision outlives the person who made it; a dropped item is not open
work, so it does not belong above.

| Dropped | Why |
|---------|-----|
| **Plugin SDK (WASM)** | Custom `exec` panels (`crates/multitop/src/config/types.rs`'s `custom_command`) cover it without a sandbox to get wrong. The cost of a plugin system is the isolation boundary, and nothing here needs one: a panel is a command the user already chose to run. Revisit if panels ever need to arrive from somewhere other than the user's own config file. |
| **A light variant per theme** | Superseded by background detection, which is one mechanism for all eight themes instead of sixteen palettes, and is correct on a background that is an image — which per-theme variants never can be. Shipped 2026-09-28. |
| **Adapting the agent's tty path** | The seam is already in place — `emit_fetch`/`emit_docker` take a `&PaletteView` — but `crates/agent/src/lib.rs:342` builds it with `for_theme`, the identity, because a process on the *remote* host cannot measure the background of the terminal the *user* is looking at. So the open question is not the plumbing but the supplier: whoever knows the client's background builds the view, and the only such caller today is the client itself. Worth doing when a second consumer appears, because the parameter is already there and the identity is the one line to change. (`Mode::Exec` does not take a view at all; it carries no colour literals, so nothing is hard-coded today.) |

## Where the invariants live

The hand-over notes this file used to carry — `Hello` first/valid/single, the
embedded agent matching the workspace version, the stable ad-hoc signature, the
vault's fallback, and "build with `./build.sh`" — are in
[DEVELOPMENT.md](../DEVELOPMENT.md#hello--agent-embedding), which is where a
developer looks for them and where they are kept current. The per-defect history
is [`detection-record.md`](detection-record.md); the general process rules are
in the house skills, not here.
