# Homarr Tile URL Fix Plan

## Architecture

- **Server**: 192.168.0.33 running Docker with 42 services across compose/*.yml
- **VPN**: gluetun (`vpn`) — WireGuard to Switzerland via NordVPN, alias `vpn` on Docker DNS
- **Traefik**: on `local_network` at 172.25.0.13, listens on host ports 80/443, routes `*.media.home`
- **DNS**: 192.168.0.158 (AdGuard + Unbound), router delegates to it. `media.home` → 192.168.0.33
- **Homarr**: on 3 Docker networks — `local_network`, `vpn_network`, `socket_proxy`

### Two service classes

| Class | Examples | Network | Docker DNS resolvable from homarr? |
|-------|----------|---------|-------------------------------------|
| VPN-mode | sonarr, radarr, lidarr, readarr, jellyfin, prowlarr, qbittorrent, slskd, deemix, calibre-web, jellyseerr, flaresolverr, trawl-proxy, pot-provider, unpackerr, recyclarr | `network_mode: service:vpn` (share gluetun namespace) | NO (no individual netif) |
| Local-mode | navidrome, calibre, tdarr, cleanuparr, droppedneedle, actual, ryot, wger-*, sparkyfitness-* | `local_network` (own netif) | YES |

### Two probing perspectives

- **Phase 2 (host/LAN browser)**: probes `href` from the Docker host (`curl http://...`)
- **Phase 3 (inside homarr)**: probes `ping_url` from inside homarr container (Node.js http)

### Three ways to reach a service

| Method | Works from | Condition |
|--------|-----------|-----------|
| `https://SERVICE.media.home/` | LAN browser (via Traefik) | DNS resolves media.home → fails when user VPN is ON |
| `http://192.168.0.33:PORT/` | LAN browser (direct IP) | Only for ports published through Docker. Gluetun publishes: 8081, 5030-5031, 9696, 8686, 50300. NOT in gluetun: 8989, 7878, 8787, 8096, 8083, 5055, 6595. |
| `http://vpn:PORT/PATH/` | Inside homarr (via vpn_network) | Always — homarr + gluetun share vpn_network; Docker DNS alias `vpn` resolves |

## Problem

Current Homarr DB has wrong URLs because it was set by a defunct earlier iteration:

- `href` set to `http://172.19.0.2:PORT/` for many services — **172.19.0.2** is gluetun's internal IP on vpn_network, unreachable from the host. Phase 2 FAILs.
- `ping_url` set to `http://SONARR:8989/` etc. — **Docker names of VPN-mode services don't resolve** from homarr container (services share gluetun's namespace, no individual netif). Phase 3 FAILs.

**Current `set_homarr_tiles.py` TILES tuple has the same two bugs** — it was written to match the DB state, so running it re-applies the broken URLs.

Additional current-state bug discovered during review: **calibre-web has no published host port** (`ports: []` in compose), but the current TILES entry uses `http://{ip}:8099/` — port 8099 is not mapped anywhere. The tile's href is broken in its current state regardless of IP reachability.

## Solution

### 1. `href` — use `https://SERVICE.media.home/`

All services have Traefik routers configured (verified via Traefik API). DNS resolves `*.media.home` to 192.168.0.33, Traefik proxies to the correct back-end. This is the only way to reach VPN-mode services that gluetun doesn't publish host ports for.

**Caveat — user VPN ON**: When the user's client VPN is active, DNS queries bypass the LAN DNS server. `.home` is not a real TLD, so the VPN's resolver returns NXDOMAIN. Two approaches:

- **Server-side (recommended)**: Publish missing service ports through gluetun. Then href can use `http://192.168.0.33:PORT/` as a VPN-proof alternative. Pro: works regardless of DNS. Con: increases attack surface (direct port access bypasses Traefik's TLS/SSO).
- **Client-side (backup)**: VPN split DNS (route `*.media.home` to 192.168.0.158) or `/etc/hosts` entries. Fragile across clients.

Decision: start with `https://SERVICE.media.home/` for href. Add gluetun ports immediately to enable VPN-proof IP-based hrefs as a follow-up while the compose change is fresh (before phase 0-4 is deployed).

### 2. `ping_url` — reachable from inside homarr

| Service type | ping_url format | Examples |
|-------------|----------------|----------|
| VPN-mode (no own netif) | `http://vpn:PORT/PATH/` | sonarr:200, radarr:200, lidarr:200, jellyfin:302, readarr:200, calibre-web:302, jellyseerr:200, deemix:200 |
| Local-mode (own netif on local_network) | `http://SERVICE:PORT/PATH/` | actual:200, ryot:302, wger-nginx:302, sparkyfitness-frontend:200, navidrome:302 |
| External IP | Direct IP | brother, tp-link |

Note: gluetun has Docker DNS alias `vpn` on both `local_network` and `vpn_network`. Homarr is on both, so `http://vpn:PORT/` always resolves and reaches any port (Docker bridge allows inter-container traffic on all ports).

## Interservice communication map

**TL;DR: This change affects NOTHING beyond homarr itself.** Every other communication path uses independently-configured URLs. No service reads the homarr DB to configure itself.

### Which URLs are we changing

Only values in the homarr `app` table (`href`, `ping_url` columns). They are:
- **Written by**: `set_homarr_tiles.py`, `update_lan_ip.py`
- **Read by**: homarr (internal dashboard rendering + status pings), `check_homarr_links.py` (healthcheck probes), `update_lan_ip.py` (IP rewrite verification)
- **Used by no other service for any purpose**

### Every other URL in the stack (independent, NOT changed)

#### Docker DNS interservice URLs (compose env vars)

These reference services by Docker DNS name (`sonarr`, `jellyfin`, `vpn`, etc.) — they don't use tile URLs, hostnames, or IPs:

| Source | Target | URL | Mechanism |
|--------|--------|-----|-----------|
| jellyseerr | jellyfin | `http://jellyfin:8096` | Compose var `JELLYFIN_URL` |
| unpackerr | sonarr | `http://sonarr:8989` | Compose var `UN_SONARR_0_URL` |
| unpackerr | radarr | `http://radarr:7878` | Compose var `UN_RADARR_0_URL` |
| unpackerr | lidarr | `http://vpn:8686/lidarr` | Compose var `UN_LIDARR_0_URL` |
| prowlarr | flaresolverr | `http://localhost` | Compose var `FLARESOLVERR_HOST` |
| sparkyfitness-server | garmin | `http://sparkyfitness-garmin:8000` | Compose var `GARMIN_MICROSERVICE_URL` |

#### OIDC/external hostname URLs (compose env vars)

These use `*.media.home` hostnames but are configured PER-SERVICE via their own env vars — not tile URLs:

| Service | Var | Value |
|---------|-----|-------|
| actual | `ACTUAL_OPENID_DISCOVERY_URL` | `https://auth.media.home` |
| actual | `ACTUAL_OPENID_SERVER_HOSTNAME` | `https://actual.media.home` |
| navidrome | `ND_EXTAUTH_LOGOUTURL` | `https://auth.media.home/logout` |
| ryot | `FRONTEND_URL` | `https://ryot.media.home` |
| ryot | `SERVER_OIDC_ISSUER_URL` | `https://auth.media.home` |
| firefly | `APP_URL` | `https://firefly.media.home` |
| homarr | `AUTH_OIDC_ISSUER` | `https://auth.media.home` |

All of these already work before the tile change. Changing tile hrefs has zero effect on them. They share DNS dependence (`*.media.home`) but NOT tile URL format.

#### Healthcheck URLs (compose healthcheck.test)

Every container's healthcheck uses `http://localhost:PORT/` — Docker-internal, no hostname dependency:

| Service | Healthcheck URL | Published ports (on host) |
|---------|----------------|---------------------------|
| qbittorrent | `http://localhost:8081` | `8081:8081` (via gluetun) |
| prowlarr | `http://localhost:9696/prowlarr/ping` | `9696:9696` (via gluetun) |
| lidarr | `http://localhost:8686/lidarr/ping` | `8686:8686` (via gluetun) |
| sonarr | `http://localhost:8989/ping` | NONE |
| radarr | `http://localhost:7878/ping` | NONE |
| readarr | `http://localhost:8787/readarr/ping` | NONE |
| jellyfin | `http://localhost:8096/health` | NONE |
| navidrome | `http://localhost:4533/ping` | `4533:4533` (own compose) |
| jellyseerr | `http://localhost:5055/api/v1/status` | NONE |
| cleanuparr | `http://localhost:11011/health` | `11011:11011` (own compose) |
| tdarr | `http://localhost:8265` | `8265:8265` (own compose) |
| droppedneedle | `http://localhost:8688/health` | `8688:8688` (own compose) |
| actual | `http://localhost:5006/` | `5006:5006` (own compose) |
| firefly | `http://localhost:8080/health` | NONE |
| ryot | `http://localhost:8000/health` | NONE |
| wger-web | `http://localhost:8000` | NONE |
| calibre | `http://localhost:8080` | `8086:8080`, `8085:8085` |

Note: VPN-mode services without published host ports (sonarr, radarr, readarr, jellyfin, jellyseerr) have healthchecks that run INSIDE the shared gluetun namespace — they use `localhost:PORT` which works because the process listens on that port inside the shared netns.

#### Script-level defaults (NOT tile URLs)

| Script | Default URL | Mechanism |
|--------|-------------|-----------|
| `sync_wger_routines.py` | `http://127.0.0.1:8000` | Env var `WGER_URL` |
| `sync_wger_routines.py` | `http://127.0.0.1:3004` | Env var `SPARKYFITNESS_URL` |
| `clean_slskd_queue.py` | `http://localhost:5030` | Env var `SLSKD_BASE_URL` |
| `jellyfin_auth_provider.py` | `http://127.0.0.1:8096/jellyfin` | Env var `JELLYFIN_URL` |
| `lldap_api.py` | `http://127.0.0.1:17170` | Env var `LLDAP_URL` |
| `check_homarr_links.py` | `http://127.0.0.1:8080` | Env var `TRAEFIK_API` |
| `check_sso.py` | `http://127.0.0.1:8096/jellyfin` | Hardcoded for JF direct probe |
| `change_user_password.py` | `http://localhost:17170` | Hardcoded for lldap |
| `ensure_calibre_path_mapping.py` | `http://localhost:{port}` | Localhost for calibre |
| `check_arr_proxy_egress.py` | `http://{host}:{port}/` | Docker exec, per-arr API |
| `check_indexer_connectivity.py` | `http://{host}:{port}/` | Docker exec, per-indexer |
| `check_vpn_children.py` | `http://localhost:{port}` | Docker exec, per-service |

All use Docker-internal or loopback addresses. None depend on tile URLs.

#### Traefik labels (NOT affected)

Every service's Traefik configuration uses `loadbalancer.server.port=N` (Docker internal port number) and `Host(`X.media.home`)` (the hostname). These are already correct and don't reference tile URLs.

#### Hardcoded infrastructure IPs (NOT affected)

- `trust_local_ca.sh`: `ztomer@192.168.0.33` (SSH target) and `http://192.168.0.33:3004/ca.crt` (CA certificate download via sparkyfitness-frontend)
- `check_sso.py`: `LAN_IP = E.get("LAN_IP", "192.168.0.33")` (fallback, override by env)

Both are independent of tile URLs.

### Who reads the homarr DB?

Only four code paths touch the homarr `app` table:
1. **`set_homarr_tiles.py`** — writes `href` and `ping_url` for every tile (what we're changing)
2. **`update_lan_ip.py`** — replaces `//<old_ip>:` → `//<new_ip>:` in both columns (will catch fewer rows after change)
3. **`check_homarr_links.py`** — reads `name, href, ping_url` from `app` table, probes each URL from host (href) and inside container (ping_url)
4. **`check_homarr_board_model.py`** — reads `section` table ONLY (board layout), does NOT touch `app` table at all

No other script reads homarr tiles. No service configures itself from homarr data.

### Verdict: isolated change

The tile URL change is SHIELDED — it affects only homarr's display and status pings. Every service-to-service communication path has its own independent URL configuration (compose env vars, script defaults, Docker DNS names, localhost healthchecks). None reference homarr tiles.

The ONE shared dependency is DNS: both tile hostnames (`https://SERVICE.media.home/`) and some OIDC config vars (`https://auth.media.home/`) require `*.media.home` to resolve. But the OIDC vars were already using hostnames before the change — the tile change doesn't add new DNS dependencies.

### Appendix: URLs in service DBs and config files

Every service's own SQLite database and config file was queried for stored URLs/IPs. Results:

#### Service DBs (queried with sqlite3)

| Service DB | URL/IP Columns Found | Type | Affected by tile change? |
|-----------|---------------------|------|--------------------------|
| sonarr.db | `RemotePathMappings.Host: localhost` | SSHFS remote mount | No |
| radarr.db | `RemotePathMappings.Host: localhost` | SSHFS remote mount | No |
| radarr.db | `MovieMetadata.Website: https://...` | External movie pages (marvel.com, disney.com) | No |
| lidarr.db | `RemotePathMappings.Host: localhost` | SSHFS remote mount | No |
| readarr.db | (no URL/IP columns found) | — | No |
| prowlarr.db | (no URL/IP columns found) | — | No |
| navidrome.db | `player.ip: 192.168.0.x/172.25.0.x` | Client connection logs | No |
| navidrome.db | `artist.image_url: https://cdn-images.dzcdn.net/...` | External deezer CDN | No |
| navidrome.db | `artist.external_url: https://...` | External artist sites | No |
| navidrome.db | `radio.stream_url: https://...` | External radio streams | No |
| jellyfin.db | `BaseItemImageInfos.Path: https://image.tmdb.org/...` | External image TMDB/CDN | No |
| jellyfin.db | `BaseItems.Path: /media/...` | Media file paths (local mount) | No |
| cleanuparr.db | `download_clients.host: http://vpn:8081/` | qbittorrent via Docker DNS `vpn` | No |
| cleanuparr.db | `arr_instances.url: http://sonarr:8989/` | Docker DNS service name | No |
| cleanuparr.db | `arr_instances.url: http://radarr:7878/` | Docker DNS service name | No |
| cleanuparr.db | `arr_instances.url: http://vpn:8686/lidarr/` | Docker DNS via `vpn` alias | No |
| calibre-web.db | (no URL/IP columns found) | — | No |

**Notable**: cleanuparr stores its interservice URLs in its OWN DB (written from compose env vars on init). These use Docker DNS names (`sonarr`, `radarr`, `vpn`) — same as the compose env vars. They are NOT tile URLs.

#### Service config files (grep for .media.home and 192.168.0.33)

| Config file | URL/IP reference | Type |
|------------|------------------|------|
| `authelia/config/configuration.yml:189` | `http://192.168.0.33:7575/api/auth/callback/oidc` | OIDC redirect URI for homarr (IP-based) |
| `authelia/config/configuration.yml:100` | `https://auth.media.home` | Authelia's own URL |
| `authelia/config/configuration.yml:101` | `https://dashboard.media.home` | Default redirect after login |
| `authelia/config/configuration.yml:165,188,203,222` | `https://*.media.home/...` | OIDC callback URIs for actual, homarr, ryot, sparkyfitness |
| `traefik/conf/dynamic.yml:4-5` | `media.home.crt/key` | TLS certificate paths |
| `traefik/conf/dynamic.yml:14,47` | `Host(`*.media.home`)` | Traefik router rules |
| `wger/prod.env:22` | `https://wger.media.home,http://192.168.0.33:8000` | Django CSRF trusted origins |
| `wger/prod.env:138` | `https://wger.media.home` | Django SITE_URL |

The **only IP-based reference that matters for server portability** is `http://192.168.0.33:7575/api/auth/callback/oidc` in Authelia config — it's the OIDC redirect URI for homarr's login callback. This is NOT a tile URL and NOT changed by this plan. It must remain IP-based because the OIDC protocol redirects the browser to this URL (which may be on a VPN-connected client that can't resolve `*.media.home`). If the server's LAN IP changes, this URI needs updating — but `update_lan_ip.py` currently does NOT handle Authelia config. This is a separate gap.

#### Summary

Every URL/IP stored in service databases and config files falls into one of these categories:
- **Docker DNS names** (`sonarr`, `radarr`, `vpn`, `localhost`) — unaffected by IP or tile changes
- **External URLs** (TMDB, deezer CDN, radio streams, artist websites) — outside the local domain
- **`*.media.home` hostnames** (authelia, traefik, wger) — configured independently of tile URLs
- **Hardcoded IPs** (Authelia OIDC callback, wger CSRF, CA download URL) — independent of tile URLs; `update_lan_ip.py` handles wger but NOT Authelia

**None of these are tile URLs.** The tile URL change does not cascade into any service DB or config file.

## Implementation

### Phase 0: Add gluetun port publishing (BEFORE tile changes)

Add missing ports to gluetun in `compose/media.yml`:
```
- 8989:8989    # sonarr
- 7878:7878    # radarr
- 8787:8787    # readarr
- 8096:8096    # jellyfin
- 8083:8083    # calibre-web
- 5055:5055    # jellyseerr
- 6595:6595    # deemix
```

Why phase 0 (not phase 5): (a) adding ports to gluetun requires restarting ALL 16 VPN-mode services — doing this later means additional downtime after tiles are fixed; (b) with these ports published, both hostname hrefs AND IP-based hrefs work, so we can toggle between them without another compose restart; (c) the current IP-based hrefs in the DB for these services are already broken (unpublished ports), so gluetun has no correct port list — this fixes the current state too.

Risk: gluetun restart takes ~3 minutes (VPN reconnection). All VPN-mode services are down during that window. Schedule during maintenance.

### Phase 1: Investigate calibre timeout

Verify `http://calibre:8080/` from inside homarr. Calibre is on `local_network` behind Traefik. If it consistently times out, use the published host port (`http://192.168.0.33:8086/`) for ping_url. Note: calibre serves selkies (VNC) on 8080 through Traefik at `calibre.media.home`; the health endpoint may not respond to plain HTTP from non-Traefik sources.

### Phase 2: Update `scripts/system/set_homarr_tiles.py`

Rewrite the `TILES` tuple. Key changes:

- **href**: `http://{ip}:PORT/` → `https://SERVICE.media.home/` for ALL Traefik-routed services
- **href**: secure-context hostname exceptions (Calibre, Actual Budget, Firefly III, Traefik) kept as-is
- **href**: external device IPs (tp-link, brother) kept as-is
- **href**: Calibre Content Server stays `http://{ip}:8085/` (published through calibre compose, NOT gluetun; reachable from host) — this is the canary that keeps `update_lan_ip.py` verify_targets working
- **ping_url**: `http://SERVICE:PORT/` → `http://vpn:PORT/` for all VPN-mode services (sonarr, radarr, lidarr, readarr, jellyfin, calibre-web, jellyseerr, deemix, qbittorrent, prowlarr, slskd)
- **ping_url**: local-mode services keep docker DNS names as-is
- **ping_url**: Traefik → `http://traefik:8080/api/version` (not /ping, which returns 404)

Also add: pre-flight DNS check that resolves each `SERVICE.media.home` before writing, so a broken DNS state doesn't silently kill all tiles.

### Phase 3: Update tests

- `checks/tests/test_scripts_homarr_tiles.py` — LIVE_TILES must match new TILES tuple. The `SECURE_CONTEXT_TILES` and `HOSTNAME_EXCEPTIONS` assertions stay valid.
- `checks/tests/test_scripts_system_lan_ip.py` — sandbox() fixture tile URLs must match new format. The canary assertion (Chaptarr IP-word-boundary check) stays valid.

### Phase 4: Update `update_lan_ip.py` for hostname tiles

Two changes needed:

1. **`verify_targets()`**: After hostname hrefs, `homarr_count(db, f"//{new_ip}:")` only matches the Calibre Content Server IP tile (preserved as canary). If that tile is ever removed or changed, verify_targets silently reports 0 matches. Add a secondary check: count `.media.home` tiles matching a known hostname pattern, OR assert that the canary tile survives.

2. **`rewrite_homarr()`**: Already correct — it replaces `//<old_ip>:` where present. With hostname hrefs, this is a no-op for most tiles (correct — no rewrite needed). Keep the function; it still handles Calibre Content Server and any future IP-based tiles.

### Phase 5: Apply DB update and verify

Run `set_homarr_tiles.py`, then run `check_homarr_links.py` and verify all three phases pass. Restart homarr.

## Risk Review

### R1: IP-vs-hostname cycle

`set_homarr_tiles.py` was created specifically to move AWAY from `*.media.home` hostnames because "a client VPN replaces the resolver, .home is not a real TLD, so the VPN's resolver NXDOMAINs it" (module docstring). The plan switches back.

**If user VPN use is unchanged, NXDOMAIN will return for any client on VPN.**

**Mitigation**: Phase 0 publishes missing gluetun ports, enabling a fast pivot: if hostname hrefs cause VPN problems, switch to `http://{ip}:PORT/` with a one-line change in set_homarr_tiles.py (no compose restart needed — ports are already published). The gluetun port list should be the single source of truth for which services are IP-accessible.

### R2: Current IP-based hrefs are partially broken

Gluetun only publishes: 8081, 5030-5031, 9696, 8686, 50300. The current IP-based hrefs for sonarr (8989), radarr (7878), readarr (8787), jellyfin (8096), calibre-web (8083), jellyseerr (5055), and deemix (6595) point at ports that don't reach their target. These tiles are **already broken** when clicked from a LAN browser — "connection refused" or timeout.

**Mitigation**: Phase 0 fixes this (ports published). Then both hostname and IP formats work.

### R3: update_lan_ip.py verify_targets silent gap

`verify_targets()` checks `homarr_count(db, f"//{new_ip}:")` — after hostname hrefs, only Calibre Content Server (`http://{ip}:8085/`) matches. If someone changes that tile's format, verify_targets reports 0 matches and the script claims "no homarr tile points at new_ip" even though hostname tiles are correct.

**Mitigation (phase 4)**: Keep Calibre Content Server IP-based (locked by test assertion). Add a secondary verify check that counts `//.media.home/` tiles matching expected hostnames.

### R4: set_homarr_tiles.py has no DNS pre-flight

The script reads `LAN_IP` from .env and applies tiles. If DNS is broken (AdGuard down, network issue) when the script runs, all tiles are written with hostnames that don't resolve → every tile href is dead. The script reports success.

**Mitigation (phase 2)**: Add a pre-flight check that resolves each `https://SERVICE.media.home/` hostname before writing. Fail early if any NXDOMAIN, with a clear message. This catches DNS issues at the point of change, not after.

### R5: Traefik catch-all silent failure

Homarr has a `PathPrefix(/)` catch-all at priority 1. Any `*.media.home` hostname without a matching router returns 200 with Homarr's dashboard content. The user sees a working tile (green ping) that opens... Homarr instead of the app. With hostname hrefs, EVERY tile depends on correct Traefik routing for its specific hostname.

**Mitigation**: `check_homarr_links.py` phases 1 (router table) and 2 (body content check) detect this. Run before/after the change. Additionally, the phase 2 DNS pre-flight in set_homarr_tiles.py can verify that each hostname returns a non-Homarr response before writing.

### R6: Firefly III TLS trust

Firefly III is on its own isolated network. Its ping_url is `https://firefly.media.home/` (through Traefik). From inside homarr, Node's https module may reject the private CA certificate (UNABLE_TO_VERIFY_LEAF_SIGNATURE), causing the tile to show DOWN while the app works fine.

**Mitigation**: Test with check_homarr_links.py phase 3 before applying. If it fails, either mount the CA cert into homarr's container or use a non-TLS health endpoint on Firefly's isolated network.

### R7: calibre-web no published ports

In the current compose, calibre-web has `ports: []` but uses `network_mode: service:vpn` with port 8083. The current TILES entry uses `http://{ip}:8099/` for the href. Port 8099 is not published anywhere — this tile's href is broken in the current system. Nobody noticed because the ping_url (`http://calibre-web:8083/`) works from inside homarr and the dashboard shows green, but clicking the tile goes nowhere.

**Mitigation**: Phase 0 adds 8083 to gluetun ports (consistent with calibre-web's actual port). Phase 2 sets href to `https://calibre-web.media.home/` (published through Traefik regardless of gluetun ports).

### R8: Calibre Content Server sole canary fragility

After the change, Calibre Content Server (`http://{ip}:8085/`) is the ONLY tile guaranteed to contain the LAN IP. If a future edit switches it to hostname format, `update_lan_ip.py`'s `verify_targets()` loses all IP anchors and reports "STALE: no homarr tile points at {new_ip}" after every LAN IP change, even though hostname tiles are fine.

**Mitigation (phase 4)**: Add a second verify_targets check that accepts hostname-based tiles and reports "verify skipped: no IP-based tiles remain" rather than a hard "STALE" failure. The check should verify that `.media.home` hostnames resolve, not that they contain the IP.

### R9: Gluetun port publishing is disruptive

Adding 7 ports to gluetun requires `docker compose up -d gluetun`, which restarts gluetun. Because all 16 VPN-mode services use `network_mode: service:vpn`, they ALL restart when gluetun restarts (Docker dependency chain). Total downtime: ~3-5 minutes.

**Mitigation**: Schedule during maintenance window. The alternative (adding ports one at a time later) causes MORE total downtime — each gluetun restart takes down all 16 services.

### R10: calibre 8080 timeout

Plan already flags this. If `http://calibre:8080/` doesn't respond from inside homarr, fall back to `http://192.168.0.33:8086/` (published host port 8086 → container port 8080). Verify during phase 1.

### R11: Traefik ping_url uses wrong path

Current TILES has `http://traefik:8080/ping` which returns 404. Traefik serves its API on port 8080 with no `/ping` route. The correct health path is `http://traefik:8080/api/version` (returns 200).

**Mitigation**: Already in the plan — change to `/api/version`. Verify with phase 3 probe.

### R12: slskd healthcheck start_period

slskd has a 15-minute healthcheck start_period. During that window, the container is "starting" and `http://vpn:5030/` may not respond. `check_homarr_links.py` handles this (`any_starting('slskd')` → warning, not failure). Acceptable.

## Final desired state

| Tile name | href | ping_url |
|-----------|------|----------|
| Sonarr | `https://sonarr.media.home/` | `http://vpn:8989/` |
| Radarr | `https://radarr.media.home/` | `http://vpn:7878/` |
| Lidarr | `https://lidarr.media.home/` | `http://vpn:8686/lidarr/` |
| Chaptarr (Readarr) | `https://readarr.media.home/` | `http://vpn:8787/readarr/` |
| Jellyfin | `https://jellyfin.media.home/` | `http://vpn:8096/health` |
| Qbittorrent | `https://qbittorrent.media.home/` | `http://vpn:8081/` |
| Prowlarr | `https://prowlarr.media.home/` | `http://vpn:9696/prowlarr/` |
| Calibre-Web Automated | `https://calibre-web.media.home/` | `http://vpn:8083/` |
| Seerr (Jellyseerr) | `https://seerr.media.home/` | `http://vpn:5055/api/v1/status` |
| slskd | `https://slskd.media.home/` | `http://vpn:5030/` |
| Deemix | `https://deemix.media.home/` | `http://vpn:6595/` |
| Cleanuparr | `https://cleanuparr.media.home/` | `http://cleanuparr:11011/` |
| Tdarr | `https://tdarr.media.home/` | `http://tdarr:8265/` |
| Navidrome | `https://navidrome.media.home/` | `http://navidrome:4533/` |
| Calibre | `https://calibre.media.home/` | `http://calibre:8080/` (verify — see phase 1) |
| Calibre Content Server | `http://192.168.0.33:8085/` | `http://calibre:8085/` |
| DroppedNeedle | `https://droppedneedle.media.home/` | `http://droppedneedle:8688/` |
| Actual Budget | `https://actual.media.home/` | `http://actual:5006/` |
| Firefly III | `https://firefly.media.home/` | `https://firefly.media.home/` |
| Ryot | `https://ryot.media.home/` | `http://ryot:8000/` |
| SparkyFitness | `https://fitness.media.home/` | `http://sparkyfitness-frontend:80/` |
| wger | `https://wger.media.home/` | `http://wger-nginx:80/` |
| Traefik | `https://traefik.media.home/` | `http://traefik:8080/api/version` |
| Owntone | `https://owntone.media.home/` | (none) |
| AdGuard | `https://adguard.media.home/` | (none) |
| tp-link | `http://192.168.0.1/webpages/index.html#/login` | (none) |
| brother | `http://192.168.0.250/` | (none) |

Note: Traefik routers use specific hostnames — `fitness.media.home` for SparkyFitness, `seerr.media.home` for Jellyseerr, `calibre-server.media.home` for Calibre Content Server, `calibre.media.home` for Calibre.
