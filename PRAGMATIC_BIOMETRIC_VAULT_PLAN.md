# Pragmatic Biometric Vault — Plan (with System Password Fallback)

## Core Idea

Use biometric (Touch ID / fingerprint) as **convenience** for unlocking a per-server password vault, but fall back to **system password** (the same sudo password you'd type anyway) if biometric fails or isn't available.

This eliminates the need for a separate "master passphrase" — the system password IS the fallback.

## Architecture

```
~/.config/multitop/vault.pqenc
    │
    ├── Encrypted with key derived from:
    │     ├── Biometric-bound secret (Secure Enclave / TPM2)  OR
    │     └── System password (Argon2id) — same as sudo password
    │
    └── Contains: { "host:port": "sudo_password", ... }
```

## Unlock Flow

```
User presses 'u' (upgrade)
    │
    ▼
Try biometric unlock (Touch ID / fprintd)
    │
    ├── Success → Derive key from biometric-bound secret → Decrypt vault ✓
    │
    └── Failure / unavailable / cancelled
            │
            ▼
        Prompt: "Enter sudo password to unlock vault"
            │
            ▼
        Derive key with Argon2id(system_password) → Decrypt vault ✓
            │
            ▼
        (Optional) Re-encrypt vault with biometric-bound key for next time
```

## Why This Works

| Scenario | Behavior |
|----------|----------|
| MacBook with Touch ID | Touch ID → instant unlock |
| Linux with fingerprint reader | Fingerprint → instant unlock |
| Headless server / no biometric | Type sudo password once → unlocks vault for all servers |
| Biometric fails (wet fingers, mask, dirty sensor) | Type sudo password → works |
| First run on new machine | Type sudo password → creates vault bound to that password |

**Key insight**: The "system password" is exactly what you'd type for `sudo` anyway. No extra secret to remember.

## Cryptographic Design (Simplified)

### Vault Encryption Key Derivation

```rust
// Two paths to the same 32-byte vault key:

// Path 1: Biometric-bound (preferred)
fn derive_key_biometric(biometric_secret: &[u8; 32], salt: &[u8; 32]) -> [u8; 32] {
    HKDF-SHA3-256(IKM=biometric_secret, salt, info="multitop-vault-v1")
}

// Path 2: System password (fallback)
fn derive_key_password(password: &str, salt: &[u8; 32]) -> [u8; 32] {
    Argon2id(password, salt, t=3, m=64MiB, p=4)  // ~1 second on modern CPU
}
```

### Vault File Format

```binary
vault.pqenc:
┌─────────────────────────────────────────────────────────────┐
│ Magic: "MQV1" (4 bytes)                                     │
├─────────────────────────────────────────────────────────────┤
│ Version: 1 (1 byte)                                         │
├─────────────────────────────────────────────────────────────┤
│ KDF Mode: 0x01 = Biometric-bound, 0x02 = Argon2id(password) │
├─────────────────────────────────────────────────────────────┤
│ Salt: 32 bytes                                              │
├─────────────────────────────────────────────────────────────┤
│ Biometric hint: (optional) public key / key handle          │
├─────────────────────────────────────────────────────────────┤
│ Nonce: 12 bytes (AES-GCM)                                   │
├─────────────────────────────────────────────────────────────┤
│ Ciphertext: variable (encrypted JSON vault)                 │
├─────────────────────────────────────────────────────────────┤
│ Signature: Ed25519 (classical, fast) over header||ciphertext │
└─────────────────────────────────────────────────────────────┘
```

**Note**: Ed25519 for signature (not post-quantum) — it's only for tamper detection, not long-term secrecy. The encryption uses AES-256-GCM with keys derived from either biometric or password. Post-quantum KEM not needed since the vault key never leaves the device.

## Biometric Implementation

### macOS (Touch ID / Face ID)

```rust
// Using security-framework + Secure Enclave
use security_framework::os::macos::secure_enclave::*;

// 1. Generate EC P-256 key in Secure Enclave (once, on vault creation)
let key = SecKey::generate_in_secure_enclave()?;

// 2. On unlock: use Touch ID to authorize key use
let context = LAContext();
context.evaluate_policy(.deviceOwnerAuthenticationWithBiometrics, "Unlock vault")?;
let biometric_secret = key.unwrap_symmetric_key()?;  // Requires biometric

// 3. Derive vault key
let vault_key = HKDF(biometric_secret, salt, "multitop-vault-v1");
```

**Fallback**: If Secure Enclave unavailable (older Mac), skip to password path.

### Linux (fprintd + TPM2 optional)

```rust
// Option A: TPM2 (best) - seal key to PCR + biometric auth
// Option B: fprintd verify + file-based key encrypted with Argon2id
// Option C: Just use fprintd verify, then password derivation

// Simplest cross-distro (Option B):
// 1. Vault creation: generate random key, encrypt with Argon2id(password), store
// 2. Unlock: fprintd verify → if success, decrypt key file with Argon2id(password)
//    (password entered once, cached in memory for session)
```

**Even simpler**: Skip biometric on Linux entirely, just use system password. Biometric is a nice-to-have optimization.

## Vault Operations

```rust
pub struct Vault {
    path: PathBuf,
    // In-memory only when unlocked
    passwords: Option<HashMap<String, String>>,  // Zeroized on drop
}

impl Vault {
    // Create new vault (first run)
    pub fn create(passwords: HashMap<String, String>, system_password: &str) -> Result<Self> {
        let salt = random_32_bytes();
        let key = Argon2id(system_password, salt);
        let ciphertext = AES256GCM(key, passwords_json);
        let sig = Ed25519_sign(signing_key, header || ciphertext);
        write_vault_file(path, header, ciphertext, sig);
        // Optionally: try to create biometric-bound version for next time
    }

    // Unlock with biometric (async, may prompt)
    pub async fn unlock_biometric(&mut self) -> Result<()> {
        let biometric_secret = try_biometric_unlock()?;  // Touch ID / fprintd
        let key = HKDF(biometric_secret, header.salt);
        self.decrypt_and_verify(key)
    }

    // Unlock with system password (sync, always works)
    pub fn unlock_password(&mut self, password: &str) -> Result<()> {
        let key = Argon2id(password, header.salt);
        self.decrypt_and_verify(key)
    }

    // Unified unlock: try biometric, fall back to password
    pub async fn unlock(&mut self) -> Result<()> {
        if let Ok(()) = self.unlock_biometric().await {
            return Ok(());
        }
        // Biometric failed/unavailable - prompt for password
        let password = prompt_password("Enter sudo password to unlock vault:")?;
        self.unlock_password(&password)
    }

    pub fn get(&self, host: &str) -> Option<&String> {
        self.passwords.as_ref()?.get(host)
    }
}
```

## Integration with Upgrade Flow

```rust
// app.rs:confirm_upgrade()
pub fn confirm_upgrade(&mut self) -> Vec<Command> {
    self.show_upgrade_modal = false;
    
    // Load or create vault
    let mut vault = Vault::load_or_create()?;
    
    // Unlock (biometric → password fallback)
    vault.unlock().await?;  // Shows Touch ID prompt or password prompt
    
    self.vault = Some(vault);
    self.run_upgrade()
}

// tasks.rs:spawn_upgrade()
pub fn spawn_upgrade(...) {
    // Get password for this server
    let pass = app.vault.as_ref()
        .and_then(|v| v.get(&server.key()))
        .cloned()
        .or_else(|| load_sso())           // Fallback 1: SSO
        .or_else(|| load_override(&server)); // Fallback 2: per-server Keychain
    
    ssh::spawn_command(&server, &command, pass.as_deref())?;
}
```

## Password Management UI

```
Configuration (press 'e')
    │
    ├── [Tab] Sudo Passwords
    │       │
    │       ├── Vault status: ● Locked  /  ○ Unlocked (biometric)  /  ○ Unlocked (password)
    │       │
    │       ├── [U] Unlock Vault (biometric or password)
    │       ├── [L] Lock Vault
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

## Security Properties

| Property | Mechanism |
|----------|-----------|
| **At-rest encryption** | AES-256-GCM with key from biometric OR Argon2id(password) |
| **Tamper detection** | Ed25519 signature on vault file |
| **Biometric binding** | Secure Enclave (macOS) / TPM2 or fprintd (Linux) |
| **Offline attack resistance** | Argon2id(t=3, m=64MiB) for password path |
| **No separate master secret** | System password = fallback = what you type for sudo anyway |
| **Memory safety** | `zeroize` crate clears passwords on drop |

## Advantages Over Previous Plans

| Aspect | SSO Plan | Master Key Vault | PQ Biometric Vault | **This Plan** |
|--------|----------|------------------|-------------------|---------------|
| Separate master password | No | Yes | Yes | **No** |
| Biometric convenience | No | Optional | Yes | **Yes** |
| Works headless | Yes | Yes | No | **Yes** |
| Post-quantum | No | No | Yes | **No (not needed)** |
| Complexity | Low | Medium | Very High | **Medium** |
| "It just works" | Yes | Mostly | Only with biometric | **Yes** |

## Why Not Post-Quantum?

1. **Threat model mismatch**: Vault never leaves device. Quantum attacker needs physical access + vault file. At that point, they can just read memory.
2. **Complexity cost**: Kyber + Dilithium = ~50KB code, 2+ new dependencies, NIST standards still stabilizing.
3. **AES-256-GCM is fine**: 256-bit symmetric = 128-bit quantum security (Grover's algorithm). Sufficient for vault lifetime.
4. **System password fallback**: If quantum computer breaks AES-256, it breaks everything else too.

## Implementation Effort

| Component | Effort |
|-----------|--------|
| Vault file format + AES-GCM + Ed25519 | 2 days |
| macOS Touch ID + Secure Enclave | 3 days |
| Linux fprintd (optional) | 2 days |
| Argon2id password derivation | 1 day |
| Vault API + zeroize | 1 day |
| Settings UI integration | 2 days |
| Upgrade flow integration | 1 day |
| **Total** | **~12 days** |

## Migration Path

```
v0.21: Current per-server Keychain items + SSO
v0.22: Add vault (opt-in) — "Use encrypted password vault"
        - First unlock: enters sudo password, creates vault
        - Subsequent: Touch ID or password
v0.23: Auto-migrate — "Import saved passwords to vault?"
v0.24: Vault becomes default; Keychain items deprecated
```

## Decision

**This is the pragmatic choice.** It gives biometric convenience where available, falls back to the password you already know (sudo), requires no extra secrets, and avoids post-quantum complexity that doesn't match the threat model.