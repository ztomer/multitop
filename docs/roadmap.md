# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history (`git log --oneline`) and in `docs/detection-record.md`. When an item
here is finished, delete it rather than ticking it off.

Order: **Now** is user-visible defects and the one feature gap a user hit, in
the order they should land. **Hygiene** is the gate work left from the
2026-10-04 currency pass. **Next** is the two larger features, unchanged in
order. The version of this file before the 2026-10-10 consolidation is
`git show 56b90bd:docs/roadmap.md`.

## Now

### N1 — the upgrade window tears: apt's progress bar is glued onto every log line

**Reported 2026-10-10.** Reproduced offline only: nothing was run against a host,
because an upgrade changes a live system. The repro feeds an apt-style byte
stream through the real `Painter`, `paint_msg`, `App::apply` and `ui::draw` at
80x24, and every dpkg line renders as a progress bar with a `7` column on the
left and an `8` column on the right, the package name clipped off the edge:

```
| 7Progress: [ 16%] [########.......................................] 8 |
| 7Progress: [ 24%] [#############..................................] 8 |
```

**Why.** Upgrades run on a pty sized to the pane (`crates/agent/src/exec/pty.rs`,
`agent_dims` in `crates/multitop/src/ui/layout.rs`), so `apt` sees a tty and
draws its pinned status line: `ESC 7`, an absolute move to the bottom row
(`ESC[<rows>;0f`), the bar, `ESC 8` — no newline. Two parsers then mishandle it:

1. `consume_controls` in `crates/multitop/src/tasks/painted.rs` only consumes
   `ESC[` at the head of a line; the leading `ESC 7` stops it, so the move, the
   bar and `ESC 8` become text of the NEXT line. Absolute moves (`H`/`f`) are
   discarded as no-ops, so the pinned footer is flattened into the log.
2. `crates/multitop/src/ansi.rs` drops only the ESC of a non-CSI, non-OSC
   escape and prints the byte after it: `ESC 7` shows as `7`, `ESC(B` as `(B`.

Ruled out: the terminal. Ratatui's real crossterm output fed through a vt100
emulator matched its in-memory buffer on every frame (0 rows differ; the
comparison was shown able to fail by injecting a stray write).

**The class:** hand-rolled terminal parsers that each understand a different
subset of the escape grammar. Siblings: `crates/multitop/src/tasks/exec_runner.rs`
(same `Painter` for journal/tail/custom commands), `strip_ansi` in
`crates/agent/src/color.rs` (consumes one byte after ESC, so `ESC(B` leaks `B`;
`refit.rs` depends on it), and `ends_mid_sequence` in `painted.rs` (knows only
`ESC[`, so a chunk split inside `ESC(` is not held back).

**Fix, in order:**
1. ONE escape tokenizer shared by `Painter`, `ansi::line_to_spans` and
   `strip_ansi` — the `vte` crate's state machine rather than a fourth subset —
   so every sequence is consumed whole everywhere.
2. `Painter` models cursor save/restore and absolute rows: text written between
   `ESC 7`/`ESC 8`, or to a row outside the scroll region, is not a log line. It
   goes to a status message drawn as a pinned footer in the Upgrade pane, so
   apt's progress stays visible instead of being dropped.
3. Tests at the class level, both red on today's code: (a) a differential test
   over a fixture corpus (apt fancy progress, docker's `ESC[nA` layer blocks, a
   Rich live display, curl's `\r`, `tput sgr0`'s `ESC(B`) — the painter's log
   must equal what a terminal-emulator oracle shows in its scroll region (the
   `vt100` crate 0.15.2 does not handle `f`; map it to `H` or pick an oracle that
   does); (b) a table over every escape form (two-byte `ESC 7/8/M/=/c`, `ESC(B`,
   every CSI final byte, OSC with both terminators): all three parsers render
   `a<esc>b` as `ab`.
4. User-POV check last: a local `upgrade_cmd` that replays a captured apt byte
   stream (never a real upgrade), screenshot at real scale.

### N2 — the command bar is near-white on a dark terminal, in every theme

**Reported 2026-10-10; a regression since v0.50.0** (`f7cfb0a`, the light-terminal
adaptation). `crates/agent/src/palette_view.rs:130` builds the bar's background
as `triple(theme.ratatui_keybar_bg, adapted, Role::Text)` — the adapted TEXT
colour. Adaptation runs for every theme but PLAIN, and text is light on a dark
terminal, so the bar is light. Measured with the release binary in tmux,
`COLORFGBG=15;0` (detection correct: `Environment(0,0,0)`):

| Theme | Bar bg on screen | Theme's own `ratatui_keybar_bg` |
|---|---|---|
| Kare | 248,248,242 | 24,26,36 |
| Dracula | 248,248,242 | 40,42,54 |
| Nord | 236,239,244 | 46,52,64 |
| Gruvbox | 235,219,178 | 40,40,40 |

The bar's text is named terminal colours (`38;5;15`, `38;5;8`), so it is white on
near-white: 1.02–1.73:1 for the accent across all 8 themes. On a LIGHT terminal
the bar goes mid-dark with the same white text, also illegible.

**Siblings:** the Settings panel fills its whole block with `keybar_bg`
(`crates/multitop/src/config_ui.rs`), the "no host matches" screen paints the
same bar (`crates/multitop/src/ui/draw.rs`), and 51 named `Color::*` sites
bypass the theme entirely: `ui/keybar.rs` 7, `ui/keybar_confirm.rs` 2,
`modals.rs` 24, `config_ui.rs` 12, `ui/draw.rs` 6 (`ansi.rs` is legitimate).

**Why no gate saw it — the instrument was blind to backgrounds.**
`tools/check_no_hard_coded_colour.py` matches only 24-bit FOREGROUND escapes
(`38;2;…`), not ratatui named colours nor a colour taken from the wrong role.
`tools/light_terminal_check.py` reads only `38;2` and measures it against the
assumed terminal background; it never reads `48;2` or `38;5`, and never measures
text against the background of the cell it sits in.
`crates/agent/src/palette_view_tests.rs` checks the seven text escapes and none
of `keybar_bg`/`border`/`accent`.

**Fix, in order:**
1. **Gate first** (it must go red on today's tree): `light_terminal_check.py`
   resolves each cell's real background (`48;2`, `48;5`, or the terminal's) and
   requires 4.5:1 for text (3.0:1 for muted) against it, and fails any indexed or
   named foreground; plus a `TestBackend` render test over every theme × dark /
   light / unknown terminal for the keybar, Settings, confirm rows and modals.
   `check_no_hard_coded_colour.py` gains `Color::<named>`/`Indexed` outside
   `ansi.rs`, seeded with an allowlist of exactly today's 51 sites that must
   shrink to empty (stale entries fail).
2. `keybar_bg` is a BACKGROUND, not a text role: keep the theme's own value when
   it sits on the same light/dark side as the terminal, else derive it from the
   detected background nudged toward the opposite end.
3. The view carries adapted RGB for text, muted and accent, adapted against
   `keybar_bg`; every named `Color::*` outside `ansi.rs` becomes one of them,
   including the black text on the active-mode pill.
4. Prove the test can fail (revert to `Role::Text`, watch it go red), then
   screenshot light AND dark per `user-pov-debug`.

### N3 — the local machine has no ops view: give the agent an MCP mode

**Reported 2026-10-10.** The local panel (`host = "127.0.0.1"`, or `--local`,
which builds one with `mcp: None` in `crates/multitop/src/main.rs`) maps
`mcp: None` to `NotConfigured` (`crates/multitop/src/app/ops_view.rs`) and starts
no poll; `crates/multitop/src/ops/render.rs` shows a quiet "no mcp_host command
for this host" with a hint to add `mcp = "..."`. The tone is right; the hint is a
dead end on a Mac, because there is no command worth adding.

**What the view needs** (`crates/multitop/src/ops/mod.rs`): four sections, each
ready / absent-with-reason / failed — health (`health://latest`, only when
`health.run` is listed), cron (`cron.status`), containers (`docker.containers`),
alerts (`alerts.since`, 24 h). Nothing else from `mcp_host` is read.

**Rejected: running `mcp_host` on the Mac** (probed 2026-10-10, build 1.79.9,
empty root). It produces two FALSE readings and a false fault: `cron.status`
errors "no run_and_alert jobs" (a warning for an absence); `docker.containers` is
unlisted because it only knows `/var/run/docker.sock`, so the view says "no
Docker" while Colima runs at `~/.colima/default/docker.sock`; `alerts.since`
returns `[]`, a green "none in 24 h" for a log nothing on a Mac writes. It also
couples this repo to the servers repo and teaches a Linux fleet tool launchd.

**Rejected: an in-process local provider behind a trait.** Least protocol code,
but a second path around the client, poll and session machinery — the parallel
pipeline that always drifts (engineering rule #9) — and Claude Code cannot ask
the Mac anything through it.

**Chosen: `multitop-agent mcp`, local panel only.** Same MCP over stdio, so
everything after process start (client, poll, render, e2e) is unchanged, and it
keeps the repo's rule that local and remote differ only in what is spawned
(`crates/multitop/src/ssh/spawn.rs`; `crates/multitop/src/mcp/spawn.rs` already
runs a local panel's command via `sh -c`). Not for remote hosts:
re-implementing `mcp_host`'s 25 tools would be two servers that drift.

1. **Server.** `multitop-agent mcp` speaks the 2026-07-28 protocol (initialize,
   `tools/list`, `tools/call`, `resources/read`) and lists only:
   * `cron.status` from launchd — `launchctl list` gives label, PID while running
     and last exit status, which maps onto the cron columns directly;
   * `docker.containers` from the agent's existing Docker client
     (`crates/agent/src/docker.rs`, which honours `DOCKER_HOST`), extended with
     `all=1`, `State`, health parsed from `Status`, the compose-project label, and
     endpoint discovery from the docker context so Colima is found.
   Health and alerts stay unlisted, so they render as absent with a reason
   ("launchd host: no health suite"; "nothing on this Mac logs alerts") — never a
   green none. `log show` is too slow for a 30 s poll; `~/.routines/events` is a
   possible alerts source later. Tested with the existing `mcp::client` over an
   in-memory pipe.
2. **Wiring.** A local server with no `mcp` defaults to the agent's `mcp` mode via
   the same lookup as `local_agent_command()` (`app/ops_view.rs`,
   `crates/multitop/src/tasks/spawn.rs`); a configured `mcp` still overrides.
   `NotConfigured` remains for remote hosts only.
3. **Verify.** `crates/multitop/tests/ops_view_e2e.rs` drives the real agent
   binary instead of the Python stand-in; then `multitop --local`, `p`,
   screenshots light and dark.
4. **Bonus:** register `multitop-agent mcp` with Claude Code as the Mac's own ops
   server, beside `media-server`/`storage`/`pihole`.

**Upstream, not this repo:** `mcp_host` reports an empty cron table as an error
and a missing alert log as "no events" — an absence shown as a fault and an
absence shown as all-clear. Fix both in `~/Projects/servers/media_server`.

## Hygiene

### H1 — dependency currency as a ratchet, not a gate

`goh deps` runs in `gates_of_heck`'s `rust_gate.sh` with three severities: FATAL
for a direct dependency pinned below the graph (zero), `[major behind]` for a
direct dependency a major or more behind, and `[drift]` for the rest. The middle
one is deliberately not fatal. As of 2026-10-10 it holds one entry, held on
purpose: `signal-hook` 0.3 -> 0.4 waits for crossterm, whose newest release
(0.29.0) still depends on 0.3 — moving the direct dependency first would put two
majors of it in the graph.

What is missing is the ratchet: today's state as a ceiling, failing only on NEW
drift. `rust_gate.sh` already reads `GOH_DEPS_RATCHET`; `.gatesrc` does not set
it. The work is a ceiling file and one line, not a new mechanism. Landing it as a
plain gate instead is the outcome to avoid.

### H2 — `rust-toolchain.toml` does not do what its own comment says

The header reads *"Pin the toolchain so the local gate and CI compile the same
code with the same compiler"*; the body is `channel = "stable"`, which floats,
and `dtolnay/rust-toolchain@stable` in CI floats with it. The two sides agree by
coincidence; the 2026-10-01 `rustup update` turned the gate red with no commit,
which is the receipt. The honest middle is to keep `stable` and make the
ARTIFACT carry its compiler — which `tools/installed_freshness.py` does by
comparing bytes.

The MSRV is the same shape: `rust-version = "1.85"` in `Cargo.toml` and "Rust
1.85+" in `README.md` are claims nothing enforces. Add a 1.85 CI job, or delete
the claim from both places — not the third option, a number nothing checks.

### Do NOT

* **Do not chase the sixteen remaining duplicate crates to zero.** All
  transitive; `secret-service 5.1.0` (via `keyring`) holds the whole 0.10 crypto
  generation. Collapsing them needs an upstream move.
* **Do not land dependency currency as a plain gate.** H1 is the ratchet shape
  for exactly this reason.
* **Do not bundle a lockfile refresh with a major move.** Its own commit.
* **Do not "fix" the empty coverage part by deleting the agent's bin target.**
  `⚠ multitop-agent (bin multitop-agent) exported a valid-but-EMPTY lcov part` is
  the coverage gate saying out loud that `main.rs` is regex-excluded; say so in
  the exclusion's reason instead. (N3 gives that binary a real `mcp` mode, which
  will need its own coverage.)
* **Do not move Android's auth hardening behind the app.** It is a security
  surface and a second client multiplies the exposure.
* **Do not "fix" N1 by stripping escapes harder in one more place.** That is a
  fourth parser subset; the fix is one tokenizer.

## Next

Two items, neither started.

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

## Sources consolidated (2026-10-10)

Every other agent session that worked on this repo, and what became of it. None
carries a plan any more; this file does.

| Source | Disposition |
|--------|-------------|
| Claude Code (this repo's slug) | This session only; no desktop sessions. `memory/` kept — it is the project's memory store. |
| opencode db sessions | Already gone; only the project row remains. |
| opencode loop goal "implement the roadmap" (2026-09-03, `active`) | Superseded: its roadmap was rewritten twice since; the current one is this file. |
| opencode loop goal "implement the plan" (2026-09-13) | Completed (upgrade stderr duplicate-line fix); landed, nothing open. N1 is a different defect in the same `Painter`. |
| opencode loop goal "impl3m3n5" (2026-09-28) | Completed (light-terminal palette, v0.50.0). Its wiring step introduced N2. |
| opencode loop: nine empty job files, `loop.log` | Residue; removed. |
| Antigravity, 2 conversations (2026-07-28: Docker header line, layout research) | Both died on API 429 after 7 steps with no output; nothing to carry. Removed. |
| Branch `ci-persist` + worktree `~/Projects/.wt-mon-ci` | Ancestor of `main`, worktree clean; removed. |
| Old Item 3 (Linux vault build CI-only) | Closed: Colima is installed, `tools/lint_linux.sh` (already a `GOH_CI_STEPS` entry) ran clean 2026-10-10. |
| Old Item 4 (spawn gate accepts a no-op `Drop`) | Closed upstream: `goh unreaped-spawn` now reads the `Drop` body; a copy of `ssh_tests.rs` with `Reap`'s `Drop` emptied is rejected ("whose `Drop` neither kills nor waits"), the real one passes. |
| "Two things this pass cost a day each" | Lessons, not plan: they live in the `calibrate-the-instrument` and `stale-artifact-under-test` skills. |

## Where the invariants live

The hand-over notes this file used to carry — `Hello` first/valid/single, the
embedded agent matching the workspace version, the stable ad-hoc signature, the
vault's fallback, and "build with `./build.sh`" — are in
[DEVELOPMENT.md](../DEVELOPMENT.md#hello--agent-embedding), which is where a
developer looks for them and where they are kept current. The per-defect history
is [`detection-record.md`](detection-record.md); the general process rules are
in the house skills, not here.
