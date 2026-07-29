# multitop Roadmap & Feature Specifications

This roadmap documents upcoming features, UX enhancements, and test suite expansion for `multitop`.

## 1. Regression Tests (`a`)
- **Upgrade Flow Regression Tests**: Verify modal trigger, confirmation via `u`/`U` / `uu`, and cancellation behavior.
- **SSO Credential Storage Tests**: Verify master password setting, single keychain read/write, fallback to SSO when individual server password is empty, and mock keychain integration.
- **State Persistence Tests**: Test loading, saving, and updating `state.toml` (located next to `config.toml`) for tracking last update timestamp.
- **Sparkline Rendering Tests**: Test updating memory/CPU sparkline histories and formatting header line with `M:` and `C:` prefixes.
- **Header Username Consistency Tests**: Verify that Monitor, Docker, Fetch, and Upgrade views consistently render `user@host` when username is provided.

## 2. Update Confirmation Modal & Last Update Persistence (`b`)
- **Confirmation Modal**:
  - Pressing `u` or `U` opens an interactive confirmation modal dialog:
    > "Updates can be slow and potentially destructive."
    > "Are you sure you want to run updates on all servers?"
    > "Last update: <Timestamp / Never>"
    > "Press 'u' or Enter to confirm, Esc to cancel."
  - Pressing `u` again (or `uu` in rapid succession), or pressing Enter / `y` confirms and executes `run_upgrade()` across all configured servers.
  - Pressing Esc, `q`, or `n` cancels the modal without triggering updates.
- **Last Update Persistence (`state.toml`)**:
  - Save execution timestamp upon update confirmation in a `state.toml` file located in the same directory as `config.toml`.
  - Load `state.toml` on app startup / modal presentation to display when the last update was completed (e.g. "Last update: 2026-07-29 17:46:00" or relative time).

## 3. Single Sign-On (SSO) & Single Touch ID / Fingerprint Unlock (`c`)
- **Single Sign-On (SSO) Master Password**:
  - Add support for a single master sudo password (`__sso_master__`) in system keychain.
  - Gating under single Touch ID / fingerprint prompt: Loading `__sso_master__` accesses the OS credential store once, unlocking all target servers without requesting separate authentications per machine.
  - In Settings (`passwords.rs` / `config_ui.rs`), provide a "Single Sign-On (SSO) Master Password" option to set a single password that applies across all servers.
  - Server-specific password override remains supported for machines requiring distinct passwords.

## 4. Visible Sparkline Integration in Border / Header (`d`)
- **Sparkline History Tracking**:
  - Track rolling memory usage % (`sparklines_mem`) and aggregate CPU usage % (`sparklines_cpu`) per server panel in `App`.
  - Push values on each telemetry frame (`Payload::Monitor`).
- **Border / Header Rendering**:
  - Render Memory sparkline on the left of server name with `M:` prefix (e.g., `M: ▂▃▅`).
  - Render CPU sparkline on the right of server name with `C:` prefix (e.g., `C: ▃▅▇█`).
  - Layout example: `M: ▂▃▅  user@server-01  C: ▃▅▇█`

## 5. Username Display Consistency (`e`)
- **Consistent `user@host` Headers**:
  - Standardize header displays across all views (Monitor, Docker, Fetch, and Upgrade).
  - Use `user@host` whenever a username is configured, falling back to `host` when username is empty.
  - Resolves discrepancy where Fetch displayed `user@host` but Monitor/Docker displayed only `host`.

## 6. UX & SSO Performance Refinements
- **Keybar Mode Highlights**:
  - Restrict active state background highlighting strictly to the mode letters (`Fetch`, `Docker`, `Stats`, `Upgrade`) without highlighting trailing margin spaces.
- **Top Border Username & Hostname**:
  - Guarantee that `user@host` (or `host` if user is empty) is centered and displayed on panel top borders alongside `M:` and `C:` sparklines across all window sizes.
- **Single-Prompt SSO Credential Resolution**:
  - Check `load_sso()` first during password loading to eliminate redundant OS keychain lookups for non-existent per-server account entries.
  - Ensures a single fingerprint / Touch ID prompt unlocks all servers without multiple password entry prompts.

## 7. Release 0.19.0 & Optional Experimental Sparklines
- **Sparklines Disabled by Default**:
  - Sparklines are disabled by default on startup.
- **Settings Toggle (`P`)**:
  - Added `Sparklines (Experimental): Enabled / Disabled` setting toggle in the configuration screen (`'e'`). Pressing `P` toggles header sparklines and persists `show_sparklines = true/false` in `config.toml`.
- **Release Version**:
  - Bumped workspace version to `v0.19.0`.

## 8. Release 0.20.1 & View Keybar Reordering
- **Keybar Hint Sequence**:
  - Reordered bottom keybar mode options to `Stats Docker Fetch Upgrade` (`[S Stats] [D Docker] [F Fetch] [U Upgrade]`).
- **View Selection Stability**:
  - Pressing active selection keys stays in current view without re-triggering network fetches.
- **Release Version**:
  - Bumped workspace version to `v0.20.1`.
