# E2E Test Gap Analysis

**Date:** 2026-07-31  
**Scope:** All source modules in `crates/multitop/src/`  
**Current Test Count:** ~130 tests (22 inline unit + ~110 E2E/integration)

---

## Summary

| Category | Modules | Test Status |
|----------|---------|-------------|
| **Zero Coverage** | 14 modules | 0 tests |
| **Partial Coverage** | 4 modules | 1-2 tests each |
| **Well Covered** | 2 modules | 18+ tests |

---

## Zero Coverage Modules (14)

### 1. `ssh.rs` — **CRITICAL GAP**
**Functions:** `is_local()`, `spawn_local_agent()`, `spawn_command()`, `ssh_command_tty()`, `upload_agent()`, `agent_path()`, `bootstrap_script()`, `wrap_with_upgrade_lock()`, `wrap_with_local_upgrade_lock()`, `parse_need_agent()`, `cleanup_old_agents_command()`, `any_agent_embedded()`

**Missing Tests:**
- Local vs remote detection (`is_local()`)
- Local shell command wrapping (with/without password, with/without mock)
- Remote SSH command construction (with/without sudo password)
- Upgrade lock wrapper (local + remote variants)
- Agent binary deployment (`upload_agent()`)
- Agent path resolution (`agent_path()`)
- Bootstrap script generation (`bootstrap_script()`)
- Need-agent parsing (`parse_need_agent()`)
- Cleanup command generation (`cleanup_old_agents_command()`)
- Embedded agent detection (`any_agent_embedded()`)

**Risk:** SSH command execution is the primary attack surface. Upgrade lock logic prevents concurrent upgrades — untested = potential data corruption.

---

### 2. `config.rs` — **CRITICAL GAP**
**Functions:** `load()`, `parse()`, `save_servers()`, `save_theme()`, `save_show_sparklines()`, `parse_ssh_config()`, `validate_host()`, `validate_user()`, `Server::target()`

**Missing Tests:**
- Config file loading (valid, missing, malformed TOML)
- SSH config parsing (multiple hosts, aliases, wildcards)
- Host/user validation (edge cases: Unicode, length, special chars)
- Server target string generation (with/without user, port)
- Config persistence (servers, theme, sparklines)

**Risk:** Configuration is the source of truth for all connections. Parsing bugs = connection failures or security issues.

---

### 3. `passwords.rs` — **IMPORTANT GAP**
**Types:** `ConfigSection`, `ServerDraft`, `PasswordManager`, `PasswordAction`  
**Functions:** `open()`, `handle_key()`, `server_key()`, `password_key()`, `ServerDraft::new()`, `ServerDraft::into_server()`, `ServerDraft::active_field()`

**Missing Tests:**
- ConfigSection tab switching
- ServerDraft field navigation (Tab/Up/Down)
- ServerDraft validation (host, user, port, upgrade_cmd)
- Password entry (SSO vs override, empty password handling)
- Server add/edit/delete flows
- Draft cancellation (Esc)
- Sparkline toggle action

**Risk:** Configuration UI is the main user interaction surface. Key handling bugs = unusable config panel.

---

### 4. `password_actions.rs` — **IMPORTANT GAP**
**Function:** `apply()` — handles all `PasswordAction` variants

**Missing Tests:**
- `ApplyServers` — server list persistence + panel sync
- `SaveServerWithPassword` — combined server + password save
- `Delete` — password removal + keychain cleanup
- `SaveSso` — SSO password propagation to all panels
- `DeleteSso` — SSO removal
- `ToggleSparklines` — config persistence
- `Save` — password storage + vault sync + upgrade resumption

**Risk:** This is the "write path" for all credential changes. Bugs here = lost passwords, failed upgrades, vault sync issues.

---

### 5. `tasks.rs` — **IMPORTANT GAP**
**Functions:** `spawn_upgrade()`, `spawn_fetch()`, `spawn_docker()`, `spawn_monitor()`

**Missing Tests:**
- Upgrade task spawning (generation tracking, aux flags)
- Fetch task spawning
- Docker task spawning
- Monitor task spawning
- Task cancellation on panel switch

**Risk:** Task management drives all async operations. Generation bugs = stale results, memory leaks.

---

### 6. `panel.rs` — **GAP**
**Functions:** `new()`, `ensure_sudo_password()`, `set_sudo_password()`, `show_last_frame()`

**Missing Tests:**
- Panel initialization with server config
- Sudo password loading (from keychain, from vault, none)
- Password setting (session vs vault vs keychain)
- Frame history management

**Risk:** Panel is the per-connection state container. State bugs = display corruption, credential leaks.

---

### 7. `modals.rs` — **GAP**
**Functions:** `draw_upgrade_modal()`, `draw_vault_password_prompt()`

**Missing Tests:**
- Upgrade modal content (interrupted state, last update time)
- Vault password prompt (dots masking, error display)
- Layout at various terminal sizes
- Theme color application

**Risk:** Modals are security-critical (password entry, destructive action confirmation). Visual bugs = user confusion, security exposure.

---

### 8. `config_ui.rs` — **GAP**
**Function:** `draw()`

**Missing Tests:**
- Servers section rendering (list, selection, draft editing)
- Passwords section rendering (status icons, SSO/override)
- Draft field highlighting
- Notice/error display
- Theme color application

**Risk:** Full-screen config UI. Rendering bugs = unusable configuration.

---

### 9. `ui.rs` — **GAP**
**Functions:** `draw()`, layout calculation, panel rendering, keybar, sparklines

**Missing Tests:**
- Multi-panel layout (horizontal/vertical splits)
- Panel content rendering (monitor, docker, fetch, upgrade modes)
- Keybar rendering
- Sparkline rendering (enabled/disabled)
- Scroll handling
- Theme application

**Risk:** Main UI rendering. Layout bugs = overlapping panels, clipped content, crashes.

---

### 10. `ssh_opts.rs` — **GAP**
**Types:** `Arch`  
**Functions:** `from_uname()`, `binary()`, `hash()`, `label()`, `word()`, `sh_quote()`

**Missing Tests:**
- Architecture detection from `uname -m` (x86_64, aarch64, armv7, etc.)
- Binary selection per arch
- Hash/label/word consistency
- Shell quoting (single quotes, escaped quotes, empty string)

**Risk:** Agent deployment fails silently on unsupported architectures. Shell quoting bugs = command injection.

---

### 11. `fmt.rs` — **GAP**
**Functions:** `error_line()`, `status_line()`, `header_line()`, `unixtime_to_str()`

**Missing Tests:**
- ANSI formatting (colors, bold, reset)
- Unix timestamp formatting (recent, old, edge cases)
- Line formatting consistency

**Risk:** Low — display utilities, but inconsistent formatting degrades UX.

---

### 12. `render_payload.rs` — **GAP**
**Function:** `render_payload()` — dispatcher for Monitor/Docker/Fetch

**Missing Tests:**
- Variant dispatch correctness
- Dimension forwarding
- Palette forwarding

**Risk:** Low — thin dispatcher, but wrong variant = wrong render output.

---

### 13. `refit.rs` — **GAP**
**Functions:** `refit_header()`, `refit_line()`

**Missing Tests:**
- Header refit (wide/narrow terminals, fullwidth chars, ANSI stripping)
- Line refit (horizontal rules, plain text, empty lines)
- Unicode width handling (fullwidth vs halfwidth)

**Risk:** Header/line truncation or corruption at resize.

---

### 14. `fetch_render.rs` — **PARTIAL** (has `fetch_render_test.rs` with 14 tests)
**Missing inline unit tests for:** `center_header()`, `pick_lines()`, `find_logo()`, `load_db()`

**Note:** `fetch_render_test.rs` covers `render_fetch()` output. Internal helpers untested.

---

## Partial Coverage Modules (4)

| Module | Current Tests | Gaps |
|--------|---------------|------|
| `state.rs` | 2 (`save/load` roundtrip, `upgrade_started_at`) | `AppState` defaults, migration from older versions |
| `password_store.rs` | 1 (mock keychain) | Real keychain integration (CI-only), vault fallback logic |
| `sparkline.rs` | 1 (capacity + rendering) | Empty history, clamping at boundaries, Unicode bar chars |
| `ansi.rs` | 18 | Well covered — only gap: 24-bit RGB edge cases (values >255) |

---

## Well Covered Modules (2)

| Module | Tests | Notes |
|--------|-------|-------|
| `ansi.rs` | 18 | Comprehensive ANSI parsing coverage |
| `password_store.rs` (mock) | 1 | Mock store enables all E2E tests |

---

## Existing E2E Test Files

| File | Tests | Coverage |
|------|-------|----------|
| `upgrade_loop_e2e.rs` | 21 | Local upgrade loop (stream, UI cycle, security) |
| `upgrade_loop_remote_e2e.rs` | 10 | Remote SSH (all `#[ignore]`) |
| `vault_upgrade_e2e.rs` | 7 | Vault + upgrade integration |
| `e2e_password_upgrade_test.rs` | 4 | Password storage + upgrade |
| `app_test.rs` | 26 | App state machine |
| `ui_test.rs` | 8 | Monitor loop, render payload, resize |
| `server_settings_test.rs` | 5 | Config UI interactions |
| `fetch_render_test.rs` | 14 | Fetch rendering variants |
| `local_agent_test.rs` | ? | Local agent spawning |
| `ui_resize_test.rs` | ? | Resize handling |
| `chrome_resize_test.rs` | ? | Chrome-specific resize |
| `roadmap_regression_test.rs` | ? | Roadmap features |
| `configuration_test.rs` | ? | Config parsing |

---

## Recommended Priority

### P0 — Security/Correctness Critical
1. **`ssh.rs`** — Upgrade lock, command injection, agent deployment
2. **`config.rs`** — Config parsing, SSH config parsing
3. **`password_actions.rs`** — Credential write path, vault sync
4. **`passwords.rs`** — Config UI key handling

### P1 — Core Functionality
5. **`tasks.rs`** — Task spawning, generation tracking
6. **`panel.rs`** — Per-connection state
7. **`ssh_opts.rs`** — Architecture detection, shell quoting

### P2 — UI/Rendering
8. **`modals.rs`** — Security modals
9. **`config_ui.rs`** — Config panel rendering
10. **`ui.rs`** — Main layout
11. **`refit.rs`** — Line/header refitting
12. **`fetch_render.rs`** (internal helpers)

### P3 — Utilities
13. **`fmt.rs`** — Formatting
14. **`render_payload.rs`** — Dispatcher

---

## Test Strategy

### Unit Tests (inline `#[cfg(test)]`)
- Pure functions: `refit.rs`, `ssh_opts.rs`, `fmt.rs`, `ansi.rs` (done), `sparkline.rs` (done)
- Parsers: `config.rs` (parse, validate), `ssh.rs` (parse_need_agent)
- State machines: `passwords.rs` (handle_key), `password_store.rs` (mock)

### Integration Tests (`crates/multitop/tests/`)
- **New file:** `ssh_e2e_test.rs` — Local command execution, upgrade lock, agent deployment
- **New file:** `config_e2e_test.rs` — Config load/save, SSH config parsing
- **New file:** `password_ui_e2e_test.rs` — Full config UI flows (add/edit/delete servers, SSO, passwords)
- **Extend:** `upgrade_loop_e2e.rs` — Add panel state, task spawning tests
- **Extend:** `vault_upgrade_e2e.rs` — Add vault unlock/password fallback tests

### Property/Invariant Tests
- `ssh_opts.rs` — `sh_quote(sh_quote(x)) == sh_quote(x)` idempotence
- `refit.rs` — `refit_line(refit_line(x, w), w) == refit_line(x, w)`
- `config.rs` — `parse(load(x)) == x` roundtrip for valid configs

---

## CI Integration

Add to `ci.yml`:
```yaml
- name: Run unit tests
  run: cargo test --package multitop --lib --all-targets

- name: Run integration tests (local)
  run: cargo test --package multitop --test upgrade_loop_e2e --test vault_upgrade_e2e --test ssh_e2e_test --test config_e2e_test --test password_ui_e2e_test

- name: Run integration tests (remote)
  if: env.MULTITOP_TEST_SSH_HOST != ''
  run: cargo test --package multitop --test upgrade_loop_remote_e2e -- --test-threads=1
```

---

## Notes

- **Mock keychain** (`cfg!(test)`) enables all password/vault tests without OS keychain
- **Local SSH path** (`127.0.0.1`) tests remote code paths without network
- **Generation tracking** (`bump()`, `upgrade_gen`) is the key to stale-result prevention — test it
- **Upgrade lock** (local + remote) prevents concurrent upgrades — test contention
- **Vault priority** > keychain > session — test fallback chain