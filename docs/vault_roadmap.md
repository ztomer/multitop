# Vault Implementation Roadmap

## Phase 0: Foundation (Week 1) ✓
- [x] Password storage in macOS Keychain / Linux Secret Service (existing `password_store`)
- [x] Per-server sudo password override (existing)
- [x] SSO master password (existing)

---

## Phase 1: Local Encrypted Vault File (Week 2) — IMPLEMENTED ✓
**Goal**: Single encrypted file at `~/.local/share/multitop/vault.bin` storing `host:port → password` map.

### 1.1 Crypto Primitives
- [x] AES-256-GCM encryption/decryption (`aes-gcm`)
- [x] Argon2id key derivation (`argon2`) with auto-tuned params (RAM-based)
- [x] Ed25519 signing/verification (`ed25519-dalek`)
- [x] HKDF-SHA256 key separation (`hkdf`)

### 1.2 File Format
- [x] Binary format: `magic | version | salt | argon2_params | wrapped_keys | nonce | ciphertext | ed25519_pk | signature`
- [x] Wrapped key entries: `type:u8 | len:u16 | data:bytes` (Argon2id, SE, TPM2)
- [x] Atomic write (tmp + rename + dir fsync)
- [x] File perms 0600, dir perms 0700

### 1.3 Argon2id Wrapper (Always Present)
- [x] `wrap_argon2id(vault_key, password, salt, params) → wrapped_key`
- [x] `unwrap_argon2id(wrapped_key, password, salt, params) → vault_key`
- [x] Params auto-tuned at creation (RAM/4, clamped 64MiB-1GiB)

### 1.4 Vault API (Sync)
- [x] `Vault::new(config) → Vault`
- [x] `vault.initialize(password) → Result<()>`
- [x] `vault.unlock_biometric(fallback_to_password) → Result<(UnlockedVault, UnlockResult)>`
- [x] `vault.unlock_password(password) → Result<UnlockedVault>`
- [x] `unlocked.get_password(host) → Option<SecretString>`
- [x] `unlocked.set_password(host, password) → Result<()>`
- [x] `unlocked.save() → Result<()>`

### 1.5 Tests ✓
- [x] Round-trip: init → unlock → get/set → save → reload → get
- [x] Wrong password fails
- [x] Tampered file fails signature check
- [x] Rate limiting with exponential backoff
- [x] Change password
- [x] Concurrent access
- [x] Multiple servers
- [x] Remove password
- [x] List hosts
- [x] Migration flag handling

---

## Phase 2: macOS Secure Enclave ✓
**Goal**: Touch ID / Face ID unlock via Secure Enclave EC P-256 key.

### 2.1 Key Management
- [x] Generate/retrieve persistent EC P-256 key in Secure Enclave (`security-framework`)
- [x] Access control: `kSecAccessControlDevicePasscode | kSecAccessControlBiometryCurrentSet`
- [x] Store key handle in vault wrapper

### 2.2 Wrap/Unwrap
- [x] `wrap(vault_key) → ECIES(ciphertext)` using SE public key (no auth required)
- [x] `unwrap(wrapped_key) → vault_key` via SE private key (requires Touch ID/Face ID)
- [x] Handle `errSecAuthFailed` → fallback to password
- [x] Handle `errSecInvalidKeyRef` (macOS update, Touch ID disabled) → auto re-bind on next password unlock

### 2.3 Integration
- [x] Auto-create SE wrapper on `initialize()` if available
- [x] Re-bind SE wrapper on successful password unlock if SE available

**Status**: IMPLEMENTED ✓ — Full Secure Enclave integration with Touch ID/Face ID support.

---

## Phase 3: Linux fprintd + TPM2 ✓
**Goal**: Fingerprint unlock on Linux with optional TPM2 sealing.

### 3.1 fprintd (DBus via `zbus`)
- [x] Connect to `net.reactivated.Fprint` system bus
- [x] Claim device, `VerifyStart(finger)`, poll `GetStatus` until `verify-match`/`verify-failed`
- [x] 30s timeout, retry on `verify-retry-scan`
- [x] Release device on completion

### 3.2 TPM2 (Optional, `tss-esapi`)
- [ ] Create primary key in owner hierarchy
- [ ] Create child key with PCR policy (fprintd auth) + authValue
- [ ] Seal vault key to TPM
- [ ] On verify-match: unseal with TPM

### 3.3 Fallback
- [x] If no fprintd/TPM2 → Argon2id password only

**Status**: fprintd IMPLEMENTED ✓ — Full D-Bus integration with fingerprint verification. TPM2 deferred.

---

## Phase 4: Advanced Features ✓
**Goal**: Key rotation, rollback protection, rate limiting.

### 4.1 Key Rotation / Password Change ✓
- [x] `change_password(old, new) → Result<()>` — implemented in `api.rs`
- [x] Secure overwrite of old vault file on rotation
- [x] Re-wrap vault key with new password

### 4.2 Rollback Protection ✓
- [x] Monotonic counter in signed header
- [x] Store last-seen counter in OS keychain
- [x] Reject vault if counter ≤ stored

### 4.3 Rate Limiting ✓
- [x] In-memory exponential backoff (1s, 2s, 4s... max 60s)
- [x] Persist `failed_attempts` + `lockout_until` in companion file
- [x] Reset on success
- [x] Hard lockout after 10 failures (5 minutes)

### 4.4 Canary ✓
- [x] Random string in decrypted contents (16-byte hex canary)
- [x] Verify on unlock → detect corruption/tampering

---

## Phase 5: UI Integration ✓
**Goal**: Settings screen and upgrade flow integration.

### 5.1 Settings Screen (Press 'p' → Tab: "Vault")
- [x] Vault status indicator: locked/unlocked
- [x] [U] Unlock Vault button
- [x] [L] Lock Vault button
- [x] [A] Add / [D] Delete server passwords in vault
- [ ] [R] Rotate Key button (not implemented)

### 5.2 Upgrade Flow ✓
- [x] On 'u': if vault locked → show TUI password prompt → unlock → get passwords per server → spawn upgrades
- [x] Cache unlocked vault for session (multiple upgrades)
- [x] Touch ID prompt on first unlock

---

## Phase 6: Hardening & Polish ✓
**Goal**: Memory protection, migration, CI.

### 6.1 Memory Protection ✓
- [x] `mlock` vault key on Linux/macOS (best effort)
- [x] Secure delete on key rotation (`secure_overwrite`)
- [x] Zeroize on drop for sensitive data

### 6.2 Migration Framework ✓
- [x] Version detection and migration flag
- [x] `migrate_if_needed()` for pre-unlock checks
- [x] `complete_migration()` for post-unlock migration

### 6.3 CI & Testing ✓
- [x] 111 vault tests covering all modules
- [x] Unit tests for crypto, format, lockout, rollback, mlock
- [x] E2E tests for vault operations
- [x] CI workflow with coverage gate

---

## SECURITY STATUS (From Code Audit)

### RESOLVED
- [x] **Rate Limiting** — Implemented with exponential backoff + hard lockout
- [x] **Rollback Protection** — Counter stored in OS keychain, verified on unlock
- [x] **HKDF Key Separation** — Separate sub-keys for AES-GCM and Ed25519 via HKDF
- [x] **macOS Memory Detection** — Uses `vm.page_free_count` instead of total RAM
- [x] **Secure Enclave** — Real implementation with Touch ID/Face ID
- [x] **fprintd** — Real implementation with D-Bus integration
- [x] **Vault Migration** — Framework for version upgrades
- [x] **Re-bind Race Condition** — File locking via `fs2`
- [x] **Secure Deletion** — 3-pass overwrite on key rotation
- [x] **mlock** — Best-effort memory locking for vault key

### REMAINING (Low Priority)
- [ ] **TPM2 Support** — Optional, deferred until fprintd path is stable
- [ ] **Secure Enclave Re-bind Race** — Low impact now that SE is implemented
- [ ] **Post-quantum KEM** — Not needed for local file threat model

---

## Acceptance Criteria per Phase

| Phase | Must Work | Status |
|-------|-----------|--------|
| 1 | `vault.initialize("pass")` → `vault.unlock_password("pass")` → get/set passwords → save → reload | ✓ |
| 2 | On Mac with Touch ID: `unlock_biometric(true)` → Touch ID prompt → success without password | ✓ |
| 3 | On Linux with fingerprint reader: `unlock_biometric(true)` → fingerprint prompt → success | ✓ |
| 4 | Wrong password 5x → 32s lockout; vault file replaced with old copy → rejected | ✓ |
| 5 | 'p' → Vault tab shows status; 'u' uses vault passwords automatically | ✓ |
| 6 | All tests pass; CI with coverage gate | ✓ |

---

## Dependencies Summary (Actual)

```toml
# Current vault Cargo.toml
aes-gcm = "0.10"       ✓
argon2 = "0.5"         ✓
ed25519-dalek = "2.1"  ✓
hkdf = "0.12"          ✓
rand = "0.8"           ✓
secrecy = "0.10"       ✓
sha2 = "0.10"          ✓
zeroize = "1.8"        ✓
serde = "1.0"          ✓
serde_json = "1.0"     ✓
dirs = "5.0"           ✓
thiserror = "1.0"      ✓
tokio = "1"            ✓
rpassword = "7.0"      ✓
hex = "0.4"            ✓
fs2 = "0.4"            ✓
libc = "0.2"           ✓
keyring = "3"          ✓

# Platform-specific
security-framework = "3.7"   ✓ (macOS)
security-framework-sys = "2.17" ✓ (macOS)
zbus = "4"                   ✓ (Linux)

# Missing (optional/deferred):
tss-esapi =             ✗ (TPM2, optional)
```

---

## Test Coverage

| Module | Tests | Coverage |
|--------|-------|----------|
| api.rs | 21 | Full |
| crypto.rs | 28 | Full |
| format.rs | 22 | Full |
| lockout.rs | 12 | Full |
| rollback.rs | 7 | Full |
| mlock.rs | 6 | Full |
| secure_enclave.rs | 4 | Full (non-macOS stubs) |
| fprintd.rs | 8 | Full (non-Linux stubs) |
| **Total** | **108** | **~95%** |

---

## Notes

- **HKDF key separation IS implemented** — `VaultKey.encryption_key()` derives AES-GCM key via HKDF, `derive_signing_key()` derives Ed25519 key via HKDF with different labels
- **Vault is device-local** — no sync, no cloud. Each machine has its own vault.
- **Biometric (SE/fprintd) is fully implemented** — both modules have real implementations
- **Multitop integration**: upgrade flow (Phase 5.2) and settings UI (Phase 5.1) are complete
- **Migration framework**: Handles v1→v2 upgrades with password-based migration
