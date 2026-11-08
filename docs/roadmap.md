# Roadmap

The one forward-looking backlog. Finished work is not listed here: it is in
`git log --oneline`, in `CHANGELOG.md` and in `docs/detection-record.md`. When an
entry here is finished, delete it. Every open entry has an id and a `Done when:`
clause; an id marked `(queue: X)` is also an item in the umbrella queue
(`~/Projects/.loop/queue.json`). The estate plan the Ops view follows is the
servers repo's ROADMAP (O92); this file is the plan for the phone and the agent.

State on 2026-11-08: v0.69.0 is the latest tag. The phone push (A3) is
built end to end - `--serve` alerts over UnifiedPush first and FCM after it, and
a private distributor (the fleet's own ntfy, servers DECISIONS #17) is named in
`push-distributor.toml` - and what is left is its live delivery and two defects.

## Phase 1 - defects in released code (no owner input needed)

Why first: both are violations of a standing rule in code that is already out
(the house load rule, and engineering rule #17), and neither waits on anything.

* **M.FLOODREAP** (queue: M.FLOODREAP). A test shell started with
  `zsh -l -i -c 'i=0; while [ $i -lt 20000 ]; do echo flood-$i; ...'` (the
  command in `crates/multitop/tests/event_loop_e2e/liveness.rs`; the marker is
  `STARTED_SENTINEL`, `__multitop_started__`, in `crates/agent/src/exec/mod.rs`)
  was found orphaned (parent 1) after 12h23m on 2026-11-08, blocked until the
  coordinator killed it. The owner of a spawned process reaps it, pass or fail:
  find the path that starts it and make the spawn reap its whole tree on every
  exit, including test failure and timeout.
  Done when: a test that fails the run asserts no `__multitop_started__` shell
  outlives it (red first on the current spawn), and the fix is released.
* **M.FCMDEPR** (queue: M.FCMDEPR). `push/FcmService.kt` still overrides the
  deprecated `onNewToken` (Firebase 26 calls it on its legacy path beside
  `onRegistered`) because Android lint's `MissingFirebaseInstanceTokenRefresh`
  predates Firebase 26 and demands it. Rule 17: no deprecated API. Use the
  current path and configure lint for Firebase 26, or report the false positive
  upstream and record it here.
  Done when: no deprecated FCM API is in the app, `tools/android_gate.sh` is
  green without the forced override, and the deprecation ratchet
  (`android/gradle_deprecations.txt`) is red on a reintroduced one.

## Phase 2 - the phone push reaches the phone (waits on the fleet and the owner)

Why second: the code is done and tested; what remains is infrastructure and
credentials outside this repo, so it cannot be hurried from here.

* **D.MON1** (queue: D.MON1). A3 live delivery. Measured 2026-11-08: a test
  alert decrypts with an independent RFC 8291 implementation; the publish to
  `ntfy.sh` was refused with 507 (it refuses a sender and a phone behind the
  same home IP), which is why the route moved to the fleet's own ntfy (queue:
  SRV.NTFY, servers repo, in progress); FCM is untested for want of credentials.
  Done when: one live alert reaches the Pixel over UnifiedPush through the
  self-hosted ntfy, one live alert reaches it over FCM (or the owner rules FCM
  out under OW.MONPUSH and the route is deleted), each path is held by a test,
  and the release is installed.
* **OW.MONPUSH** (queue: OW.MONPUSH; owner's). The FCM fallback needs a Firebase
  project for `com.ztomer.multitop.android`: `android/app/google-services.json`
  at build time (gitignored) and `fcm-service-account.json` (`chmod 600`) beside
  the server's `config.toml`. Without them the route is built and inert, and the
  app says so.
  Done when: both files exist where named, or the owner rules the FCM route out.
* **M.DEVICE.** The pairing screen's live camera path and its Keystore key have
  only ever been exercised by building, never by a test or on a phone (CameraX
  into ZXing, the Keystore-sealed pairing file); no automated test can hold
  them.
  Done when: the app installed on the Pixel pairs by scanning the QR the server
  prints and the hosts panel lists the hosts, observed on the device.

## Phase 3 - parked on a trigger (nothing to do until it fires)

Why last: each is gated by an event, not a date; doing it early is guesswork.

* **M.RATCHET.** `tools/deps_ratchet.txt` holds two majors held behind on
  purpose: `signal-hook` 0.3 -> 0.4 waits for `crossterm` (0.29.0 is still the
  newest release, checked 2026-11-08; moving ours first puts two majors in the
  graph), and `zbus` 5 -> 6 (5.19.0 locked, 6.2.1 current), transitive and
  deferred pending upstream migration.
  Done when: `tools/deps_ratchet.txt` has no entries and the dependency gate is
  green; delete each entry in the commit that moves it.
* **M.AGENTTTY.** The agent's tty path draws with `for_theme`, the identity
  palette (`crates/agent/src/lib.rs`), because a process on the remote host
  cannot measure the user's terminal background; the seam exists
  (`emit_fetch` and `emit_docker` take a `&PaletteView`). Trigger: a second
  consumer that knows the client's background appears.
  Done when: (owner to decide).
* **M.PQKEM.** Post-quantum KEM for the vault (size M when it fires). For a
  device-local vault read by a process that already holds the key,
  harvest-now-decrypt-later is not the binding risk. It becomes one when the
  vault leaves the device (a synced file, a backup, a shared host) or the KEM
  protects something with a long confidentiality life; neither had happened on
  2026-10-12. The shape is prepared: a vault key is wrapped once per
  authenticator, tagged by a discriminant (`crates/vault/src/crypto/wrapper.rs`:
  `SecureEnclave = 0x01`, `Tpm2 = 0x02`, `Argon2id = 0x03`); a KEM is a fourth
  `WrapperType` whose wrapping key comes from a KEM shared secret, beside the
  others and not a migration. The content cipher is untouched.
  It must not touch the hardware-wrapped variants (a Secure Enclave or TPM key is
  non-exportable, a KEM in front buys nothing), must not bump `min_proto_version`
  (`is_compatible`, `crates/agent/src/proto/mod.rs`: raising the floor breaks
  every deployed client at once) and must not be hand-rolled (a maintained
  crate, as with `argon2` and `aes-gcm`). The agent transport is a separate,
  weaker case; bundle it only if the vault trigger fires.
  Done when: a fourth wrapper opens a vault in a round-trip test, and a vault
  written before it still opens.

## Rulings in force

Each is dated and sourced; the later ruling wins where two disagree.

* **2026-11-08, OW.26, "both, ntfy first".** Alerts go UnifiedPush first, FCM
  after it (also when UnifiedPush fails), none when nothing is registered
  (`crates/multitop/src/push/`). Owner, same day: "Self-host ntfy" (queue:
  SRV.NTFY); a private distributor's token and CA apply to that origin only.
* **2026-10-18, D4.** The Android companion is native Kotlin in this repo at
  `android/`, with pinned self-signed TLS from pairing, its own gates and a
  `check_gate_parity.py` entry. A read-only glance surface, not a terminal.
  If the app ever links Rust, its targets are `aarch64-linux-android` and
  `x86_64-linux-android` and the build fails on any other ABI (house platform
  policy); minSdk 28 and 64-bit ABIs only.
* **2026-10-18, A2.2 - a Kotlin coverage floor of 80%.** The number lives once,
  in `tools/android_coverage_floor.txt` (`tests/test_android_gate.py` fails if
  this line and that file disagree). The floor only rises: lift it in the commit
  that adds the tests.
* **2026-10-17, A1 (commits `2cdbd04`, `1729119`).** `--serve` binds loopback by
  default. A non-loopback bind needs no hand-supplied token (the generated one is
  256 bits and persists in its `0600` file) but needs TLS before a phone uses it,
  and plain HTTP is refused on every non-loopback bind. The token is never taken
  on argv nor printed to a non-tty (`a_serve_token_on_the_command_line_is_refused`
  and the banner test fail it). Do not build a client on plain HTTP over a LAN.
* **2026-11-01, M1, owner design.** "We have limited number of potential
  processes with limited length, and we can use a vector only if we're above
  this number": a warm agent tick does no heap work in proportion to the host.
  `crates/agent/tests/rss_bound_test.rs` fails it, and the musl allocator is the
  reason (mimalloc was tried and rejected: flat, but 8 MB resident from minute
  one).
* **2026-10-10, H1 (`ba9093e`), dependency currency is a ratchet**
  (`GOH_DEPS_RATCHET=tools/deps_ratchet.txt`), not a plain gate; do not chase the
  remaining duplicate crates to zero (all transitive; `secret-service` via
  `keyring` holds the 0.10 crypto generation); do not bundle a lockfile refresh
  with a major move.
* **Standing, enforced by their checks.** No minimum Rust version without a job
  that builds on it (`tools/check_msrv_claim.py`). Do not delete the agent's bin
  target to empty the coverage part: the `valid-but-EMPTY lcov part` warning is
  the gate saying `main.rs` is regex-excluded, and the `mcp` mode lives in the
  library, driven by `ops_view_e2e.rs`. Terminal escapes are read only in
  `multitop_agent::escape` (`tools/check_one_escape_reader.py`). ratatui chrome
  is drawn only in a `Chrome` role and app text only in the current palette
  (`chrome_contrast_test`, `tools/check_no_hard_coded_colour.py`).
* **Declined: a plugin SDK (WASM).** Custom `exec` panels (`custom_command` in
  `crates/multitop/src/config/types.rs`) cover it without a sandbox to get
  wrong. Revisit if panels ever arrive from somewhere other than the user's own
  config.

## Where the invariants live

The hand-over notes - `Hello` first/valid/single, the embedded agent matching the
workspace version, the stable ad-hoc signature, the vault's fallback, "build
with `./build.sh`" - are in
[DEVELOPMENT.md](../DEVELOPMENT.md#hello--agent-embedding). The per-defect
history is [`detection-record.md`](detection-record.md); general process rules
are in the house skills.
