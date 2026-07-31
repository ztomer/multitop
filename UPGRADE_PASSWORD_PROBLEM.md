# Upgrade Password Prompt Analysis

## Problem Statement

When starting an upgrade (`u` key), the user is prompted for:
- **Password 3 times** (once per panel/server being upgraded)
- **Fingerprint once** (SSH host key verification)

## Root Cause Analysis

### 1. Three Password Prompts

**Current flow** (`ssh.rs:spawn_command` lines 319-323):

```bash
# SSH session shell (non-interactive, non-login)
echo "password" | sudo -S -p '' -v 2>/dev/null && \
# NEW login shell - separate process!
zsh -l -i -c 'source ~/.zshrc; source ~/.zprofile; eval "apt update && apt upgrade -y"'
```

**Why it fails:**

1. `sudo -v` validates the password and creates a sudo timestamp in the **SSH session shell**
2. `zsh -l -i -c '...'` spawns a **completely new login shell** as a child process
3. The upgrade command typically contains `sudo` (e.g., `sudo apt update && sudo apt upgrade -y`)
4. The new login shell **does not inherit the sudo timestamp** from its parent because:
   - It's a login shell (`-l`) with a fresh environment
   - It may allocate a different tty/pts
   - The parent's `sudo -v` timestamp is tied to the parent's tty
5. Each `sudo` inside the upgrade command prompts for a password again

**Per-panel multiplier:** If monitoring 3 servers, each panel runs its own `spawn_command` → 3 separate SSH sessions → 3 separate `sudo -v` validations → 3 × N password prompts where N = number of `sudo` calls in the upgrade command.

### 2. One Fingerprint Prompt

**Cause:** SSH_OPTS (`ssh_opts.rs`) does not disable host key checking:

```rust
pub const SSH_OPTS: &[&str] = &[
    "-o", "ControlMaster=auto",
    "-o", "ControlPath=/tmp/multitop-ssh-%u-%C",
    "-o", "ControlPersist=30s",
    "-o", "ConnectTimeout=10",
    "-o", "ServerAliveInterval=15",
    "-o", "ServerAliveCountMax=3",
    "-o", "SendEnv=-*",
    "-T",  // Disable pseudo-terminal
];
```

Missing: `-o StrictHostKeyChecking=accept-new` or `-o UserKnownHostsFile=/dev/null`

On first connection to a new server, SSH prompts for fingerprint confirmation. This happens once per unique host.

### 3. Connection Multiplexing Not Helping

`ControlMaster=auto` + `ControlPersist=30s` should reuse SSH connections, but:
- `spawn_command` creates a **new `ssh` process** for each panel's upgrade command
- Each `ssh` process connects to the control socket, but runs its own command in a **new channel/shell**
- The `sudo -v` timestamp doesn't persist across channels in a way that helps the child login shell

## Proposed Solution

### Primary Fix: Run `sudo -v` INSIDE the login shell

Move the sudo validation into the same shell that runs the upgrade command:

```bash
# Single login shell - sudo timestamp persists for ALL sudo calls within
zsh -l -i -c 'echo "password" | sudo -S -p "" -v && source ~/.zshrc; source ~/.zprofile; eval "apt update && apt upgrade -y"'
```

**Key changes:**
1. `sudo -v` runs **inside** `zsh -l -i -c`, not in the parent SSH shell
2. The sudo timestamp is created in the login shell's context
3. Subsequent `sudo` calls in the upgrade command inherit this timestamp
4. Only **one** password prompt per panel (for the initial `sudo -v`)

### Secondary Fix: Disable fingerprint prompt

Add to `SSH_OPTS`:
```rust
"-o", "StrictHostKeyChecking=accept-new",
```

This automatically accepts new host keys (TOFU model). For stricter security, use `UserKnownHostsFile=/dev/null` with a known_hosts management strategy.

### Tertiary Optimization: Single sudo validation for all panels on same host

If multiple panels monitor the same host, they share the SSH connection via `ControlMaster`. The sudo timestamp from the first panel's upgrade could be reused by subsequent panels if we:
1. Run `sudo -v` once per host (not per panel)
2. Share the timestamp across all upgrade commands on that host

This requires coordination in `tasks.rs:spawn_upgrade` to track which hosts have validated sudo.

## Implementation Plan

### Option A: Minimal fix (move sudo -v inside login shell)
**File:** `ssh.rs:spawn_command`
- Change the remote command construction to pipe password to `sudo -v` inside the `zsh -l -i -c` string
- Remove the outer `sudo -v &&`

### Option B: Full fix with host-level sudo caching
**Files:** `ssh.rs`, `tasks.rs`
- Add host-level sudo validation tracking in `tasks.rs`
- Only run `sudo -v` once per unique host per upgrade session
- Subsequent panels on same host skip `sudo -v`

### Option C: Use `sudo -S` for entire upgrade pipeline
**File:** `ssh.rs:spawn_command`
- Instead of `sudo -v` + separate sudos, pipe password to the entire command:
  ```bash
  echo "password" | sudo -S -p "" sh -c 'source ~/.zshrc; apt update && apt upgrade -y'
  ```
- But this requires the upgrade command to NOT contain `sudo` internally (user must omit sudo from their `upgrade_cmd`)

## Recommended Approach: Option A + Fingerprint Fix

**Changes:**
1. `ssh.rs:spawn_command` - restructure remote command to run `sudo -v` inside the login shell
2. `ssh_opts.rs:SSH_OPTS` - add `StrictHostKeyChecking=accept-new`
3. Test with multi-panel setup on same and different hosts

**Expected result:**
- 1 password prompt per panel (for `sudo -v` inside login shell)
- 0 fingerprint prompts (auto-accept new hosts)
- If 3 panels on 3 hosts → 3 passwords total (down from 9+)
- If 3 panels on 1 host → 1 password total (with Option B)