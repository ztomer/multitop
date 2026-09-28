# Roadmap

The one forward-looking backlog. Shipped work is not listed here — it is in git
history (`git log --oneline`) and in `docs/detection-record.md`. When an item
here is finished, delete it rather than ticking it off.

## Next

Three items with enough stated to be started cold. Each names what it needs and
what is already true, so nobody rediscovers the state of the ground to find out
whether any of it exists.

### A palette for light terminals

Found 2026-09-23 screenshotting the Ops view on a light backdrop. All 8 themes
(`KARE`, `DRACULA`, `NORD`, `GRUVBOX`, `CATPPUCCIN`, `TOKYO_NIGHT`, `MONOKAI`,
`CYBERPUNK` in `crates/agent/src/color.rs`) are dark, so yellow — the `⚠` glyph
and the warn state — and cyan, which every host banner is drawn in, wash out to
near-invisible on a light terminal. This is true in every view, not just Ops.

Meaning survives; legibility does not. That matters because the UI house rule
is that no distinction may ride on one channel: a distinction carried by colour
that the background has eaten is not carried at all, so every place yellow or
cyan is the only signal is a legibility bug, not a style preference.

Two shapes of fix, and the second is much cheaper:

* a light variant per theme — 8 themes × the whole palette, and a design pass
  across every view, since nothing today distinguishes "the theme's background"
  from "the terminal's background"
* background detection — ask what the terminal is actually painting, and derive
  a foreground that survives it. One mechanism instead of 16 palettes, and it is
  correct on a terminal whose background is an image, which per-theme variants
  never can be.

The second is the one worth building. The obstruction to check first: the
palette is a fixed table of ANSI escapes, so a derived variant needs a place to
live that is not a table entry.

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
