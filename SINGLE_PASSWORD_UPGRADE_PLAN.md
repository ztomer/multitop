# Single Password Upgrade Across All Machines — Plan

## Current State

- **SSO Password**: Stored in Apple Keychain (`password_store.rs:save_sso`/`load_sso`)
- **Per-server override**: Also stored in Keychain per server (`save`/`load`)
- **Upgrade flow**: Each panel runs `spawn_upgrade` → `spawn_command` → separate SSH session → own `sudo -v`
- **Result**: 1 password prompt per panel × panels = 3 prompts for 3 servers

## Goal

Single password prompt → upgrade all servers using the SSO password from Keychain.

## Architecture

```
User presses 'u'
    │
    ▼
App checks: Do we have SSO password in Keychain?
    │
    ├── YES → Use SSO password for ALL servers with upgrade_cmd
    │
    └── NO  → Fall back to per-server passwords (current behavior)
```

## Implementation Plan

### 1. Load SSO Password at Upgrade Start (`app.rs:confirm_upgrade`)

```rust
pub fn confirm_upgrade(&mut self) -> Vec<Command> {
    self.show_upgrade_modal = false;
    
    // Load SSO password from Keychain ONCE
    let sso_password = if let Some(ref manager) = self.password_manager {
        // Already in memory from settings screen
        None // Will be loaded per-panel below
    } else {
        crate::password_store::load_sso().ok() // Load from Keychain
    };
    
    // Store for upgrade tasks
    self.sso_upgrade_password = sso_password;
    
    self.run_upgrade()
}
```

### 2. Per-Host Sudo Validation (`tasks.rs:spawn_upgrade`)

**Key insight**: Multiple panels on the SAME host share SSH connection via `ControlMaster`. A `sudo -v` in one channel validates for all channels on that connection.

```rust
pub fn spawn_upgrade(
    idx: usize,
    gen: u64,
    server: Server,
    sso_password: Option<String>,  // NEW: shared SSO password
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Determine password: SSO > per-server override > none
        let pass = sso_password
            .or_else(|| crate::password_store::load(&server).ok().flatten());
        
        // Track which hosts have validated sudo
        // Use a shared HostSudoState in Tasks struct
    })
}
```

### 3. Shared Sudo State in Tasks (`run.rs:Tasks`)

```rust
struct HostSudoState {
    validated: bool,
    password: Option<String>,  // The password that worked
}

pub struct Tasks {
    // ... existing fields ...
    sudo_state: HashMap<String, HostSudoState>,  // Key = host:port
}
```

### 4. Modified spawn_command (`ssh.rs`)

**Option A: Reuse validated sudo (preferred)**

```rust
// In spawn_command, if host already validated:
if let Some(state) = tasks.sudo_state.get(&host_key) {
    if state.validated {
        // Run command WITHOUT sudo -v - reuse existing timestamp
        let remote_cmd = wrap_with_upgrade_lock(&format!(
            "zsh -l -i -c 'source ~/.zshrc; source ~/.zprofile; eval {}'",
            quoted_escaped
        ));
    }
} else {
    // First panel on this host: run sudo -v
    let remote_cmd = wrap_with_upgrade_lock(&format!(
        "echo {} | sudo -S -p '' -v && zsh -l -i -c 'source ~/.zshrc; source ~/.zprofile; eval {}'",
        sh_quote(&pass),
        quoted_escaped
    ));
    // Mark host as validated on success
}
```

**Option B: Single sudo for entire upgrade pipeline**

```bash
# Run ENTIRE upgrade under single sudo -S
echo "password" | sudo -S -p "" sh -c '
    source ~/.zshrc
    source ~/.zprofile
    apt update && apt upgrade -y
'
```

Requires user's `upgrade_cmd` to NOT contain `sudo` internally.

### 5. Password Priority Order

```
1. SSO password (from Keychain) — applies to ALL servers
2. Per-server override (from Keychain) — specific server
3. No password — attempt passwordless sudo
```

### 6. UI Flow

```
Press 'u'
    │
    ├─► Modal: "Upgrade 3 servers?"
    │       [Yes] [No]
    │
    ├─► Check Keychain for SSO password
    │       │
    │       ├─► Found: "Using saved SSO password for all servers"
    │       │
    │       └─► Not found: "Enter sudo password for each server..."
    │
    └─► Run upgrades
            │
            ├─► Server A (panel 0): sudo -v ✓ → upgrade
            ├─► Server B (panel 1): REUSE Server A's sudo ✓ → upgrade
            └─► Server C (panel 2): new host → sudo -v ✓ → upgrade
```

## Key Files to Modify

| File | Change |
|------|--------|
| `app.rs` | Add `sso_upgrade_password: Option<String>` field; load in `confirm_upgrade` |
| `tasks.rs` | Add `sudo_state: HashMap<String, HostSudoState>` to `Tasks`; pass to `spawn_upgrade` |
| `tasks.rs:spawn_upgrade` | Accept `sso_password`, determine effective password, check `sudo_state` |
| `ssh.rs:spawn_command` | Accept `reuse_sudo: bool` flag; skip `sudo -v` if true |
| `run.rs` | Initialize `sudo_state` in `Tasks::new`; pass to `spawn_upgrade` |

## Security Considerations

1. **SSO password in memory**: Only during upgrade session; cleared after
2. **Keychain access**: Uses existing `password_store::load_sso()` — no new permissions
3. **Per-host isolation**: `sudo -v` timestamp is per-host, not global
4. **Fallback**: If SSO fails, falls back to per-server passwords automatically

## Edge Cases

| Scenario | Handling |
|----------|----------|
| 3 panels, 2 on same host | Host A validated once; panels 0&1 reuse; panel 2 validates separately |
| SSO password wrong | First `sudo -v` fails → error shown → user can enter correct password in settings |
| Mixed: some servers need different password | Per-server override takes precedence over SSO for that server |
| No Keychain access (Linux headless) | Falls back to per-server passwords in memory |

## Testing

1. **Single host, 3 panels**: 1 password prompt total
2. **3 different hosts, same SSO password**: 3 password prompts (one per host)
3. **SSO password in Keychain**: Auto-loaded, no user input
4. **Wrong SSO password**: Graceful error, fallback to manual entry
5. **Mixed override + SSO**: Override wins for that server

## Future Enhancement: Background Sudo Keep-Alive

For long upgrades (>15 min sudo timeout), spawn a background task that runs `sudo -v` every 10 minutes to keep timestamp alive across all channels on the host.