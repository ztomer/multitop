# Post-Quantum Biometric Vault — Plan

## Concept

A custom encrypted password vault for per-server sudo passwords, using:
- **Post-quantum encryption**: ML-KEM (Kyber) + ML-DSA (Dilithium) or hybrid classical+PQ
- **Biometric unlock**: Touch ID/Face ID on macOS (Secure Enclave), fprintd on Linux
- **No Keychain dependency**: Self-contained encrypted file, unlocked via biometric auth

## Architecture

```
~/.config/multitop/vault.pqenc
    │
    ├── Header: version, KEM algorithm, KDF params, salt
    ├── Ephemeral public key (for KEM)
    ├── Ciphertext (encrypted vault)
    └── Signature (ML-DSA)

Unlock flow:
    User presses 'u' (upgrade)
    │
    ▼
Biometric prompt: "Authenticate to unlock sudo vault"
    │
    ├── macOS: LAContext.evaluatePolicy(.deviceOwnerAuthenticationWithBiometrics)
    │
    └── Linux: polkit/fprintd via dbus (org.freedesktop.login1)
    │
    ▼
Success → Derive key from biometric-bound secret → Decrypt vault
    │
    ▼
Per-server passwords available for upgrade
```

## Cryptographic Design

### Key Encapsulation (ML-KEM / Kyber)

```
KeyGen() → (pk, sk)
Encaps(pk) → (ciphertext, shared_secret)
Decaps(sk, ciphertext) → shared_secret
```

- **Algorithm**: ML-KEM-768 (NIST FIPS 203) — 128-bit quantum security
- **Library**: `pqcrypto-kyber` (Rust) or `oqs` (Open Quantum Safe)
- **Hybrid option**: X25519 + ML-KEM-768 (defense in depth)

### Key Derivation

```
master_key = HKDF-SHA3-256(
    IKM = shared_secret || device_binding,
    salt = vault_salt,
    info = "multitop-vault-v1"
)
```

**Device binding**: On macOS, use Secure Enclave to bind key to device (key cannot be extracted). On Linux, bind to TPM2 if available, else fallback to file-based with strong KDF.

### Vault Encryption (AEAD)

```
ciphertext = AES-256-GCM(master_key, nonce, plaintext, aad)
```

- **Nonce**: Random 96-bit per vault
- **AAD**: Vault version + host identifier
- **Plaintext**: JSON `{ "host:port": "password", ... }`

### Signature (ML-DSA / Dilithium)

```
Sign(sk, message) → signature
Verify(pk, message, signature) → bool
```

- **Algorithm**: ML-DSA-65 (NIST FIPS 204)
- **Purpose**: Authenticate vault file, prevent tampering
- **Message**: Header || ciphertext

## Biometric Integration

### macOS (Touch ID / Face ID)

```swift
// Via Swift bridging or objc crate
let context = LAContext()
context.localizedReason = "Unlock multitop sudo password vault"
let success = try await context.evaluatePolicy(
    .deviceOwnerAuthenticationWithBiometrics,
    localizedReason: reason
)
// On success, use LASecureEnclave to derive/unwrap key
```

**Rust integration options:**
1. `security-framework` crate + custom Secure Enclave key operations
2. `local-auth` crate (Touch ID only, no Secure Enclave key storage)
3. Small Swift helper binary called via `Command` (simplest)

**Secure Enclave approach:**
- Generate EC P-256 key in Secure Enclave (`kSecAttrTokenIDSecureEnclave`)
- Use it to wrap/unwrap the vault master key
- Key never leaves Secure Enclave — biometric required for each unwrap

### Linux (fprintd / polkit)

```rust
// Via dbus (zbus crate)
let proxy = fprintd::Device::new(&connection).await?;
let verify = proxy.VerifyFinger("multitop-vault", user).await?;
// On success, use TPM2 (tss-esapi) or fallback to Argon2id(passphrase)
```

**Options:**
1. **TPM2** (preferred): Seal/unseal vault key with PCR policy + biometric auth
2. **fprintd + file**: Verify fingerprint, then decrypt key file with Argon2id(passphrase)
3. **polkit**: `pkexec` helper that requires biometric, returns decrypted key

**Simplest cross-distro**: fprintd verification + Argon2id-encrypted key file (no TPM required)

## Vault File Format

```binary
vault.pqenc:
┌─────────────────────────────────────────────────────────────┐
│ Magic: "MQV1" (4 bytes)                                     │
├─────────────────────────────────────────────────────────────┤
│ Version: 1 (1 byte)                                         │
├─────────────────────────────────────────────────────────────┤
│ KEM Algorithm: 0x01 = ML-KEM-768, 0x02 = Hybrid X25519+KEM │
├─────────────────────────────────────────────────────────────┤
│ KDF Algorithm: 0x01 = HKDF-SHA3-256                         │
├─────────────────────────────────────────────────────────────┤
│ Salt: 32 bytes                                              │
├─────────────────────────────────────────────────────────────┤
│ KEM Ciphertext: 1088 bytes (ML-KEM-768) or 1152 (hybrid)   │
├─────────────────────────────────────────────────────────────┤
│ Nonce: 12 bytes (for AES-GCM)                               │
├─────────────────────────────────────────────────────────────┤
│ Ciphertext: variable (vault JSON + padding)                 │
├─────────────────────────────────────────────────────────────┤
│ Signature Algorithm: 0x01 = ML-DSA-65                       │
├─────────────────────────────────────────────────────────────┤
│ Signature: 3309 bytes (ML-DSA-65)                           │
└─────────────────────────────────────────────────────────────┘
```

## Implementation Plan

### Phase 1: Core Crypto (Rust)

```rust
// crates/vault/src/crypto.rs
use pqcrypto_kyber::kyber768;
use pqcrypto_dilithium::dilithium3;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use hkdf::Hkdf;
use sha3::Sha3_256;

pub fn generate_keypair() -> (PublicKey, SecretKey) { ... }
pub fn encapsulate(pk: &PublicKey) -> (Ciphertext, SharedSecret) { ... }
pub fn decapsulate(sk: &SecretKey, ct: &Ciphertext) -> SharedSecret { ... }
pub fn encrypt_vault(master_key: &[u8; 32], plaintext: &[u8]) -> VaultCiphertext { ... }
pub fn decrypt_vault(master_key: &[u8; 32], ct: &VaultCiphertext) -> Vec<u8> { ... }
pub fn sign_vault(sk: &SigningKey, data: &[u8]) -> Signature { ... }
pub fn verify_vault(pk: &VerifyingKey, data: &[u8], sig: &Signature) -> bool { ... }
```

### Phase 2: Biometric Unlock (Platform-Specific)

```rust
// crates/vault/src/biometric.rs
#[cfg(target_os = "macos")]
mod macos {
    use security_framework::os::macos::secure_enclave::*;
    pub async fn unlock_with_touchid(key_handle: &SecKey) -> Result<[u8; 32], Error> { ... }
}

#[cfg(target_os = "linux")]
mod linux {
    use zbus::Connection;
    pub async fn unlock_with_fingerprint() -> Result<[u8; 32], Error> { ... }
    // Falls back to TPM2 (tss-esapi) or Argon2id passphrase
}

pub async fn unlock_vault_key() -> Result<[u8; 32], Error> { ... }
```

### Phase 3: Vault API

```rust
// crates/vault/src/lib.rs
pub struct Vault {
    path: PathBuf,
    kem_pk: PublicKey,
    sig_pk: VerifyingKey,
}

impl Vault {
    pub fn create(passwords: HashMap<String, String>) -> Result<Self, Error> { ... }
    pub fn open(path: &Path) -> Result<Self, Error> { ... }
    pub async fn unlock(&self) -> Result<UnlockedVault, Error> { ... }
    pub fn get_password(&self, host: &str) -> Option<String> { ... }
    pub fn set_password(&mut self, host: &str, pass: String) -> Result<(), Error> { ... }
    pub fn save(&self) -> Result<(), Error> { ... }
}

pub struct UnlockedVault {
    inner: HashMap<String, String>,
    _lock: VaultLock,  // Zeroizes on drop
}
```

### Phase 4: Integration with Upgrade Flow

```rust
// In app.rs:confirm_upgrade()
let vault = Vault::open(config_dir.join("vault.pqenc"))?;
let unlocked = vault.unlock().await?;  // Biometric prompt here
self.vault_passwords = Some(unlocked);

// In tasks.rs:spawn_upgrade()
let pass = self.vault_passwords
    .as_ref()
    .and_then(|v| v.get_password(&server.key()))
    .or_else(|| load_sso())  // Fallback
    .or_else(|| load_per_server_override());
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| `pqcrypto-kyber` | ML-KEM-768 (KEM) |
| `pqcrypto-dilithium` | ML-DSA-65 (signatures) |
| `aes-gcm` | AEAD encryption |
| `hkdf` | Key derivation |
| `sha3` | SHA3-256 for HKDF |
| `security-framework` | macOS Secure Enclave |
| `zbus` / `fprintd` | Linux fingerprint |
| `tss-esapi` | Linux TPM2 (optional) |
| `argon2` | Fallback KDF |
| `zeroize` | Secure memory clearing |

## Security Properties

| Property | Achieved By |
|----------|-------------|
| **Post-quantum confidentiality** | ML-KEM-768 (128-bit quantum security) |
| **Post-quantum authenticity** | ML-DSA-65 signatures |
| **Forward secrecy** | Ephemeral KEM per vault creation |
| **Biometric binding** | Secure Enclave (macOS) / TPM2 (Linux) |
| **Offline attack resistance** | Argon2id fallback + high iteration count |
| **Key extraction resistance** | Keys never leave Secure Enclave/TPM |
| **Tamper evidence** | ML-DSA signature on entire vault |

## UX Flow

```
First run (no vault):
    Press 'u' → "No vault found. Create one?"
    → Enter master passphrase (or skip for biometric-only)
    → Touch ID / fingerprint to bind
    → Vault created

Subsequent runs:
    Press 'u' → "Authenticate to unlock vault" (Touch ID/fingerprint)
    → Success → all upgrades use per-server passwords from vault
    → Failure (3 attempts) → fallback to manual password entry
```

## Fallback Strategy

| Scenario | Fallback |
|----------|----------|
| No biometric hardware | Master passphrase (Argon2id) |
| Biometric fails (wet fingers, mask) | Passphrase |
| TPM/Secure Enclave unavailable | File-based key + Argon2id |
| Vault corrupted | Recreate from per-server Keychain items |
| Migration from old SSO | Import existing passwords into vault |

## Timeline Estimate

| Phase | Effort |
|-------|--------|
| Core crypto (Kyber + Dilithium + AES-GCM) | 1 week |
| macOS Touch ID + Secure Enclave | 3-5 days |
| Linux fprintd + TPM2/fallback | 1 week |
| Vault file format + API | 3 days |
| Integration with upgrade flow | 2 days |
| Testing (cross-platform, edge cases) | 1 week |
| **Total** | **~4-5 weeks** |

## Open Questions

1. **Hybrid KEM?** X25519 + ML-KEM-768 for defense in depth (adds ~32 bytes)
2. **Key rotation?** Re-encrypt vault with new key (requires biometric re-auth)
3. **Backup/restore?** Export encrypted vault (can only decrypt on same device with biometric)
4. **Sync across machines?** Each machine has own vault; passwords entered once per machine
5. **Audit?** Consider formal verification of crypto implementation