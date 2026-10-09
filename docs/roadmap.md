# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history (`git log --oneline`) and in `docs/detection-record.md`. When an item
here is finished, delete it rather than ticking it off.

## Now

**The currency and freshness pass ran 2026-10-04 to 2026-10-05.** Items 1, 2 and
3 below are finished and have been deleted rather than ticked: the gate is green,
the installed binary is provably this build's, and no direct dependency sits
below what the graph already resolves. What is left is here, and the two things
this pass learned the hard way are recorded because neither is visible from the
code.

### Item 2 — dependency currency as a ratchet, not a gate

`goh.sh deps` now runs in `rust_gate.sh` and reports three severities: FATAL for
a direct dependency pinned below the graph (zero of them, as of 2026-10-05),
`[major behind]` for a direct dependency a major or more behind, and `[drift]`
for everything else. The middle one is deliberately not fatal, because a gate
that is red on every honest commit is one nobody reads. As of 2026-10-08 it
holds one entry, and it is held on purpose: `signal-hook` 0.3 -> 0.4 waits for
crossterm, whose newest release (0.29.0) still depends on 0.3 -- moving the
direct dependency first would put two majors of it in the graph.

What is missing is the ratchet: today's state as a ceiling, failing only on NEW
drift, with entries deleted as each move above lands. `tools/ratchet_check.py`
is already that shape in this repo for line counts, and
`gates_of_heck`'s `--ratchet` flag already exists for this gate, so the work is
wiring and a ceiling, not a new mechanism. Landing it as a plain gate instead is
the outcome to avoid.

### Item 3 — the Linux build of the vault is CI-only, and this machine cannot run it

`crates/vault/src/fprintd.rs` is `#[cfg(target_os = "linux")]`, so no macOS gate
compiles a line of it. `tools/lint_linux.sh` is the local answer and it needs a
container runtime, which is not installed here -- so the zbus move was verified
by compiling the module's whole zbus surface against 5.19.0 in a scratch crate
(the API is proven; the platform is not) and left to CI's ubuntu jobs for the
rest. That split was recorded in the commit rather than left to be discovered,
and it is worth a second look whenever the next Linux-only change lands: start
Colima, or install a container runtime, or the next person repeats the exercise
with less evidence.

### Item 4 — the house spawn gate cannot tell a guard from a wrapper

`check_no_unreaped_spawn.py` is right to insist on a guard and wrong about what
makes one. Measured 2026-10-04 with the `Drop` body of a correct `Reap` emptied
to nothing: the gate still reported clean. It accepts the wrapper *shape* -- any
`impl Drop` in the file, plus a `Self(` in the same function -- without reading
what the `Drop` does, and `_spawn_shapes.py`'s own docstring says a guard is
"judged by what its `Drop` does, never by its name". So a guard that reaps
nothing passes.

Not fixable from here (`gates_of_heck` is not this repo's to edit), and the
consequence is recorded where it bites: `ssh_tests.rs`'s `Reap` carries its own
test, `a_panic_after_the_spawn_leaves_no_child`, which panics between the spawn
and the reap and requires the pid to be gone -- `kill -0` distinguishes that from
a zombie, and therefore from a guard that kills and forgets to wait. Red with the
`Drop` emptied, green with it in. Every future `Reap` in this repo needs that
test, because the gate will not ask.

### Item 5 — `rust-toolchain.toml` does not do what its own comment says

Unchanged and still open, and still deliberately not a quick fix. The header
reads *"Pin the toolchain so the local gate and CI compile the same code with the
same compiler"*; the body is `channel = "stable"`, which floats, and
`dtolnay/rust-toolchain@stable` in all three CI jobs floats with it. The two
sides agree by coincidence, not by construction.

The estate-wide `rustup update` on 2026-10-01 turned this repo's gate red with no
commit in the repository, which is the receipt. The honest middle is to keep
`stable` (so CI keeps moving) and make the *artifact* carry its compiler -- which
is what `tools/installed_freshness.py` now does, comparing bytes rather than
trusting a version string.

The MSRV is the same shape: `rust-version = "1.85"` in `Cargo.toml` and "Rust
1.85+" in `README.md` are claims nothing enforces, since CI has only ever built
current stable. Pick one deliberately -- add a 1.85 job to CI, or delete the
claim from both places. Do not leave the third option, where a number is written
down and nothing checks it.

### Do NOT

Carried forward from the assessment, minus the parts this pass settled. Each is
still worse than not making the move.

* **Do not chase the sixteen remaining duplicate crates to zero.** They are all
  transitive now -- `cargo tree --target all -i digest@0.10.7` names
  `secret-service 5.1.0` as the single parent holding the whole 0.10 crypto
  generation, reached through `keyring`. Collapsing them needs an upstream move,
  and forcing them from here is churn.
* **Do not land dependency currency as a plain gate.** It is red on arrival --
  four direct deps are a major or more behind. A gate people `--no-verify` past
  is worse than no gate. Item 2 is the ratchet shape for exactly this reason.
* **Do not treat the 71-package lockfile refresh as one commit with a major
  move.** It is semver-compatible and the cheapest win in this list, which is
  precisely why it is the easiest to bundle wrongly. Its own commit.
* **Do not "fix" the empty coverage part by deleting the agent's bin target.**
  `⚠ multitop-agent (bin multitop-agent) exported a valid-but-EMPTY lcov part`
  is the coverage gate doing its job out loud: `main.rs` is regex-excluded from
  coverable lines, so the target measures nothing. The fix is to say so in the
  exclusion's reason. Deleting the target silences a warning about an unmeasured
  target, which is how a real hole gets to look tidy.
* **Do not take a `Reap` guard's word for it.** Item 4: the house gate accepts
  the wrapper shape, so the guard's own test is the only thing that checks it.
* **Do not re-order the two items under `## Next`.** The Android client's auth
  hardening is a security surface, it is already first, and nothing in this pass
  competes with it.

### Two things this pass cost a day each, and neither is visible from the code

- **A gate can be green and wrong.** `check_no_unreaped_spawn.py` reporting clean
  over a neutered `Drop` is item 4, and it is the general shape: the check
  recognised a pattern rather than the property. The fix in this repo was not to
  trust the green but to write a test that could fail -- and the only way to know
  a new test can fail is to break the code once and watch it go red.
- **Two stale artifacts compare equal to each other.** The freshness gate's first
  version compared `target/release/multitop` against `/opt/homebrew/bin/multitop`
  and reported byte-identical while BOTH were the stale build it existed to
  catch. The lesson generalises past this gate: a comparison is only evidence if
  one side is independently known-good, and "the build tree" is not a known-good
  side until something has rebuilt it.


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
