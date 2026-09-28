# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history (`git log --oneline`) and in `docs/detection-record.md`. When an item
here is finished, delete it rather than ticking it off.

## Next

Three items with enough stated to be started cold. Each names what it needs and
what is already true, so nobody rediscovers the state of the ground to find out
whether any of it exists.

### A palette for light terminals

**Partly done (2026-09-28).** The mechanism and a real palette defect are fixed;
background detection and the render wiring are not, so the item stays open.

Found 2026-09-23 screenshotting the Ops view on a light backdrop. All 8 themes
(`KARE`, `DRACULA`, `NORD`, `GRUVBOX`, `CATPPUCCIN`, `TOKYO_NIGHT`, `MONOKAI`,
`CYBERPUNK` in `crates/agent/src/color.rs`) are dark, so yellow — the `⚠` glyph,
the warn state — and cyan, which every host banner is drawn in, wash out to
near-invisible. This is true in every view, not just Ops.

Meaning survives; legibility does not. That matters because the UI house rule is
that no distinction may ride on one channel: a distinction carried by colour that
the background has eaten is not carried at all, so every place yellow or cyan is
the only signal is a legibility bug, not a style preference.

**Measured, not assumed.** `adapt_tests.rs` asserts the claim arithmetically for
all eight themes, so it cannot rot into an opinion and a ninth theme is covered by
the same assertions. On a near-white terminal the roles land at:

| role | contrast on light |
|------|-------------------|
| `text` | 1.03 – 1.34 : 1 |
| `primary` (the banner) | 1.35 – 2.05 : 1 |
| `meter_mid` (the warn yellow) | 1.09 – 1.95 : 1 |

**Done — the mechanism.** `crates/agent/src/surface.rs` is the colour arithmetic
(WCAG luminance and ratio, HSL, CIE76 ΔE) and `crates/agent/src/adapt.rs` is the
palette-level adaptation. One affine map on relative luminance, fitted across the
palette's own range, with the brightest role landing on the ratio threshold. It is
one mechanism for all eight themes rather than sixteen palettes, and it is
correct on a terminal whose background is an image, which per-theme variants
never can be.

**The obvious implementation was wrong, and the test says so.** Adjusting each
role independently until it reaches the floor drives every failing role to the
*same* luminance, so Kare's `primary` and `secondary` came out 1.01 : 1 apart on a
light terminal — the same colour. That buys legibility by spending a distinction.
`adapting_never_reduces_the_separation_two_roles_had` is the check, and it is
judged by ΔE rather than by contrast, because contrast is a function of luminance
alone and a cyan and a green at one lightness are 1.0 : 1 in contrast terms while
being obviously different colours.

**Done — a real defect this turned up, which was not on the roadmap.** Measuring
the palettes against *each theme's own* background (they are not all the same
value, and using one theme's for another asks a question about neither) found four
roles below the legibility floor in three shipped themes, and one below a
de-emphasis floor: Nord `secondary` 4.41 : 1, and `meter_high` at 3.05 : 1 in
Nord, 4.29 : 1 in Gruvbox, 3.93 : 1 in Monokai. `meter_high` is the colour for
high meters *and for alerts*, and the house rule is that alarm is for faults, so
an alert at 3.05 : 1 is the one colour on screen that must not be marginal. All
four were lifted along the hue line, keeping each theme's hue, and the floors are
now tests.

**Not done, and it is the part that makes this reach a terminal.** The background
is not detected yet. The obstruction is architectural and worth writing down: the
agent renders on the *remote* host but its escapes are interpreted by the *local*
terminal, so the remote side cannot answer the question for itself. Detection is
therefore `OSC 11` on the local side, and either the answer is passed to the
agent or the local side rewrites the escapes as frames pass through. Then
`adapt` has to be called from the render path, which is what the four
`reachability:` notes in the new modules record — they are honest about the debt
rather than hiding it behind a test that passes.

### Mobile companion, Android

Listed for years as "same `MTOP` over `WebSocket`, needs `Hello` + `Token`
hardening + `APNs`", which understated the state of the ground by half. The
serving half already ships and is tested:

* `multitop --serve [<ADDR>]` and `--serve-token <TOKEN>` (`crates/multitop/src/main.rs`)
* `crates/multitop/src/server.rs` — 404 lines: bearer `check_auth`, `index`,
  and `/api/hosts`, `/api/health`, `/api/snapshot`, `/api/history`,
  `/api/mtop_raw`, plus `spawn_collectors`, which reuses the agent's own
  `stream::connect` and `Payload::Hello` validation

So the remaining work is a client, and the transport it should speak is HTTP —
the API above — not the `WebSocket` the old entry named. Nothing needs building
that the server does not already answer.

What is actually open, in the order it blocks:

1. **A read-only Android app** over the existing `/api/*`. A phone is a glance
   surface: which hosts are up, what the last upgrade did, what is degraded. Not
   a terminal emulator, and not the TUI's key handling in a smaller window.
2. **Push.** APNs needs a device token, which means the app, so this is step 2
   rather than a parallel track. The trigger has to be the server noticing a
   transition — the polling model here is a client asking, which is fine for a
   glance surface and is why push is a genuine addition and not a latency tweak.
3. **Auth hardening**, and it is the one item here that is a security surface
   rather than a feature. `--serve-token` is a bearer token in a header, and the
   listener binds a socket. A phone on a home network is the realistic
   deployment, which makes LAN exposure and a token that can be leaked in a
   proxy log the questions to answer before this is exposed anywhere else.

The `repack`/`convert` logo pipeline (`scripts/repack_logos.py`,
`scripts/convert_logos.py`, operating on `crates/multitop/data/logos.bin.zst`)
already renders and repacks app icons, so an icon is not a blocker.

### Post-quantum KEM

Tracked on request. Recorded honestly so the sequencing is not a surprise later:
for the current threat model — a device-local file, read by a process holding
the vault key — a harvest-now-decrypt-later attack on that file is not the
binding risk, because an attacker who can store a copy of the vault can also
wait for a build with a classical KEM. The item earns its place when the threat
model changes, not before, and those are the changes to watch for:

* the vault leaves the device — a synced file, a backup, a shared host. A copy
  is then durable and the argument above stops applying.
* the KEM is used for something with a long confidentiality life, where the
  ciphertext is the thing being protected rather than the file it wraps.

Where it would go: the vault wraps host credentials with a symmetric key, so
this is a key-agreement replacement at seal/open — `crates/multitop/src/vault.rs`
and the agent's equivalent — with the symmetric AEAD underneath unchanged. Note
that the handshake that negotiates it is the agent's `Hello`, which is
version-gated already, so a KEM-capable agent and a legacy one can coexist; that
is a genuine advantage of doing it here rather than in a new transport.

Nothing above is a reason not to do it. It is a reason to pick the trigger
deliberately instead of discovering the need during an incident.

## Dropped, and why

Kept so the decision outlives the person who made it; a dropped item is not
open work, so it does not belong in the section above.

| Dropped | Why |
|---------|-----|
| **Plugin SDK (WASM)** | Custom `exec` panels (`crates/multitop/src/config/types.rs`'s `custom_command`) cover it without a sandbox to get wrong. The cost of a plugin system is the isolation boundary, and nothing here needs one: a panel is a command the user already chose to run. Revisit if panels ever need to arrive from somewhere other than the user's own config file. |

## Where the invariants live

The hand-over notes this file used to carry — `Hello` first/valid/single, the
embedded agent matching the workspace version, the stable ad-hoc signature, the
vault's fallback, and "build with `./build.sh`" — are in
[DEVELOPMENT.md](../DEVELOPMENT.md#hello--agent-embedding), which is where a
developer looks for them and where they are kept current. The per-defect history
is [`detection-record.md`](detection-record.md); the general process rules are
in the house skills, not here.
