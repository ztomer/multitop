# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history (`git log --oneline`) and in `docs/detection-record.md`. When an item
here is finished, delete it rather than ticking it off.

## Now

One item is close enough to finish to be worth finishing.

### Finish the light-terminal palette

**The mechanism and a real palette defect are done (2026-09-28).** What remains is
detection and one call site. Details below, because the shape of the remaining
work was wrong in the previous version of this file and correcting it changed
what is left.

**The claim, measured.** All 8 themes (`KARE`, `DRACULA`, `NORD`, `GRUVBOX`,
`CATPPUCCIN`, `TOKYO_NIGHT`, `MONOKAI`, `CYBERPUNK` in
`crates/agent/src/color.rs`) are dark, so yellow — the `⚠` glyph, the warn state —
and cyan, which every host banner is drawn in, wash out on a light terminal. In
every view, not just Ops. On a near-white terminal the roles land at:

| role | contrast on light |
|------|-------------------|
| `text` | 1.03 – 1.34 : 1 |
| `primary` (the banner) | 1.35 – 2.05 : 1 |
| `meter_mid` (the warn yellow) | 1.09 – 1.95 : 1 |

`the_roadmaps_claim_holds_for_every_theme_on_a_light_terminal` in
`crates/agent/src/adapt_tests.rs` asserts that arithmetically for all 8, so it
cannot rot into an opinion and a 9th theme is covered by the same assertions. It
asserts the *defect*: it goes red when the palettes are fixed, which is the point.

**Done.** `surface.rs` (WCAG luminance and ratio, HSL, and a `const fn` that
reads the RGB out of a palette's own ANSI escape, so there is one definition per
colour rather than eight more fields per theme), `lab.rs` (CIE76 ΔE — a separate
instrument, because contrast is luminance-only and rates a cyan and a green at one
lightness as identical while they are obviously not), and `adapt.rs`, which fits
one affine map on relative luminance across a palette's own range. One mechanism
for 8 themes instead of 16 palettes, and correct on a background that is an image,
which per-theme variants never can be.

The obvious implementation was wrong and shipped first: adjusting each role
independently drives every failing role to the *same* luminance, so Kare's
`primary` and `secondary` came out 1.01 : 1 apart on light — the same colour. That
buys legibility by spending a distinction, which is what the house rule about
single-channel distinctions exists to prevent.

**Also done, and not on this roadmap before:** measuring each theme against its
*own* background (they differ, and one shared dark asked a question about none of
them) found four roles below the floor in three shipped palettes. `meter_high` —
the high-meter *and* alert colour — was 3.05 : 1 in Nord, 4.29 : 1 in Gruvbox,
3.93 : 1 in Monokai, and Nord `secondary` 4.41 : 1. All lifted along the hue line,
hues preserved, each now a test.

#### What is actually left, and the correction

The previous entry said the obstruction was architectural: *"the agent renders on
the remote host but its escapes are interpreted by the local terminal, so the
remote side cannot answer the question for itself."* **That was wrong, and
checking it rather than trusting it is what unblocked the item.** The agent has
two rendering paths and they are not symmetric:

* **When stdout is not a tty** — which is every connection from this client, since
  the agent's stdout is a pipe over SSH — `emit_fetch`, `emit_docker` and
  `monitor_loop` all encode a `Payload` and write it to the pipe. Nothing is
  rendered remotely. The client decodes it and calls `render_payload`
  (`crates/multitop/src/render_payload.rs`), which calls the agent's own
  `render` **in the client process, with the client's palette**. So the main panes
  — Monitor, Docker, Fetch, and the banner — are already drawn locally, and
  adapting the local palette is the whole job for them. There is no protocol
  change and nothing to pass to the agent.
* **When stdout *is* a tty** — someone running `multitop-agent` directly over SSH —
  `palette_for_env()` picks `PLAIN` under `NO_COLOR` and `ANSI` (Kare) otherwise,
  and the agent paints with literal escapes. This path is a person looking at
  their own terminal, and it gets the fix only if we choose to pass the background
  to the agent. **Defer it.** It is the small case, it can be fixed by a config
  setting rather than a protocol change, and no evidence yet says it is the one
  people hit.

So the remaining work is three steps, all on the client:

1. ~~**Detect.**~~ **Done.** `crates/multitop/src/background.rs` queries `OSC 11`
   at startup, falls back to `COLORFGBG`, then to the theme's own
   `ratatui_keybar_bg`, and `boot_app` stores the result on `App`. `OSC 11` is
   right and not universally answered; `COLORFGBG` is free and wrong on some
   terminals. Three things that are easy to get wrong and are handled: the reply
   is 16 bits per channel, so it is scaled by the width actually sent (`f` and
   `ffff` are the same white, and reading either as a byte gives 15 or 255);
   `COLORFGBG=7;0` is what an *unconfigured* emulator leaves behind, and index 7
   is light grey, so treating it as a measurement adapts a dark terminal to a
   light palette — it is refused; and a terminal that never answers is the common
   case, so the read is bounded at 120 ms and a probe that hangs is a probe that
   cannot ship. 15 tests, calibrated by sabotage: reversing the source priority,
   removing the width scaling, accepting the unset default, and accepting a lone
   `COLORFGBG` field each turn the right test red.

   The bound is enforced by the call that waits, not checked around it —
   `crossterm::event::poll` with the remaining budget — because a deadline in a
   `while` condition around a blocking `stdin.read` is not a timeout at all. The
   first version had exactly that bug and hung the entire tmux suite; it got
   through one commit attempt because the release binary was older than the probe,
   so the suite exercised a build without it.
2. ~~**Adapt once, in the one accessor every caller already goes through.**~~
   **Done.** `PaletteView` (`crates/agent/src/palette_view.rs`) reads like a
   `Palette` — same field names, same accessor names, same `status_color` /
   `cpu_bar` / `mem_bar` classification — so the render path takes `&PaletteView`
   and the bodies do not change. The App holds one, behind an `Arc`, and rebuilds
   it on theme-cycle and at boot.
   `App::current_theme` is the single source of the palette and has **17 call
   sites**: four in `app/views.rs`, three in `app/upgrade.rs`, two in
   `app/ops_view.rs`, and one each in `app/render.rs`, `app/apply.rs`,
   `ui/draw.rs`, `modals.rs`, `config_ui.rs` and `password_actions.rs` — all
   under `crates/multitop/src`. The other two are theme-`save` sites that want
   the *name* only. So the seam is one function, not seventeen: it returns
   `&'static Palette` today, and the version that returns an adapted one owns a
   cache keyed on (theme index, background), recomputed only when either changes.
   Theme-cycling re-adapts for free, and adapting per frame would be the obvious
   mistake — a bisection per role per cell.

   The measurement that made it small: the 26 signatures all either pass the
   palette to `format!` or read `pal.reset` as a field, and **neither needs a
   `'static` lifetime** — so a view with owned escapes and `&str` returns
   satisfies them as they stand. No refactor. Counting them properly cost ten
   minutes and saved an afternoon; the first draft of this entry assumed all 26
   would need editing.

   Three things the view needed that `Palette` did not have, each a real finding:
   the three `ratatui_*` colours (the keybar background, the border, the accent)
   are painted as `ratatui::Color` rather than as escapes and are adapted too —
   a dark theme's keybar on a light terminal is a black bar across a white screen;
   `Arc` rather than `Rc`, because the `App` is moved onto a tokio task in the
   event-loop tests and `Rc` is not `Send`, and an `Rc` fails in exactly one
   place; and `App::current_theme` returns the handle, not a borrow, because a
   palette reachable only by borrowing `self` collides with every `&mut panels` in
   the render path.

   **The bug this cost, and why the test that catches it is shaped the way it is.**
   For one commit the app detected the background correctly, stored it correctly,
   adapted correctly — and rendered with the view built in `App::new`, *before*
   detection ran, against a default. 1373 tests passed. Every one of them tested a
   module; none tested the connection. So the test that covers it goes through
   `boot_app_with`, a seam added for exactly this: a test that calls
   `rebuild_palette_view` itself proves the function works and cannot prove
   anything calls it. Removing the rebuild from boot fails it with
   `primary is 1.32:1` — the roadmap's own headline number, which is the right
   thing for a failure message to say.

   The two `save_theme` sites want `Palette::name`, which is `&'static str` and
   unaffected; that is the one thing to check before changing the return type, and
   the reason to check it rather than assume.
3. **Prove it on a real light terminal.** A test that the arithmetic is right is
   not a screenshot, and 1380 green tests have already demonstrated how a correct
   module can be wired to nothing. This step needs the real app on a real light
   backdrop, and it is the only one left.

The one thing worth deciding before writing code: whether a user on a light
terminal should get an *automatic* adaptation or a `light = true` config setting
that also forces it. Automatic is friendlier and is what detection is for;
a setting is honest when detection is unreliable. Worth both — auto-detect, with
the setting to override — and the setting is nearly free once the machinery exists.

## Next

Neither is started, and both are larger than the item above.

### Mobile companion, Android

The serving half already ships and is tested, which the old entry understated by
half:

* `multitop --serve [<ADDR>]` and `--serve-token <TOKEN>` (`crates/multitop/src/main.rs`)
* `crates/multitop/src/server.rs` — 404 lines: bearer `check_auth`, `index`, and
  `/api/hosts`, `/api/health`, `/api/snapshot/:host`, `/api/history/:host`,
  `/api/mtop`, plus `spawn_collectors`, which reuses the agent's own
  `stream::connect` and `Payload::Hello` validation

So the remaining work is a client, and the transport is **HTTP** — the API above
— not the `WebSocket` this entry used to name. Nothing needs building that the
server does not already answer.

In the order it blocks:

1. **Auth hardening**, and it comes *first* now rather than last. It is the one
   item here that is a security surface rather than a feature, and a client
   multiplies the exposure: `--serve-token` is a bearer token in a header, and the
   listener binds a socket. A phone on a home network is the realistic deployment,
   which makes LAN exposure and a token leaked into a proxy log the questions to
   settle *before* a second client exists to lose the token. Constant-time
   comparison, no token in a log, and a decision about whether it binds loopback
   by default.
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

Tracked on request, sequenced against its trigger rather than a date. For the
current threat model — a device-local file read by a process holding the vault key
— harvest-now-decrypt-later is not the binding risk, because an attacker who can
store a copy of the vault can also wait for a build with a classical KEM. It earns
its place when the threat model changes, and these are the changes to watch for:

* the vault leaves the device — a synced file, a backup, a shared host. A copy is
  then durable and the argument above stops applying.
* the KEM protects something with a long confidentiality life, where the
  ciphertext is the thing being protected rather than the file it wraps.

Where it goes: the vault wraps host credentials with a symmetric key, so this is
key-agreement replacement at seal/open — `crates/multitop/src/vault.rs` and the
agent's equivalent — with the symmetric AEAD underneath unchanged. The handshake
that negotiates it is the agent's `Hello`, already version-gated, so a
KEM-capable agent and a legacy one coexist; that is a real advantage of doing it
here rather than in a new transport.

One note on ordering, since it is easy to get backwards: this needs a mature
implementation of each KEM, not a hand-rolled one. Preferring a crate with a
maintained implementation over a novel primitive is not a style preference here.

## Dropped, and why

Kept so the decision outlives the person who made it; a dropped item is not open
work, so it does not belong above.

| Dropped | Why |
|---------|-----|
| **Plugin SDK (WASM)** | Custom `exec` panels (`crates/multitop/src/config/types.rs`'s `custom_command`) cover it without a sandbox to get wrong. The cost of a plugin system is the isolation boundary, and nothing here needs one: a panel is a command the user already chose to run. Revisit if panels ever need to arrive from somewhere other than the user's own config file. |

## Where the invariants live

The hand-over notes this file used to carry — `Hello` first/valid/single, the
embedded agent matching the workspace version, the stable ad-hoc signature, the
vault's fallback, and "build with `./build.sh`" — are in
[DEVELOPMENT.md](../DEVELOPMENT.md#hello--agent-embedding), which is where a
developer looks for them and where they are kept current. The per-defect history
is [`detection-record.md`](detection-record.md); the general process rules are in
the house skills, not here.
