# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history (`git log --oneline`), in `CHANGELOG.md`, and in
`docs/detection-record.md`. When an item here is finished, delete it rather than
ticking it off.

State on 2026-10-21: Phase 0 (goh 0.27.0 wiring and the shipped defects,
P0.1-P0.7) and Phase 1 (gate drift, G1-G3) are closed and pushed; their record is
`git log --oneline` (`7a316b3`, `f096334`, `579ec14`, `3af81a5`, `dccdc7a`) and
`git show 2475be7:docs/roadmap.md`. Phase 2's A1 closed on 2026-10-17, A2.4
(the pairing screen and its Keystore-sealed store) closed on 2026-10-19 in
`e7da25a`, and the Ops view's fifth section - it reads `zfs.health` now, and
names the tool where a host does not serve it - closed on 2026-10-20 in
`d595639`. The Ops view then grew the machine model's sections on 2026-10-20:
the tunnel (`net.egress`) with the stack that declares it, the machine's
capacity (`disk.usage` and `host.facts`), the containers grouped by their
stack, and the estate's words for the sections the other client uses
(`zfs`, `containers`, `vpn`, `disk`) - servers ROADMAP O92, the plan of record
for the estate. **The Ops poll became a JOB on 2026-10-21** (servers ROADMAP
E36): it was bound to the Ops view, so it ran only while a human was looking
at the panel and the estate's fingerprint tools fired 0.3 a day against the
shared tools' 87. It is now `multitop --ops-poll`, a pass the scheduler runs
at twice the estate's publication rate, and what it finds is kept beside the
config so the view shows the poll's answer instead of asking the estate
again. **The rate is READ, not carried, since 2026-10-21** (servers ROADMAP
E53): the server publishes its publication cadence with the verdict, and the
client derives its rate from what it read, so a server that publishes faster
than its clients read is a warning the client prints rather than a silent
aliasing. **The installed agent's TICK is read too, since 2026-10-21** (servers
ROADMAP E53): launchd fixes an interval when the job is loaded, so the agent's
tick used to be a number this repo carried — and a publication that moved left
the agent reading too slowly, with the client's own warning firing on every
tick and nothing anyone could do about it but reinstall. The agent now paces
itself (`multitop --ops-agent`): it reads the published cadence every cycle
and waits the rate that cadence implies, and the installed launchd job carries
no interval at all. **The Ops frame can be PRINTED since 2026-10-21** (servers ROADMAP
O90): `multitop --ops-frame <host>` draws the frame the Ops view draws, from
the client's own answers, so the other client's parity check can compare the
two renderings without reading a terminal. **The TOOL CATALOGUE is KEPT
since 2026-10-21** (servers ROADMAP E61): the list of tools a host offers
is the biggest thing this client pulled off a host — 149,907 B a gather
across the estate, 33.1 MB a day of one immutable list, at the scheduler's
own rate — and it is now kept between passes, keyed to the server's
advertised version and to the set of tools the server says it does not
offer, BOTH read from the live server on every pass, and bounded by the
server's own `ttlMs`. The poll's inbound bytes fall from **66.0 MB a day
to 36.5 MB a day**, and the frame says where the list came from and how
old it is. **The kept list is checked against the server's own DIGEST
since 2026-10-21** (servers ROADMAP E72): the version and the absent set
were a guess about the list, and the guess re-listed the whole catalogue
at every release — including the four of the last ten that touched no
tool table at all — and could not see a table that moved without one. The
server publishes the digest of the exact list it serves
(`dev.multitop/catalogueDigest`), the client remembers the digest it was
handed and checks its kept copy against it, and a digest that is ABSENT
says so rather than passing for a check. Measured on the live estate
after the fleet shipped v1.99.21: the digest costs **+105 B a session a
host** and saves the **149,997 B a gather** the guess forced at every
release — **149,682 B a gather** net. **A MISSED DEADLINE IS A GAP IN ONE
ANSWER since 2026-10-21** (servers ROADMAP E74): the poll's local host
failed intermittently with `tools/call: no answer in 20s` — **20 of 133
passes** in the poll's own log — and the deadline ENDED the session, so
the host's whole reading was discarded and the estate's other readings
were never asked: an intermittent blind spot, read as a flaky tool. A
missed deadline now leaves the session good (a late answer carries its
own request's id, and the client matches the id, so it can never be taken
for the next one), the section that missed it reads **UNKNOWN** with the
call that missed it named, the host is gathered with the rest of its
reading kept, and the pass still says the gap instead of passing for
clear. The 20-second budget is the estate's and was not raised. **A2.5,
the read-only screens over `/api/hosts` and `/api/health` through
`PinnedTls`, closed on 2026-10-24 in `72efd5f`.** The next open work is A3 (push). **The Ops view draws the machine's VERDICT since 2026-10-24** (servers
ROADMAP O92, this repo's client half): `machine.triage` is asked with the
other sections and drawn at the top of the card - one `✗` row per item,
naming its stack, the owner's fix verbatim or `fix: none stated by the
owner`, the witnesses under it, then each stack's quiet line (`·`, what was
read there), so a stack nobody read never passes for one that is fine
(`ops/triage.rs`, `ops/render/triage.rs`, `ops/triage_tests.rs`; proven
live on all three machines with `--ops-frame`). The releases:
v0.56.0 carried A2.4 and the installer's replace-by-rename fix (`2413764`),
v0.57.0 carried the Ops view's ZFS section and the Android wrapper's move to
Gradle 9.8.2 — the release table is at the end.

**One thing the servers plan said that was wrong.** The plan (servers ROADMAP
O82) claimed the Ops view "reads `tools/list` generically ... with no code
change". The CLIENT read the list generically; the VIEW had four hard-coded
sections and named no tool at all, so the estate's SUSPENDED pool on 2026-10-19
was invisible to the client meant to show it. The plan is the plan of record for
the estate; this repo's own roadmap is the one for the phone.

Every item names its proof: the test or gate that goes red until it is done.

One question from Phase 0 was left open on purpose: whether `tools/lint_linux.sh`
should start its own container runtime. Today it fails closed naming `colima
start`, which is right for a pre-push gate on a shared machine.

## Phase 2 — Android companion (size L, three to five sessions)

Goal: a phone shows which hosts are up, what the last upgrade did, and what is
degraded, without a terminal. Exit: the app reads a LAN `--serve` over an
authenticated, hardened API, and a transition push reaches the phone.

The serving half ships and is tested; this is a client. Transport is **HTTP**
over the existing API (TLS on a LAN bind, below), not a `WebSocket`:

* `multitop --serve [<ADDR>]` and `--serve-token-file <PATH>`
  (`crates/multitop/src/main.rs`; the token in `serve_token.rs`); `--serve`
  binds **loopback by default** (`serve_socket_addr`: `:8080` means
  `127.0.0.1:8080`). Keep it that way.
* `crates/multitop/src/server.rs`: the default-deny `require_token` layer,
  static `index`, and `/api/hosts`, `/api/health`, `/api/snapshot/{host}`,
  `/api/history/{host}`, `/api/mtop`; `spawn_collectors` reuses the agent's `stream::connect` and
  `Payload::Hello` validation.

A1 (LAN-bind hardening) closed 2026-10-17: auth is a router-wide default-deny
layer and a 401 echoes nothing (`2cdbd04`); the token lives in a `0600` file,
is refused on argv, and is printed only to a tty (`1729119`). Its third
question is decided, and the decision is what A2 builds on:

* **A non-loopback bind does NOT require a hand-supplied token.** The generated
  one is 256 bits from the CSPRNG and persists in its file, so the phone pairs
  once; demanding one by hand only adds a step and invites a weaker token.
* **A non-loopback bind DOES need TLS before a phone uses it.** The remaining
  exposure is the wire: on plain HTTP anyone who can observe the LAN reads the
  bearer token and every response, and Android refuses cleartext by default
  (`usesCleartextTraffic` is false from API 28), so shipping without TLS would
  mean an app that opts out of the platform's protection. The shape is a
  self-signed certificate created beside the token and PINNED by the client at
  pairing (one string or QR: address, token, certificate fingerprint) -- no CA,
  no domain. It is built with A2, not before, because the pin's format and the
  pairing flow belong to the client, and a browser-based client would make a
  self-signed certificate the wrong answer.

In the order it blocks:

1. **A2 — a read-only Android app** over `/api/*`. A glance surface, not a
   terminal emulator and not the TUI's key handling in a smaller window.
   **Owner ruling D4 (2026-10-18):** native Kotlin, in this repo at `android/`,
   pinned self-signed TLS from pairing, its own gates and a
   `check_gate_parity.py` entry from the first commit.

   **A2.1 done 2026-10-18** (the commit that added `android/`): Gradle 9.8.1,
   AGP 9.4.1 with KGP 2.4.21, compileSdk android-37.2 / targetSdk 37, ktlint
   1.8.0, JUnit 6.1.3 -- the newest releases that day. **minSdk 28**: the floor
   at which Android refuses cleartext by default (the protection this client
   relies on rather than opts out of), and the floor `koffee_big` already ships
   to the owner's phone. 64-bit ABIs only (`abiFilters` plus an APK check). The
   gate is `tools/android_gate.sh` (ktlint, Android lint and kotlinc with
   warnings as errors, unit tests, a from-clean build, a deprecation ratchet in
   `android/gradle_deprecations.txt`, refused on an unsupported build host and
   red with install steps when the SDK is missing), run by CI's `android` job
   and by GOH_CI_STEPS; `check_gate_parity.py` fails either one dropping it.
   The pairing core is pure Kotlin under test
   (`android/app/src/main/java/com/ztomer/multitop/android/pairing/`), and the
   one screen says plainly that the app is not paired.

   **The pairing format is fixed by the client**, and it differs from the
   sketch above in one field: it carries the certificate itself, not only its
   fingerprint --
   `multitop://pair?v=1&addr=<host>:<port>&token=<64 hex>&sha256=<64 hex>&cert=<base64url DER>`.
   Android lint refuses any hand-written `X509TrustManager`
   (CustomX509TrustManager, reported on every implementing class), and a
   fingerprint alone can only be enforced by one. So trust is the PLATFORM
   trust manager over a store holding only the pinned certificate, then a
   session check that the leaf IS that certificate by SHA-256 (the platform
   alone accepts what the pinned key signs). `sha256` stays as the short form
   a human compares and must agree with `cert`. A P-256 certificate is ~530
   characters of payload: a dense but ordinary QR code.

   Remaining, in order:
   * **A2.2 done 2026-10-18 -- a Kotlin coverage floor of 80%** (line coverage;
     71% when it landed, measured 71.58%, rounded down; raised to 74% by A2.4,
     measured 238 of 318 lines, 74.84%, rounded down; raised to 80% by A2.5,
     measured 80.75%, rounded down). Kover 0.9.11 (JetBrains, released
     2026-09-29) rather than JaCoCo: it is built for Kotlin and Android and
     enforces a floor itself (`koverVerify`), where AGP's JaCoCo path produces a
     report and leaves the verification task to be hand-built. Scope: the main
     code of the debug and release variants under the JVM unit tests (the one
     activity included: it is 27 of the 95 lines, so the floor is honest about
     the screen being untested). The number lives once, in
     `tools/android_coverage_floor.txt`, read by `android/app/build.gradle.kts`
     and by `tools/android_gate.sh`, which runs `koverVerify` in the same Gradle
     invocation CI's `android` job runs; `tests/test_android_gate.py` fails if
     this paragraph and that file disagree. Proof: the floor at 72 printed
     `Rule violated: lines covered percentage is 71.578900, but expected minimum
     is 72` and failed the gate; at 71 it passes. The floor only rises: lift it
     in the commit that adds the tests.
   * **A2.3 done 2026-10-18 -- the server half of A2-TLS** (item 2 below).
     `crates/multitop/src/serve_tls.rs` (identity, `0600` beside the token,
     reused; the per-connection TLS listener), `pairing.rs` (the payload, read
     back by the client's rules before it prints, and its QR code),
     `serve_start.rs` (what `--serve` settles and prints). rustls 0.23 / rcgen
     0.14 on `ring`, one of each in the lock. **Plain HTTP's fate:** refused on
     every non-loopback bind (no HTTP answer on the TLS port); kept on loopback
     only, for the page at `/` and same-host `curl`. Proof: the golden pair
     `android/app/src/test/resources/rust_server.pem` and
     `android/app/src/test/resources/rust_server_pairing.txt` are written
     and checked by `crates/multitop/src/pairing_tests.rs` and parsed, then
     handshaken against, by
     the Kotlin tests; `crates/multitop/tests/serve_tls_test.rs` refuses a
     client pinned to
     another certificate.
   * **A2.4 done 2026-10-19 -- pairing in the app** (`e7da25a`). One Compose screen
     (`android/app/src/main/java/com/ztomer/multitop/android/ui/`), four honest
     states, each carried by a glyph and words, never colour alone:
     `· Not paired`, `→ Pairing`, `✓ Paired to <host>:<port>` (with the
     certificate's `sha256=` for a human to compare against the server's
     line), `✗ Pairing failed: <why>`. Scan (CameraX 1.6.2 frames into ZXing
     core 3.5.4 with `ALSO_INVERTED`, `scan/`) or paste; both go through
     `PairingPayload.parse`, so a wrong fingerprint is refused with its reason.
     Storage (`store/`): the canonical pairing string (`PairingPayload.format`,
     byte for byte what the server prints) sealed AES-256-GCM under one
     Android Keystore key (alias `multitop-pairing-v1`), one file in
     `noBackupFilesDir`; nothing in the clear, re-validated on every read, an
     unreadable one (a lost key, a flipped bit) reported and forgotten. Unpair
     deletes it. Compose UI tests run on the JVM under Robolectric 4.17.
     Proof: `QrDecoderTest` paints the QR the server really draws
     (`rust_server_pairing_qr.txt`, held to `pairing::qr` by the Rust test
     `the_golden_pairing_qr_is_what_the_server_draws`) as a dark and a light
     terminal and decodes both, and calibrates that a stock reader finds
     nothing in the dark one (dropping the inversion hint turns it red);
     `PairingStoreTest` and `PairingControllerTest` (round trip, no plaintext,
     unpair clears, wrong key and tamper unreadable, wrong fingerprint refused
     and nothing stored); `PairingScreenTest`, one per state. Not proven on a
     device: no phone or emulator is attached here, so the live camera path
     and the Keystore key are exercised only by building, not by a test.
   * **A2.5 done 2026-10-24 -- the read-only screens** (`72efd5f`). The paired screen carries the hosts panel
     (`ui/HostsPanel.kt`, `ui/HostsController.kt`): the server's own count
     from `/api/health`, then one line per host from `/api/hosts`, each with
     its own glyph and words (`✓ health N/100`, `⚠ breaching`, `· not read
     yet`, `✗ unreachable`), a failed read in words with the last overview
     kept and labelled as the last; last-wins refresh. The client
     (`api/ServerApi.kt`) is `HttpsURLConnection` over `PinnedTls` only, and the
     JSON is read strictly (`api/Overview.kt`: a missing field is refused by
     name, never defaulted). **One server defect it found:** a host with no
     reading was `"health": 100` and counted in `healthy`, so a server that
     had read nothing said every host was well; `health` is now absent until a
     reading, `/api/health` carries `unread`, and `/api/hosts` carries the
     config's `name` and a per-host `breaching`
     (`a_host_with_no_reading_is_never_counted_healthy`). Proof:
     `ServeEndToEndTest` starts the real `multitop --serve` on `0.0.0.0` (so it
     speaks TLS), pairs through `PairingPayload`, reads both endpoints, refuses
     a client pinned to another certificate and a wrong token; it went red
     against a binary built before `breaching` existed. `tools/android_gate.sh`
     builds `multitop` and sets `MULTITOP_BIN`; absent, the test fails rather
     than skips. Not proven on a device (none attached).

   Measured 2026-10-17: Android SDK at `~/Library/Android/sdk`, Gradle 9.8.1,
   Temurin JDK 27; no device attached; `ANDROID_HOME` unset (the gate defaults
   to that path on macOS). If the app ever links Rust, its targets are
   `aarch64-linux-android` and `x86_64-linux-android` and nothing else, and
   the build fails on any other ABI (house platform policy).
2. **A2-TLS — the certificate.** Server half: created `0600` beside the token,
   reused across restarts, served only on a non-loopback bind, and printed as
   the A2.1 pairing payload (certificate and fingerprint, to a tty only, as the
   token is). A leaf (`CA:FALSE`), since nothing it signs should be trusted. Proof: a
   server test that a non-loopback bind answers TLS and a loopback one plain
   HTTP; the client's test that a different certificate is refused.
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
* **Do not build a client on plain HTTP over a LAN.** A1's decision above:
  the pinned certificate lands with A2, not after it.
* **Do not take the `--serve` token on argv or print it to a non-tty.**
  `a_serve_token_on_the_command_line_is_refused` and the banner test fail it.
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

## Releases

One row per release, with the work it carried and the commits that carry it.
The tag is the record; the per-release detail is in `CHANGELOG.md`.

| Release | What it carried | Commits |
|---------|-----------------|---------|
| v0.56.0 | A2.4 (the Android pairing screen and its Keystore-sealed store, both QR polarities) and the installer's replace-by-rename fix (`install.sh` copies beside the target, signs the copy, and renames it over) | `e7da25a`, `2413764` |

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
