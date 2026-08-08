# File-Split Plan — Topological Dependency Analysis

## Dependency Graphs

### app.rs (1547 lines)

```
apply ──→ accepts, current_theme, fail_vault_creation, now_secs, persist_state,
           push_capped, report_rotation, seed_vault_from_panels, upgrades_in_flight
confirm_upgrade ──→ filtered_indices, mark_upgrades_started, run_upgrade, upgrade_runnable
run_upgrade ──→ bump, current_theme, filtered_indices, load_known_passwords, new, reset_scroll
upgrade_pane_header ──→ current_theme, host_update, new, now_secs
mark_upgrades_started ──→ now_secs, persist_state
mark_upgrade_interrupted ──→ now_secs, persist_state
enter_upgrade_view ──→ load_known_passwords
note_nothing_to_upgrade ──→ current_theme
rerender_all ──→ current_theme
request_quit ──→ upgrades_in_flight
toggle_fetch ──→ bump, current_theme, in_fetch, new, reset_scroll
toggle_docker ──→ bump, current_theme, in_docker, new, reset_scroll
switch_stats ──→ bump, new
upgrade_skip_hosts ──→ filtered_indices
upgrade_runnable ──→ filtered_indices

# Isolated clusters (only call bump_vault_epoch):
begin_vault_unlock, begin_vault_creation, cancel_vault_biometric,
cancel_vault_creation, cancel_vault_verify, set_vault_unlocking
```

**Clusters by connectivity:**
| Cluster | Functions | Internal deps | External deps |
|---------|-----------|---------------|---------------|
| **Vault** | 6 methods | `bump_vault_epoch` | None |
| **Views** | 10+ methods | `bump`, `new`, `reset_scroll`, `current_theme` | None |
| **Upgrade** | 7 methods | `mark_upgrades_started`, `run_upgrade` | Vault (`load_known_passwords`), Views (`filtered_indices`, `bump`) |
| **Apply** | 2 methods | `accepts` | All clusters |

**Optimal split:** `vault.rs`, `views.rs`, `upgrade.rs`, `apply.rs`, `types.rs`
- Cross-cluster deps: `upgrade` → `vault` (1 call), `upgrade` → `views` (2 calls), `apply` → all (9 calls)
- Resolution: Each cluster imports siblings via `crate::app::{vault, views, upgrade}`

### tasks.rs (837 lines)

```
# Production functions — NO internal calls to each other:
deliver_sudo_password (independent)
spawn_fetch (independent)
spawn_docker (independent)
spawn_upgrade (independent)

# Test helpers:
run ──→ deliver_sudo_password
shown ──→ (helper, no production calls)
```

**Key insight:** The 4 production functions are completely independent! They share no internal calls.

**Optimal split:** `spawn.rs` (deliver_sudo_password + spawn_fetch + spawn_docker), `upgrade.rs` (spawn_upgrade), `painted.rs` (painted_states + Marker + marker + is_sudo_help), tests inline or separate

### config.rs (547 lines)

```
load ──→ legacy_config_path, missing_config_message, parse
parse ──→ validate_host, validate_user
default_config_path ──→ config_home
legacy_config_path ──→ config_home
```

**Clusters:**
| Cluster | Functions | Internal deps |
|---------|-----------|---------------|
| **Types** | Server, Config, ConfigError, constants | None |
| **Load** | load, parse | validate_host, validate_user |
| **Save** | save_servers, save_theme, save_banner_style, strip_plaintext_passwords | None |
| **SSH** | ssh_config_path, merge_ssh_hosts, parse_ssh_config | None |
| **Paths** | default_config_path, legacy_config_path, config_home | None |

**Optimal split:** `types.rs`, `load.rs`, `save.rs`, `ssh.rs`
- Cross-cluster deps: `load` → `paths` (legacy_config_path)
- Resolution: `load.rs` imports `config_home` from `crate::config` (re-exported)

**Blocker:** Nightly compiler bug. Defer.

### ssh/prod.rs (611 lines)

```
spawn_agent ──→ bootstrap_script, is_local, spawn_local_agent, ssh_command
bootstrap_script ──→ agent_path
upload_agent ──→ cleanup_old_agents_command, ssh_command, upload_command, upload_failure
spawn_local_agent ──→ detached
ssh_command ──→ detached
ssh_command_tty ──→ detached
probe_remote_arch ──→ ssh_command
```

**Clusters:**
| Cluster | Functions | Internal deps |
|---------|-----------|---------------|
| **Command** | agent_path, bootstrap_script, upload_command, cleanup_old_agents_command, parse_need_agent, password_preamble, ssh_command_tty, is_local, detached, ssh_command, sh_quote | `detached` (3 callers), `agent_path` (1), `ssh_command` (1) |
| **Spawn** | spawn_local_agent, spawn_agent, wrap_with_upgrade_lock, wrap_with_local_upgrade_lock, Spawned, spawn_command, upload_agent, probe_remote_arch, upload_failure | `bootstrap_script`, `is_local`, `ssh_command`, `ssh_command_tty` (from Command) |

**Optimal split:** `command.rs`, `spawn.rs`
- Cross-cluster deps: `spawn` → `command` (4 functions)
- Resolution: `spawn.rs` imports from `crate::command::*`

## Topological Ordering for Execution

Based on dependency isolation (fewest cross-references first):

1. **tasks.rs** — 4 independent production functions, zero internal deps
2. **ssh/prod.rs** — 2 clusters, 4 cross-refs (all from spawn→command)
3. **app.rs** — 4 clusters, ~10 cross-refs
4. **config.rs** — blocked by compiler bug

## Execution Plan

### Phase 1: tasks.rs (easiest, highest confidence)

```
tasks/
├── mod.rs          # declarations + re-exports
├── spawn.rs        # deliver_sudo_password + spawn_fetch + spawn_docker + constants
├── upgrade.rs      # spawn_upgrade
├── painted.rs      # painted_states + Marker + marker + is_sudo_help
└── tests.rs        # all tests (with pub(crate) helpers)
```

- Production functions have ZERO internal deps → clean split
- Tests need `deliver_sudo_password` and `BufReader` → make `pub(crate)` or keep inline
- Blocker: `while let` pattern in tests triggers compiler bug when in separate file

### Phase 2: ssh/prod.rs

```
ssh/
├── mod.rs          # declarations + re-exports (already exists)
├── command.rs      # command-building functions
├── spawn.rs        # spawn functions
└── ssh_tests.rs    # tests (already exists)
```

- `spawn.rs` imports from `command.rs` via `use crate::ssh::command::{is_local, ssh_command_tty, ...}`
- All functions made `pub` for cross-module access

### Phase 3: app.rs

```
app/
├── mod.rs          # App struct + LOG_AMORTIZE + push_capped + re-exports
├── types.rs        # AppMode, Confirm, VaultState
├── vault.rs        # 6 vault methods (only call bump_vault_epoch)
├── views.rs        # constructors, view switches, scroll, theme
├── upgrade.rs      # upgrade methods (calls vault + views)
└── apply.rs        # apply + accepts (calls everything)
```

- Each submodule has its own `impl App { ... }` block
- Cross-module calls: `self.method()` works across impl blocks
- `upgrade.rs` imports: `crate::app::vault::load_known_passwords` (via re-export)
- `apply.rs` imports: `crate::app::{vault, views, upgrade}` as needed

### Phase 4: config.rs (deferred)

- Wait for nightly compiler bug fix or pin to working version
- Split: `types.rs`, `load.rs`, `save.rs`, `ssh.rs`
- All items re-exported from mod.rs so downstream code is unchanged

## Risk Matrix

| File | Risk | Blocker | Mitigation |
|------|------|---------|------------|
| tasks.rs | Low | `while let` compiler bug | Replace with `loop { match }` |
| ssh/prod.rs | Low-Med | Cross-module imports | Make fns `pub`, use explicit paths |
| app.rs | Med | Cross-cluster impl calls | Re-export from mod.rs, use `self.method()` |
| config.rs | High | Nightly compiler bug | Defer until bug fixed |

## Success Criteria

- All files under 500 LOC
- 469+ tests pass
- No new `#[allow]` (except known compiler bugs)
- Clean module boundaries
