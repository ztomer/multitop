# Master Key + Encrypted Password Vault — Alternative Plan

## Concept

Instead of a single SSO password that works everywhere, use a **master key** in Keychain to decrypt a local encrypted vault containing per-server sudo passwords.

```
Keychain (master key)
    │
    ▼
Local encrypted file: ~/.config/multitop/passwords.enc
    │
    ├── server-a.example.com: "sudo_pass_a"
    ├── server-b.example.com: "sudo_pass_b"
    └── server-c.example.com: "sudo_pass_c"
```

## Comparison

| Aspect | SSO Password (current plan) | Master Key + Vault |
|--------|----------------------------|-------------------|
| **Assumption** | Same sudo password on all servers | Different passwords per server |
| **Keychain item** | 1 (the SSO password) | 1 (the master encryption key) |
| **Local storage** | None | Encrypted vault file |
| **Sync across machines** | Automatic (Keychain sync) | Manual (copy vault file) or iCloud Drive |
| **Compromise impact** | All servers at once | All servers at once (if vault + key stolen) |
| **User workflow** | Set once in settings | Set master key once, then add per-server passwords |
| **Fallback** | Per-server overrides | Per-server in vault |

## When This Makes Sense

1. **Different sudo passwords per server** — Common in heterogeneous environments
2. **No shared SSO** — Can't set same password everywhere (policy, different teams)
3. **Want local encrypted storage** — Don't want per-server items in Keychain (clutters Keychain)
4. **Offline capable** — Vault works without Keychain access (e.g., Linux headless with file-based key)

## When SSO Is Better

1. **Homogeneous environment** — Same password everywhere (most personal/homelab setups)
2. **Simplicity** — One password to remember/rotate
3. **Keychain sync** — Passwords sync across Macs automatically
4. **Less local state** — No encrypted file to manage/backup

## Implementation Plan (if chosen)

### 1. Master Key in Keychain

```rust
// password_store.rs
const MASTER_KEY_LABEL: &str = "multitop-master-key";

pub fn save_master_key(key: &[u8; 32]) -> Result<(), String> { ... }
pub fn load_master_key() -> Result<[u8; 32], String> { ... }
pub fn delete_master_key() -> Result<(), String> { ... }
```

### 2. Encrypted Vault Format

```
~/.config/multitop/passwords.enc
```

**Format (age / libsodium / aes-gcm):**
```
version: 1
salt: <32 bytes>
nonce: <12 bytes>
ciphertext: <encrypted JSON>
```

**Decrypted JSON:**
```json
{
  "server-a.example.com:22": "sudo_pass_a",
  "server-b.example.com:22": "sudo_pass_b"
}
```

### 3. API

```rust
pub fn vault_save_password(server: &Server, password: &str) -> Result<(), String>
pub fn vault_load_password(server: &Server) -> Result<Option<String>, String>
pub fn vault_delete_password(server: &Server) -> Result<(), String>
pub fn vault_list_servers() -> Result<Vec<String>, String>
```

### 4. Settings UI Integration

```
Configuration (press 'e')
    │
    ├── [Tab] Sudo Passwords
    │       │
    │       ├── [M] Set/Change Master Key (Keychain)
    │       │
    │       ├── [A] Add Server Password
    │       │       Host: server-a.example.com
    │       │       Password: ********
    │       │
    │       ├── [D] Delete Server Password
    │       │
    │       └── List:
    │               server-a.example.com  ● (saved)
    │               server-b.example.com  ○ (not saved)
    │
    └── [Tab] Servers
```

### 5. Upgrade Flow

```rust
// In confirm_upgrade()
let master_key = load_master_key()?;  // From Keychain
let vault = decrypt_vault(master_key)?;  // Load ~/.config/multitop/passwords.enc

// For each server with upgrade_cmd:
let pass = vault.get(&server.key())  // Exact match host:port
    .or_else(|| load_sso())  // Fallback to SSO if set
    .or_else(|| load_per_server_override());  // Legacy fallback
```

## Security Analysis

### Threat Model

| Attacker | SSO Plan | Vault Plan |
|----------|----------|------------|
| Local user (no Keychain access) | Can't get SSO | Can't decrypt vault |
| Local user + Keychain access | Gets SSO → all servers | Gets master key → decrypts vault → all servers |
| Stolen laptop (locked) | Keychain locked | Keychain locked + vault encrypted |
| Stolen laptop (unlocked, user logged in) | Keychain accessible | Keychain accessible + vault decryptable |
| Backup theft (Time Machine) | Keychain in backup | Vault file in backup (encrypted), Keychain in backup |

### Key Derivation

Use **Argon2id** (or PBKDF2) with per-vault salt:
```rust
let master_key = argon2id(password_from_keychain, salt, iterations);
```

### Rotation

- **Master key rotation**: Re-encrypt vault with new key
- **Per-server rotation**: Update single entry in vault
- **Compromise recovery**: Delete vault file, recreate

## Recommendation

**Default to SSO plan** for 90% of users (homogeneous environments, simpler).

**Add vault as "Advanced" option** behind a feature flag or settings toggle:
- "Use per-server password vault (encrypted)" checkbox in settings
- When enabled, hide SSO field, show vault management
- Migration path: SSO passwords can be imported into vault

## Decision Matrix

```
User has same sudo password everywhere? → SSO (simpler)
       │
       └─ No → Different passwords per server?
                    │
                    ├─ Yes, few servers (2-5) → Per-server Keychain items (current override)
                    │
                    └─ Yes, many servers (5+) → Vault (cleaner)
```

## Migration Path

1. **v0.21**: SSO plan (default)
2. **v0.22**: Vault as opt-in advanced feature
3. **v0.23**: Auto-detect → suggest vault if >3 different passwords detected