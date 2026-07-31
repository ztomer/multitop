# Pragmatic Biometric Vault — Final Fixed Plan (Post Security Reviews)

## Core Idea

Use biometric (Touch ID / fingerprint) as **convenience** for unlocking a per-server password vault, with **system sudo password** as fallback. No separate master passphrase.

**Key security invariant**: Multiple **independent** wrapped forms of the same `vault_key` — no shared derivation between biometric and password paths. Eliminates downgrade attack.

---

## Vault File Format (Version 2)

```binary
~/.local/share/multitop/vault.bin (0600, dir 0700)
┌────────────────────────────────────────────────────────────────────┐
│ Magic: "MQV2" (4 bytes)                                            │
├────────────────────────────────────────────────────────────────────┤
│ Version: 2 (1 byte)                                                │
├────────────────────────────────────────────────────────────────────┤
│ Key Version: u8 (incremented on rotation)                          │
├────────────────────────────────────────────────────────────────────┤
│ Created Timestamp: u64 (Unix ms)                                   │
├────────────────────────────────────────────────────────────────────┤
│ Monotonic Counter: u32 (incremented on each save, prevents rollback)│
├────────────────────────────────────────────────────────────────────┤
│ Salt: 32 bytes (random per vault)                                  │
├────────────────────────────────────────────────────────────────────┤
│ Argon2id Params: t=u8, m=u32 (KiB), p=u8  (stored in header)       │
├────────────────────────────────────────────────────────────────────┤
│ Wrapped Key Entries (TLV array):                                   │
│   ┌────────────────────────────────────────────────────────────┐   │
│   │ Type: u8 (0x01=SecureEnclave, 0x02=TPM2, 0x03=Argon2id)    │   │
│   ├────────────────────────────────────────────────────────────┤   │
│   │ Length: u16                                                │   │
│   ├────────────────────────────────────────────────────────────┤   │
│   │ Data: variable (wrapped key blob)                          │   │
│   └────────────────────────────────────────────────────────────┘   │
│   (repeated; 0x03 ALWAYS present)                                 │
├────────────────────────────────────────────────────────────────────┤
│ Nonce: 12 bytes (random per encryption)                            │
├────────────────────────────────────────────────────────────────────┤
│ Ciphertext: AES-256-GCM(vault_key, JSON)                           │
├────────────────────────────────────────────────────────────────────┤
│ Ed25519 Signature (HKDF(vault_key, "signing") over header||ct)   │
└────────────────────────────────────────────────────────────────────┘
```

---

## Key Wrapping Methods (Independent)

### 0x01: macOS Secure Enclave (Preferred on macOS)

**Creation** (once per vault):
```rust
// 1. Generate random vault_key (32 bytes)
let vault_key = random_32_bytes();

// 2. Create EC P-256 key in Secure Enclave (persistent, biometric-gated)
let se_key = SecKey::generate_in_secure_enclave(
    label: "multitop-vault-key",
    require_biometric: true,
    access_control: kSecAccessControlDevicePasscode  // + biometric
)?;

// 3. Wrap vault_key with Secure Enclave key (AES-KW or direct encrypt)
//    This produces a blob that ONLY Secure Enclave can unwrap, and ONLY with biometric
let wrapped = se_key.wrap_key(&vault_key)?;  

// 4. Store: wrapped blob + SE key handle (persistent reference)
```

**Unlock**:
```rust
// 1. Get SE key handle from vault header
// 2. LAContext.evaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, "Unlock vault")
// 3. se_key.unwrap_key(&wrapped) → vault_key (requires biometric)
// 4. Use vault_key to decrypt vault
```

**Properties**: Key never leaves Secure Enclave. Biometric required **every unwrap**. No password fallback for this wrapping type.

**Key invalidation handling**: If unwrap fails with `errSecInvalidKey` / `errSecAuthFailed` (SE key invalidated by macOS update, Touch ID disable, hardware repair):
- Silently remove this wrapper from vault
- Fall back to Argon2id path
- On successful password unlock, re-create SE wrapper (re-bind)

### 0x02: Linux TPM2 (Optional, Advanced)

**Not in MVP**. Deferred to v0.23+ behind feature flag. Primary Linux path is fprintd + Argon2id.

### 0x03: Argon2id (Password Fallback — ALWAYS Present)

**Creation**:
```rust
// 1. vault_key = random_32_bytes()
// 2. params = auto_detect_argon2_params()  // or user config
// 3. key = Argon2id(password, salt, t, m, p)
// 4. wrapped = AES-256-GCM(key, vault_key, nonce=random_12())
// 5. Store: wrapped (nonce||ciphertext||tag) + params (t,m,p) in header
```

**Auto-detected params** (conservative defaults):
| Device Class | t | m (MiB) | p | Est. Time |
|--------------|---|---------|---|-----------|
| Desktop/Server | 10 | 256 | 4 | ~3-5s |
| Laptop | 8 | 128 | 4 | ~2-3s |
| CI/Container (mem limit < 512MiB) | 4 | 64 | 4 | ~1-2s |
| Embedded (Pi Zero) | 3 | 32 | 1 | ~1s |

**Config override** (config.toml):
```toml
[vault]
argon2_t = 10
argon2_m_mib = 256
argon2_p = 4
```

**Unlock**:
```rust
// 1. Prompt: "Enter sudo password to unlock vault"
// 2. key = Argon2id(password, salt, t, m, p)  // params from header
// 3. vault_key = AES-256-GCM-Decrypt(key, wrapped)
// 4. Verify vault_key decrypts vault correctly (GCM tag)
// 5. RATE LIMIT CHECK (see below)
// 6. On success: if biometric available → re-bind (re-create 0x01 wrapper)
```

---

## Unlock Flow (No Downgrade)

```rust
pub async fn unlock(&mut self) -> Result<()> {
    // 1. Read vault file (atomic: read to vec, then parse)
    let bytes = fs::read(&self.path)?;
    
    // 2. Parse header, verify magic/version
    let header = parse_header(&bytes)?;
    
    // 3. VERIFY SIGNATURE BEFORE DECRYPT (fail fast on tampering)
    //    signing_key = HKDF(header.salt, "signing") from vault_key
    //    But we don't have vault_key yet! 
    //    → Ed25519 public key stored in header (derived from vault_key at creation)
    //    → Verify signature over header_without_sig || ciphertext
    verify_ed25519(&header.ed25519_pk, &header.signed_data, &header.signature)?;
    
    // 4. Try wrappers in priority order
    let vault_key = match try_wrappers(&header) {
        Some(key) => key,
        None => {
            // 5. All biometric/TPM failed or unavailable → password path
            let password = prompt_password("Enter sudo password to unlock vault:")?;
            try_argon2id(&header, &password)?
        }
    };
    
    // 6. Decrypt ciphertext with vault_key
    let plaintext = aes256gcm_decrypt(vault_key, header.nonce, &ciphertext)?;
    
    // 7. Validate JSON schema
    let passwords: HashMap<String, SecretString> = 
        serde_json::from_slice(&plaintext)?;
    validate_passwords_schema(&passwords)?;
    
    // 8. Store in memory (zeroized on drop)
    self.vault_key = Some(Zeroizing::new(vault_key));
    self.passwords = Some(passwords);
    
    // 9. Re-bind if used password but biometric available
    if used_password_path && biometric_available() {
        rebind_biometric(&mut header, &password).await?;
    }
    
    Ok(())
}
```

---

## Rate Limiting on Password Attempts (P1)

**In-memory state** (per process):
```rust
struct RateLimiter {
    attempts: u32,
    last_attempt: Instant,
    locked_until: Option<Instant>,
}

impl RateLimiter {
    fn check(&mut self) -> Result<(), LockoutError> {
        if let Some(until) = self.locked_until {
            if Instant::now() < until {
                return Err(LockoutError::LockedUntil(until));
            }
            self.locked_until = None;
        }
        Ok(())
    }
    
    fn record_failure(&mut self) {
        self.attempts += 1;
        self.last_attempt = Instant::now();
        match self.attempts {
            3 => self.locked_until = Some(Instant::now() + Duration::from_secs(1)),
            4 => self.locked_until = Some(Instant::now() + Duration::from_secs(2)),
            5 => self.locked_until = Some(Instant::now() + Duration::from_secs(4)),
            6 => self.locked_until = Some(Instant::now() + Duration::from_secs(8)),
            7 => self.locked_until = Some(Instant::now() + Duration::from_secs(16)),
            8 => self.locked_until = Some(Instant::now() + Duration::from_secs(32)),
            9 => self.locked_until = Some(Instant::now() + Duration::from_secs(60)),
            10.. => self.locked_until = Some(Instant::now() + Duration::from_secs(300)),
        }
    }
    
    fn record_success(&mut self) {
        self.attempts = 0;
        self.locked_until = None;
    }
}
```

**Persisted in vault header** (survives process restart):
```rust
// Header field: failed_attempts: u8, lockout_until: u64 (Unix ms)
// On unlock attempt:
//   if lockout_until > now() → reject immediately
//   on failure: increment failed_attempts, update lockout_until
//   on success: failed_attempts = 0, lockout_until = 0
```

---

## Secure Enclave Key Invalidation Handling

```rust
fn try_secure_enclave(header: &Header) -> Option<[u8; 32]> {
    let wrapped = header.get_wrapper(0x01)?;
    let key_handle = load_se_key_handle(wrapped.handle)?;
    
    let context = LAContext::new();
    context.evaluate_policy(LAPolicy::DeviceOwnerAuthenticationWithBiometrics, 
                            "Unlock multitop vault")?;
    
    match key_handle.unwrap_key(&wrapped.blob) {
        Ok(vault_key) => Some(vault_key),
        Err(err) if err.code() == errSecInvalidKey || err.code() == errSecAuthFailed => {
            // Key invalidated (macOS update, Touch ID disabled, hardware repair)
            // Remove this wrapper, fall back to password
            remove_wrapper(header, 0x01);
            None
        }
        Err(err) => {
            // Other error (user cancelled, biometric not enrolled, etc.)
            None
        }
    }
}
```

---

## Re-bind Biometric After Password Unlock

```rust
async fn rebind_biometric(header: &mut Header, password: &str) -> Result<()> {
    // Only if biometric hardware available and not already bound
    if !biometric_available() || header.has_wrapper(0x01) {
        return Ok(());
    }
    
    // Generate new SE key + wrap current vault_key
    let se_key = SecKey::generate_in_secure_enclave(...)?;
    let wrapped = se_key.wrap_key(&vault_key)?;
    
    // Add wrapper to header
    header.add_wrapper(0x01, wrapped);
    header.increment_counter();
    
    // Rewrite vault file ATOMICALLY
    atomic_write_vault(&header, &ciphertext)?;
    
    Ok(())
}
```

**Atomic write**: Write to `vault.bin.tmp`, `fsync`, `rename` (POSIX atomic replace).

---

## Vault Migration Framework

```rust
// Versioned migration trait
trait VaultMigration {
    fn from_version(&self) -> u8;
    fn to_version(&self) -> u8;
    fn migrate(&self, vault: &mut VaultV1) -> Result<VaultV2>;
}

// Registry
static MIGRATIONS: &[&dyn VaultMigration] = &[
    &V1ToV2Migration,  // adds Argon2id params in header, Ed25519 sig
    &V2ToV3Migration,  // adds TPM2 wrapper type, key versioning
];

pub fn migrate_if_needed(path: &Path) -> Result<()> {
    let current_version = read_version(path)?;
    let target_version = CURRENT_VERSION;
    
    if current_version == target_version {
        return Ok(());
    }
    
    // Apply migrations sequentially
    let mut vault = load_vault(path)?;
    for migration in MIGRATIONS.iter().filter(|m| m.from_version() > current_version) {
        vault = migration.migrate(vault)?;
    }
    
    // Save with new version
    save_vault(path, &vault)?;
    Ok(())
}
```

**Test in CI**: Generate v1 vault → run migration → verify v2 vault decrypts with same passwords.

---

## File Permissions & Atomic Operations

```rust
fn atomic_write_vault(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("bin.tmp");
    
    // Write with 0600
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    
    file.write_all(data)?;
    file.flush()?;
    file.sync_all()?;  // Ensure data on disk
    
    // Atomic rename
    fs::rename(&tmp, path)?;
    
    // Sync directory (ensure rename persisted)
    let dir = path.parent().unwrap();
    let dir_fd = OpenOptions::new().read(true).open(dir)?;
    dir_fd.sync_all()?;
    
    Ok(())
}
```

**Directory creation** (on first run):
```rust
fn ensure_vault_dir() -> Result<PathBuf> {
    let dir = dirs::data_local_dir()
        .unwrap()
        .join("multitop");
    
    fs::create_dir_all(&dir)?;
    set_permissions(&dir, Permissions::from_mode(0o700))?;
    
    Ok(dir.join("vault.bin"))
}
```

---

## Memory Safety (Zeroize + Secrecy)

```rust
use secrecy::{Secret, SecretString, ExposeSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(ZeroizeOnDrop)]
struct VaultKey([u8; 32]);

#[derive(ZeroizeOnDrop)]
struct UnlockedVault {
    vault_key: VaultKey,
    passwords: HashMap<String, SecretString>,  // SecretString zeroizes on drop
    _rate_limiter: RateLimiter,
}

impl UnlockedVault {
    fn get_password(&self, host: &str) -> Option<Secret<String>> {
        self.passwords.get(host).cloned()
    }
}

// Usage in spawn_upgrade:
let pass = app.vault.as_ref()
    .and_then(|v| v.get_password(&server.key()))
    .map(|s| s.expose_secret().to_string())  // Only at point of use
    .or_else(|| load_sso())
    .or_else(|| load_override(&server));
```

---

## Settings UI (Passwords Tab)

```
Configuration (press 'e')
    │
    ├── [Tab] Sudo Passwords
    │       │
    │       ├── Vault: ● Locked    (or: ○ Unlocked via Touch ID / ○ Unlocked via password)
    │       │
    │       ├── [U] Unlock Vault
    │       │       → Biometric prompt → success: "Vault unlocked"
    │       │       → Cancel → Password prompt → success: "Vault unlocked"
    │       │
    │       ├── [L] Lock Vault
    │       │
    │       ├── [R] Re-bind Biometric (visible only if unlocked via password)
    │       │
    │       ├── [A] Add Server Password
    │       │       Host: server.example.com
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

## Dependencies

| Crate | Purpose |
|-------|---------|
| `aes-gcm` | AES-256-GCM encryption |
| `argon2` | Argon2id key derivation |
| `ed25519-dalek` | Ed25519 signatures |
| `hkdf` | Key derivation (signing key from vault_key) |
| `secrecy` | `SecretString`, `ExposeSecret` |
| `zeroize` | `ZeroizeOnDrop` for keys |
| `security-framework` | macOS Secure Enclave, Keychain, LAContext |
| `local-auth` | macOS Touch ID (simpler LAContext wrapper) |
| `zbus` / `fprintd-async` | Linux fingerprint |
| `dirs` | XDG data dir (`~/.local/share/multitop`) |
| `serde_json` | Vault JSON (with schema validation) |
| `tokio` | Async biometric prompts |

---

## Integration with Upgrade Flow

```rust
// app.rs:confirm_upgrade()
pub async fn confirm_upgrade(&mut self) -> Vec<Command> {
    self.show_upgrade_modal = false;
    
    // Load or create vault
    let mut vault = Vault::load_or_create()?;
    
    // Unlock (biometric → password fallback)
    vault.unlock().await?;
    
    self.vault = Some(vault);
    self.run_upgrade()
}

// tasks.rs:spawn_upgrade()
pub fn spawn_upgrade(idx, gen, server, app, tx) {
    let pass = app.vault.as_ref()
        .and_then(|v| v.get_password(&server.key()))
        .map(|s| s.expose_secret().to_string())
        .or_else(|| crate::password_store::load_sso().ok().flatten())
        .or_else(|| crate::password_store::load(&server).ok().flatten());
    
    ssh::spawn_command(&server, &command, pass.as_deref())?;
}
```

---

## Config Options (config.toml)

```toml
[vault]
# Argon2id parameters (auto-detected if not set)
# argon2_t = 10
# argon2_m_mib = 256
# argon2_p = 4

# Vault file location (default: ~/.local/share/multitop/vault.bin)
# path = "/custom/path/vault.bin"
```

---

## Testing Requirements

| Test | Description |
|------|-------------|
| `test_vault_create_unlock_password` | Create vault, unlock with password, verify passwords accessible |
| `test_vault_biometric_rebind` | Unlock with password → re-bind → unlock with biometric |
| `test_vault_rate_limit` | 3 failures → 1s delay, 10 failures → 5min lockout |
| `test_vault_migration_v1_v2` | Load v1 vault fixture → migrate → decrypt with v2 code |
| `test_vault_signature_verification` | Tamper ciphertext → unlock fails before decrypt |
| `test_vault_atomic_write` | Crash during write → vault.bin either old or new, never corrupt |
| `test_vault_permissions` | vault.bin 0600, dir 0700 |
| `test_se_key_invalidation` | Mock SE error → fallback to password → re-bind works |
| `test_concurrent_unlock` | Two processes unlock same vault → no corruption |

---

## Effort Estimate (Final)

| Component | Days |
|-----------|------|
| Vault format + crypto + Argon2id auto-detect | 4 |
| macOS Secure Enclave (create, unwrap, invalidate handling, rebind) | 5 |
| Linux fprintd + password (TPM2 optional, not MVP) | 3 |
| Rate limiting + lockout (memory + persisted) | 2 |
| Migration framework + v1→v2 test | 3 |
| Atomic write + permissions + mlock | 1 |
| Vault API + Secrecy + Zeroize | 2 |
| Settings UI (unlock, rebind, add/delete, list) | 3 |
| Upgrade integration | 1 |
| Cross-platform testing (macOS, Linux, CI) | 10 |
| **Total** | **~34 days (6-7 weeks)** |

---

## Security Properties (Final)

| Property | Mechanism |
|----------|-----------|
| **Confidentiality** | AES-256-GCM with vault_key (32 bytes random) |
| **Integrity** | AES-GCM tag + Ed25519 signature (key = HKDF(vault_key, "signing")) |
| **Biometric binding** | Secure Enclave key (macOS) — key never leaves SE, biometric per unwrap |
| **Password fallback** | Argon2id(t=10, m=256MiB, p=4) — params stored in header, auto-detected |
| **No downgrade** | Independent wrapped forms — biometric and password paths use DIFFERENT wrapping |
| **Rollback protection** | Monotonic counter in signed header |
| **Offline attack resistance** | Argon2id memory-hard; vault file 0600 in 0700 dir |
| **Online attack resistance** | Rate limiting + exponential backoff + persistent lockout |
| **Forward secrecy** | Key versioning — rotate on password change / biometric re-enroll |
| **Memory safety** | `zeroize` + `secrecy` — keys/passwords zeroed on drop |
| **Crash safety** | Atomic write (tmp + rename + dir fsync) |
| **Migration safety** | Versioned migrations tested in CI |

---

## What's NOT in This Plan (Deferred)

- TPM2 wrapper (Linux) — behind feature flag, v0.23+
- Vault sync across machines — each device has own vault
- Hardware security module (HSM) support
- Post-quantum KEM — not needed for local vault threat model
- Biometric on Windows (WSL2) — not applicable

---

## Approval

This plan addresses all P0/P1 issues from both security reviews. Ready for implementation.