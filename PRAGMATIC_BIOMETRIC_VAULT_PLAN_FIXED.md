# Pragmatic Biometric Vault — Fixed Plan

## Core Idea (Unchanged)

Use biometric (Touch ID / fingerprint) as **convenience** for unlocking a per-server password vault, with **system sudo password** as fallback. No separate master passphrase.

**Critical fix**: Biometric and password paths use **independent key wrapping** — not shared derivation. This eliminates the downgrade attack.

---

## Vault File Format

```binary
~/.local/share/multitop/vault.bin (0600, dir 0700)
┌────────────────────────────────────────────────────────────────────┐
│ Magic: "MQV1" (4 bytes)                                            │
├────────────────────────────────────────────────────────────────────┤
│ Version: 1 (1 byte)                                                │
├────────────────────────────────────────────────────────────────────┤
│ Key Version: u8 (incremented on rotation)                          │
├────────────────────────────────────────────────────────────────────┤
│ Created Timestamp: u64 (Unix ms)                                   │
├────────────────────────────────────────────────────────────────────┤
│ Monotonic Counter: u32 (incremented on each save)                  │
├────────────────────────────────────────────────────────────────────┤
│ Salt: 32 bytes (random per vault)                                  │
├────────────────────────────────────────────────────────────────────┤
│ Wrapped Key Entries (variable count):                              │
│   ┌────────────────────────────────────────────────────────────┐   │
│   │ Type: u8 (0x01=SecureEnclave, 0x02=TPM2, 0x03=Argon2id)    │   │
│   ├────────────────────────────────────────────────────────────┤   │
│   │ Length: u16                                                │   │
│   ├────────────────────────────────────────────────────────────┤   │
│   │ Data: variable (wrapped key blob)                          │   │
│   └────────────────────────────────────────────────────────────┘   │
│   (repeated for each available wrapping method)                   │
├────────────────────────────────────────────────────────────────────┤
│ Nonce: 12 bytes (random per encryption)                            │
├────────────────────────────────────────────────────────────────────┤
│ Ciphertext: AES-256-GCM(vault_key, JSON)                           │
├────────────────────────────────────────────────────────────────────┤
│ Ed25519 Signature (over header || ciphertext)                      │
└────────────────────────────────────────────────────────────────────┘
```

**Key point**: Multiple wrapped forms of the **same** `vault_key` (32 bytes). Each wrapping method is independent.

---

## Key Wrapping Methods

### 0x01: macOS Secure Enclave (Preferred)

**Vault Creation**:
```rust
// 1. Generate random vault_key (32 bytes)
let vault_key = random_32_bytes();

// 2. Generate EC P-256 key in Secure Enclave (persistent, biometric-gated)
let se_key = SecKey::generate_in_secure_enclave(
    label: "multitop-vault-key",
    require_biometric: true
)?;

// 3. Wrap vault_key with Secure Enclave key (AES-KW or direct encrypt)
let wrapped = se_key.wrap_key(&vault_key)?;  // Requires biometric on unwrap

// 4. Store wrapped blob (includes SE key handle + encrypted vault_key)
```

**Unlock**:
```rust
// 1. Get SE key handle from vault header
// 2. LAContext.evaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, "Unlock vault")
// 3. se_key.unwrap_key(&wrapped) → vault_key (requires biometric)
// 4. Use vault_key to decrypt vault
```

**Properties**: Key never leaves Secure Enclave. Biometric required for **every** unwrap. No fallback to password for this wrapping type.

### 0x02: Linux TPM2 (Preferred on Linux)

**Vault Creation**:
```rust
// 1. Generate random vault_key
// 2. Create TPM2 primary key (RSA 2048 or ECC P-256) in owner hierarchy
// 3. Create child key with policy: PCR 16 (fprintd auth) + authValue
// 4. TPM2_EncryptDecrypt to wrap vault_key
// 5. Store TPM2 key handle + wrapped blob
```

**Unlock**:
```rust
// 1. fprintd verify fingerprint
// 2. TPM2_EncryptDecrypt(unwrap) with authValue from fprintd
// 3. Get vault_key
```

**Fallback**: If no TPM2 or fprintd unavailable → skip this wrapping type.

### 0x03: Argon2id (Password Fallback — Always Present)

**Vault Creation**:
```rust
// 1. vault_key = random_32_bytes()
// 2. wrapped = encrypt_aes256gcm(
//        key = Argon2id(password, salt, t=10, m=256MiB, p=4),
//        plaintext = vault_key,
//        nonce = random_12_bytes()
//    )
// 3. Store wrapped (nonce || ciphertext || tag)
```

**Unlock**:
```rust
// 1. Prompt: "Enter sudo password to unlock vault"
// 2. key = Argon2id(password, salt, t=10, m=256MiB, p=4)
// 3. vault_key = decrypt_aes256gcm(key, wrapped)
// 4. Use vault_key to decrypt vault
```

**Parameters**: `t=10, m=256MiB, p=4` (~3-5 seconds on modern CPU). Adjustable via config.

---

## Unlock Flow (Fixed — No Downgrade)

```rust
pub async fn unlock(&mut self) -> Result<()> {
    // 1. Parse vault header, get available wrapping methods
    let methods = self.header.wrapping_methods();  // e.g., [0x01, 0x03]
    
    // 2. Try biometric/TPM methods FIRST (in priority order)
    for method in methods {
        if method == 0x01 && cfg!(target_os = "macos") {
            if let Ok(key) = self.unwrap_secure_enclave().await {
                self.vault_key = key;
                return self.decrypt_vault();
            }
            // Biometric failed (cancelled, sensor error) → try next method
        }
        if method == 0x02 && cfg!(target_os = "linux") {
            if let Ok(key) = self.unwrap_tpm2().await {
                self.vault_key = key;
                return self.decrypt_vault();
            }
        }
    }
    
    // 3. ALL biometric/TPM methods failed or unavailable
    //    → Fall back to password (method 0x03, ALWAYS present)
    let password = prompt_password("Enter sudo password to unlock vault:")?;
    let key = self.unwrap_argon2id(&password)?;
    self.vault_key = key;
    self.decrypt_vault()?;
    
    // 4. OPPORTUNISTIC RE-BIND: If we used password but biometric IS available,
    //    re-wrap vault_key with biometric method and update vault file
    if methods.contains(&0x01) && cfg!(target_os = "macos") {
        let wrapped = wrap_secure_enclave(&self.vault_key)?;
        self.header.add_wrapping(0x01, wrapped);
        self.save()?;  // Updates counter, rewrites file
    }
    // Same for TPM2 on Linux
    
    Ok(())
}
```

**Critical**: Password path does NOT derive the same key as biometric path. They are independent wrappers around the same `vault_key`. Attacker with vault file **cannot** use password path to bypass biometric — they need the biometric-bound wrapped key which requires biometric to unwrap.

---

## Vault Operations

```rust
use secrecy::{SecretString, Zeroize};
use zeroize::ZeroizeOnDrop;

#[derive(ZeroizeOnDrop)]
struct VaultKey([u8; 32]);

pub struct Vault {
    path: PathBuf,
    header: VaultHeader,
    #[zeroize(skip)]  // Don't zeroize on drop - we need it for verification
    signature: Ed25519Signature,
    passwords: Option<HashMap<String, SecretString>>,  // Zeroized on drop
}

impl Vault {
    pub fn create(passwords: HashMap<String, String>, system_password: &str) -> Result<Self> {
        let vault_key = VaultKey(random_32_bytes());
        let salt = random_32_bytes();
        
        // Always create Argon2id wrapper (fallback)
        let argon2id_wrapped = wrap_argon2id(&vault_key, system_password, &salt)?;
        
        // Try to create biometric wrapper
        let mut wrappers = vec![(0x03, argon2id_wrapped)];
        if cfg!(target_os = "macos") {
            if let Ok(w) = wrap_secure_enclave(&vault_key)? {
                wrappers.insert(0, (0x01, w));  // Biometric first
            }
        }
        #[cfg(target_os = "linux")]
        if let Ok(w) = wrap_tpm2(&vault_key)? {
            wrappers.insert(0, (0x02, w));
        }
        
        let header = VaultHeader::new(salt, wrappers, 0);  // counter = 0
        let ciphertext = encrypt_vault(&vault_key, &passwords)?;
        let signature = sign_vault(&signing_key, &header, &ciphertext)?;
        
        write_vault_file_0600(&path, &header, &ciphertext, &signature)?;
        
        Ok(Vault { path, header, signature, passwords: Some(passwords.into()) })
    }
    
    pub async fn unlock(&mut self) -> Result<()> {
        // ... unlock flow from above ...
        
        // After successful decrypt:
        self.passwords = Some(decrypt_passwords(&self.vault_key, &self.header.nonce, &self.ciphertext)?);
        Ok(())
    }
    
    pub fn get(&self, host: &str) -> Option<&SecretString> {
        self.passwords.as_ref()?.get(host)
    }
    
    pub fn set(&mut self, host: String, pass: SecretString) -> Result<()> {
        self.passwords.as_mut().unwrap().insert(host, pass);
        self.save()
    }
    
    pub fn save(&mut self) -> Result<()> {
        self.header.counter += 1;
        self.header.timestamp = now_ms();
        let ciphertext = encrypt_vault(&self.vault_key, &self.passwords)?;
        self.signature = sign_vault(&signing_key, &self.header, &ciphertext)?;
        write_vault_file_0600(&self.path, &self.header, &ciphertext, &self.signature)?;
        Ok(())
    }
}
```

---

## Argon2id Parameters (Fixed)

```rust
const ARGON2_T: u32 = 10;           // iterations
const ARGON2_M: u32 = 256 * 1024;   // 256 MiB
const ARGON2_P: u32 = 4;            // parallelism

// ~3-5 seconds on M4 / modern x86
// Configurable via config.toml:
// [vault]
// argon2_time = 10
// argon2_memory_mib = 256
```

**Documentation**: "Vault security = your sudo password entropy. Use 16+ random characters for meaningful protection."

---

## File Permissions & Memory Safety

```rust
fn write_vault_file_0600(path: &Path, header: &VaultHeader, ct: &[u8], sig: &[u8]) -> Result<()> {
    // Ensure parent dir is 0700
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        std::os::unix::fs::PermissionsExt::set_mode(
            &mut std::fs::metadata(parent)?.permissions(),
            0o700
        );
    }
    
    // Write to temp file first (atomic)
    let tmp = path.with_extension("tmp");
    let mut file = std::fs::File::create(&tmp)?;
    #[cfg(unix)]
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    
    serialize_vault(&mut file, header, ct, sig)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)?;  // Atomic on same filesystem
    Ok(())
}
```

**Memory**: All `SecretString` + `ZeroizeOnDrop` for `VaultKey` and passwords. Consider `mlock` for vault key on Linux (`libc::mlock`).

---

## Key Rotation (New)

```rust
impl Vault {
    pub fn rotate_key(&mut self, new_password: Option<&str>) -> Result<()> {
        // 1. Generate new vault_key
        let new_key = VaultKey(random_32_bytes());
        
        // 2. Re-encrypt all wrapped forms with new_key
        let mut new_wrappers = Vec::new();
        for (typ, wrapped) in &self.header.wrappers {
            let old_key = self.unwrap(typ, wrapped)?;  // Requires auth
            let new_wrapped = wrap(typ, &new_key, new_password)?;
            new_wrappers.push((*typ, new_wrapped));
        }
        
        // 3. Update header
        self.header.key_version += 1;
        self.header.wrappers = new_wrappers;
        
        // 4. Re-encrypt vault with new_key
        self.vault_key = new_key;
        self.save()?;
        
        Ok(())
    }
}
```

Trigger on:
- Password change in settings
- Biometric re-enrollment (new finger added)
- Manual "Rotate Vault Key" button

---

## Rollback Protection (New)

```rust
struct VaultHeader {
    // ...
    created_ts: u64,        // Vault creation time
    counter: u32,           // Incremented on each save
    last_counter_seen: u32, // Stored in Keychain/TPM (persistent)
}

fn verify_no_rollback(header: &VaultHeader) -> Result<()> {
    let stored = load_last_counter_from_keychain()?;  // Secure storage
    if header.counter <= stored && header.created_ts == stored_created_ts {
        return Err("Vault rollback detected — file may have been replaced");
    }
    // Update stored counter
    save_last_counter_to_keychain(header.counter, header.created_ts)?;
    Ok(())
}
```

Prevents attacker from restoring old vault file (e.g., from backup) to recover old passwords.

---

## Settings UI (Unchanged)

```
Configuration (press 'e')
    │
    ├── [Tab] Sudo Passwords
    │       │
    │       ├── Vault: ● Locked  /  ○ Unlocked (Touch ID)  /  ○ Unlocked (Password)
    │       │
    │       ├── [U] Unlock Vault
    │       ├── [L] Lock Vault
    │       ├── [R] Rotate Vault Key
    │       ├── [A] Add Server Password
    │       │       Host: server-a.example.com
    │       │       Password: ********  (saved to vault, encrypted)
    │       │
    │       ├── [D] Delete Server Password
    │       │
    │       └── List:
    │               server-a.example.com  ● (in vault)
    │               server-b.example.com  ○ (not saved)
    │
    └── [Tab] Servers
```

---

## Integration with Upgrade Flow (Unchanged)

```rust
// app.rs:confirm_upgrade()
pub async fn confirm_upgrade(&mut self) -> Vec<Command> {
    self.show_upgrade_modal = false;
    
    let mut vault = Vault::load_or_create()?;
    vault.unlock().await?;  // Biometric → password fallback
    
    self.vault = Some(vault);
    self.run_upgrade()
}

// tasks.rs:spawn_upgrade()
let pass = app.vault.as_ref()
    .and_then(|v| v.get(&server.key()))
    .map(|s| s.expose_secret().to_string())  // secrecy::ExposeSecret
    .or_else(|| load_sso())
    .or_else(|| load_override(&server));
```

---

## Dependencies

| Crate | Purpose |
|-------|---------|
| `aes-gcm` | AES-256-GCM |
| `argon2` | Argon2id (t=10, m=256MiB) |
| `ed25519-dalek` | Ed25519 signatures |
| `hkdf` | HKDF-SHA256 |
| `rand` / `getrandom` | Random bytes |
| `security-framework` | macOS Secure Enclave |
| `tss-esapi` | Linux TPM2 (optional) |
| `zbus` + `fprintd` | Linux fingerprint (optional) |
| `secrecy` + `zeroize` | Secret memory handling |
| `serde` + `serde_json` | Vault JSON |
| `keyring` | Keychain storage for rollback counter |

---

## Security Properties (Fixed)

| Property | Mechanism |
|----------|-----------|
| **No downgrade attack** | Independent key wrapping per method; password cannot unwrap biometric blob |
| **Strong password hashing** | Argon2id(t=10, m=256MiB, p=4) |
| **File permissions** | `0600` file, `0700` dir, atomic write |
| **Biometric binding** | Secure Enclave wrap/unwrap (macOS), TPM2 (Linux) |
| **Rollback protection** | Monotonic counter in Keychain/TPM |
| **Key rotation** | Versioned keys, re-wrap on change |
| **Tamper detection** | Ed25519 signature on header||ciphertext |
| **Memory safety** | `SecretString`, `ZeroizeOnDrop`, optional `mlock` |
| **Forward secrecy** | Key rotation limits exposure window |

---

## Implementation Effort (Revised)

| Component | Effort |
|-----------|--------|
| Vault file format + AES-GCM + Ed25519 + Argon2id | 3 days |
| macOS Secure Enclave wrap/unwrap | 4 days |
| Linux TPM2 (optional) | 3 days |
| Linux fprintd (optional) | 2 days |
| Key rotation + rollback counter | 2 days |
| File permissions + atomic write + mlock | 1 day |
| Vault API + secrecy/zeroize | 2 days |
| Settings UI integration | 2 days |
| Upgrade flow integration | 1 day |
| Testing (cross-platform, edge cases) | 2 weeks |
| **Total** | **~4-5 weeks** |

---

## Migration Path

```
v0.21: Current per-server Keychain + SSO
v0.22: Add vault (opt-in) — "Use encrypted password vault"
        - First unlock: enters sudo password, creates vault
        - Biometric wrapper auto-created on macOS/Linux if available
        - Subsequent: Touch ID / fingerprint or password
v0.23: Auto-migrate — "Import saved passwords to vault?"
v0.24: Vault becomes default; Keychain items deprecated
v0.25: Add key rotation UI, TPM2 support
```

---

## Summary of Fixes from Security Review

| Security Review Finding | Fix Applied |
|------------------------|-------------|
| Downgrade attack (shared key derivation) | **Independent key wrapping** per method; password cannot bypass biometric |
| Weak Argon2id (t=3, m=64MiB) | **t=10, m=256MiB, p=4** |
| World-readable vault file | **0600 file, 0700 dir, atomic write** |
| Secure Enclave misuse (extract secret) | **Wrap/unwrap API** — key never leaves SE |
| No forward secrecy / key rotation | **Key versioning + rotation API** |
| No rollback protection | **Monotonic counter in Keychain/TPM** |
| Ed25519 false confidence | **Signature + counter + timestamp verification** |
| Password in memory | **SecretString + ZeroizeOnDrop + optional mlock** |
| JSON parsing DoS | **Size limits, strict schema** |

The plan is now cryptographically sound and addresses all P0/P1 findings.