# Vault Security Gaps — Fix Plan

Dependency order: items that change the crypto format (HKDF separation,
migration framework) must precede items that depend on the new format.
Items with no cross-dependency can run in parallel.

---

## Phase A: HKDF Key Separation (P1) — Phase 1

**Why first**: Foundational crypto change. Every encrypt/decrypt/sign path
uses `VaultKey` directly — fixing this means touching `encrypt_vault`,
`decrypt_vault`, `sign_vault`, and `verify_vault_signature`. All subsequent
phases that touch the vault file benefit from correct key derivation.

**Approach** (A1 additive before subtractive):
1. Add `VaultKey::encryption_key(&self) -> [u8; 32]` via
   `HKDF-Expand(vault_key, b"vault-aes-gcm-key")`
2. Add `VaultKey::signing_key(&self) -> SigningKey` replacing the existing
   `derive_signing_key()` — use label `b"vault-ed25519-sig-key"` instead
   of `b"multitop-vault-signing"` (old label was fine but unprincipled)
3. Modify `encrypt_vault` / `decrypt_vault` to call `key.encryption_key()`
   instead of `key.as_bytes()`
4. Modify `sign_vault` to call `key.signing_key()` instead of
   `key.derive_signing_key()`
5. Derive the Ed25519 verifying key for the header from the sig sub-key,
   not vault_key directly
6. `verify_vault_signature` — unchanged (already takes a public key)

**Format change**: No. The vault file stores the Ed25519 public key in
the header. That PK is now derived from `sig_key` instead of directly from
`vault_key`, but the stored bytes are the same size (32 bytes). Old vaults
still verify because the old `derive_signing_key()` and new `signing_key()`
both produce valid Ed25519 keypairs from the same vault_key — as long as the
HKDF label changes, old signatures won't verify under the new derivation.
**Requires migration (Phase D)** to re-sign the vault.

**Kill criterion**: `test_hkdf_key_separation`: encryption key ≠ signing
key (byte comparison). All existing tests pass.

**Estimate**: 1 day.

---

## Phase B: Rate Limiting (P0) — Phase 1 (parallel with A)

**Why P0**: No lockout means unlimited online brute force.

**Approach**:
1. Add `failed_attempts: AtomicU32` to `Vault` struct (in-memory, survives
   process lifetime)
2. In `unlock_with_password`: check `failed_attempts`, if ≥ 3 apply
   exponential backoff: `min(2^(n-3), 60)` seconds delay via `std::thread::sleep`
3. On success: reset counter. On failure: increment.
4. At 10 failures: 5-minute hard lockout (`lockout_until` as Instant),
   enforced before even checking password
5. Add `failed_attempts: u32` and `lockout_until_epoch_ms: u64` to
   `VaultHeader` to persist across process restarts
6. Persist on every failed attempt (save header), persist reset on success

**No format change** required — new header fields are additive; old code
deserializes missing fields as 0 (no lockout).

**Kill criterion**: 3 quick wrong passwords → 4th attempt blocks 2s; 10
wrong → 5m lockout. Header persists after process restart.

**Estimate**: 1.5 days.

---

## Phase C: Rollback Protection (P0) — Phase 1 (parallel with A/B)

**Why P0**: Counter exists but unverifiable — vault can be silently replaced.

**Approach**:
1. Add `store_last_counter(path: &Path, counter: u32, created_ts: u64)` to
   `password_store` or a new store module using `keyring` crate
2. Keychain entry: service = `"multitop-vault-rollback"`, account = SHA-256
   hash of vault path
3. In `UnlockedVault::save()`: after writing, update the stored counter
4. In `unlock_with_password`: after decrypt but before returning, verify
   `header.counter > stored_counter` (and same `created_ts`)
5. On first-ever unlock (no Keychain entry), silently store current counter
6. On rollback detected: return `Err(VaultError::RollbackDetected)` with
   message showing old and new counter values

**No format change** — all state lives in Keychain.

**Kill criterion**: Save vault, copy file, modify password, replace with
old file → unlock fails with rollback error.

**Estimate**: 1 day.

---

## Phase D: Migration Framework (P1) — Phase 2 (after A)

**Why after A**: Once the Ed25519 derivation changes (Phase A), old vaults
won't verify under the new derivation key. Need migration to re-sign.

**Approach** (A1 additive):
1. Implement `migrate(vault_path, current_version) -> Result<()>` in
   `format.rs` or `api.rs`
2. Version table:
   - v1 (current): stores Ed25519 PK derived from vault_key w/o key
     separation
   - v2 (post-Phase A): stores Ed25519 PK derived from HKDF sig sub-key
3. Migration v1→v2: read vault, verify with old derivation, re-sign header
   with new derivation, write back
4. The `Vault` struct calls `migrate_if_needed()` in `new()` or `initialize()`
5. Test: v1 vault file → `migrate()` → v2 vault file → unlock succeeds

**Format change**: Header `version` field incremented from 1 to 2.

**Kill criterion**: Old vault file (golden test data) → unlock with new
code → succeeds (migration transparent). New vaults created at v2.

**Estimate**: 1 day.

---

## Phase E: macOS Memory Detection (P1) — Phase 1 (parallel)

**Why P1**: Auto-tuning picks Argon2id params based on total RAM instead of
available memory; can cause OOM on memory-constrained systems.

**Approach**:
1. Replace `get_available_memory_kib()` on macOS — instead of `hw.memsize`
   (total), use `vm_page_free_count * page_size` via `sysctl vm.page_free_count`
2. Fallback: if unavailable, use `hw.memsize` × 0.5 (conservative)
3. Linux path already reads `MemAvailable` from `/proc/meminfo` — correct
4. Other platforms: use a conservative 64 MiB minimum

**No format change**.

**Kill criterion**: On macOS, `vm_page_free_count` read succeeds and
returns ≤ total RAM. Argon2id params drop under memory pressure.

**Estimate**: 0.5 day.

---

## Phase F: Secure Enclave Implementation (P1) — Phase 2

**Why P1**: Entire biometric unlock path is a stub; Touch ID doesn't work.

**Approach** (A4 foundation before cutover):
1. Foundation: Implement `secure_enclave.rs` using `security-framework`:
   - `get_or_create()`: Generate EC P-256 key in SE via
     `SecKeyGeneratePair` with access control
     `kSecAccessControlBiometryCurrentSet` (Touch ID required)
   - `wrap_key(vault_key)`: Use SE public key to encrypt vault_key via
     `SecKeyCreateEncryptedData`
   - `unwrap_key(wrapper)`: Use SE private key to decrypt via
     `SecKeyCreateDecryptedData` — Touch ID prompt auto-triggered by OS
   - Store persistent key reference (CFData/NSData) in wrapper data field
   - Handle `errSecAuthFailed` → `Err(VaultError::BiometricFailed)`
   - Handle `errSecInvalidKeyRef` → `Err(VaultError::SecureEnclaveError)` —
     triggers cleanup + password fallback in unlock flow
2. Cutover: In `api.rs:initialize()`, if `get_secure_enclave()` succeeds,
   auto-create SE wrapper alongside Argon2id wrapper
3. In `api.rs:try_unlock_biometric()`, replace stub call with real
   `get_secure_enclave()?.unwrap_key()`
4. Opportunistic re-bind in password unlock: after password unlock, if SE
   available but wrapper missing/failed, create new SE wrapper and save
   (with file locking from Phase G)
5. Guard: `#[cfg(target_os = "macos")]` gating already in place

**Kill criterion**: On Mac with Touch ID: `unlock_biometric(false)` → Touch
ID prompt → `Ok(UnlockedVault)`. `unlock_with_password("wrong")` → retry
with correct password → SE wrapper re-created.

**Estimate**: 4 days.

---

## Phase G: fprintd Implementation (P1) — Phase 2 (parallel with F)

**Why P1**: Linux fingerprint path is a stub.

**Approach** (A4):
1. Implement `fprintd.rs` using `zbus`:
   - `FingerprintVerifier::new()`: Connect to `net.reactivated.Fprint`
     on system bus, get first device
   - `verify(timeout_sec)`: Call `VerifyStart` on claimed device, poll
     `GetStatus` in loop until `verify-match` or `verify-failed` or timeout
   - Map `verify-no-match` → retry (up to 3 times), `verify-finger-not-set`
     → `NotEnrolled`
   - Stop on `verify-match` (success) or timeout
2. In `api.rs:try_unlock_biometric()`: on Linux, call fprintd verify, if
   success → try each available wrapping method (only Argon2id until TPM2
   is implemented)
3. fprintd verify is a **presence check only** — the actual unlock still
   goes through the existing wrapping methods. fprintd just proves the user
   is physically present.

**Kill criterion**: On Linux with fingerprint reader and enrolled finger:
`unlock_biometric(false)` → finger scan → unlocks. Without reader: falls
through to password.

**Estimate**: 3 days.

---

## Phase H: Re-bind Race Condition (P2) — Phase 3 (after F)

**Why after F**: Race only matters once SE exists (re-bind writes happen).
Currently (SE stub), no re-bind ever triggers.

**Approach**:
1. Add `fs2` or `fslock` crate for advisory file locking
2. In `UnlockedVault::save()`: acquire exclusive `flock()` on vault file
   before writing, release after atomic rename
3. In `unlock_with_password()`: if re-binding SE wrapper, acquire lock
   before reading → writing, so two concurrent processes don't race

**No format change**.

**Kill criterion**: Two concurrent password unlocks with SE re-bind → both
succeed, vault file intact, both SE wrappers present.

**Estimate**: 0.5 day.

---

## Phase I: Secure Deletion (P2) — Phase 3 (after D)

**Why after D**: Writes vault file; migration framework (Phase D) also
writes vault files, so deletion logic should be consistent.

**Approach**:
1. Add `secure_overwrite(path: &Path, passes: u32)` helper:
   - Write random data over file length, then zeros, then random again
   - On Linux: optionally shell out to `shred -n 3`
   - On macOS: use `fwrite` + `fsync` overwrites (APFS encryption makes
     this best-effort)
   - Fallback: `std::fs::remove_file` if secure overwrite fails
2. Call `secure_overwrite()` on old vault file before `atomic_write_vault`
   rename in `change_password()`
3. Document: "Vault files on modern SSDs with encryption may retain data
   despite overwrite — enable FileVault/LUKS for full protection"

**No format change**.

**Kill criterion**: `secure_overwrite()` runs without error on test file,
file content is unrecoverable via `dd` (visual check on non-SSD).

**Estimate**: 0.5 day.

---

## Phase J: mlock Support (P3) — Phase 3 (parallel)

**Why P3**: Defense-in-depth for key material in RAM.

**Approach** (A1 additive):
1. In `UnlockedVault::new()` (or `decrypt_and_load`): call `mlock()` on
   the vault_key's backing memory
2. Cross-platform:
   - Unix: `libc::mlock(vault_key_ptr, 32)`
   - macOS: `mach_vm_wire` or `mlock` (same API on Darwin)
   - If `EPERM` (no CAP_IPC_LOCK on Linux): log warning, continue
   - If `ENOMEM` (rlimit too low): log warning, continue
3. In `VaultKey` / `UnlockedVault::drop()`: call `munlock()` before
   zeroize (order: munlock → zeroize → free)

**No format change**.

**Kill criterion**: On Linux with `CAP_IPC_LOCK`: vault key is mlocked
(`grep VmLock /proc/pid/status` shows non-zero). On macOS: works silently
(`mlock` is available). Without caps: logs warning, continues.

**Estimate**: 0.5 day.

---

## Deferred (Not in Scope — Phase 4+)

| Item | Rationale |
|------|-----------|
| TPM2 for Linux | Requires `tss-esapi`, distro-dependent, niche. fprintd + Argon2id covers 95% |
| Vault settings UI (Phase 5.1) | UX polish, not security. Already tracked in VAULT_ROADMAP.md |
| Fuzz testing | CI infrastructure, not security per se |
| Binary integrity verification | Out of application scope — build/packaging concern |

---

## Schedule

```
Week 1:  A (HKDF separation) + B (rate limiting) + C (rollback) + E (macOS mem)
         → all tests pass, golden vault file updated
Week 2:  D (migration framework) + F (SE implementation, macOS)
         → v1→v2 migration works, Touch ID unlock works on Mac
Week 3:  G (fprintd, Linux) + H (re-bind race) + I (secure deletion) + J (mlock)
         → fingerprint unlock on Linux, all gaps closed
```

## Total Estimate: ~12 days

| Phase | Days | Type |
|-------|------|------|
| A — HKDF separation | 1 | Crypto refactor |
| B — Rate limiting | 1.5 | New feature |
| C — Rollback | 1 | New feature |
| D — Migration | 1 | Infrastructure |
| E — macOS mem | 0.5 | Bugfix |
| F — Secure Enclave | 4 | New feature (macOS) |
| G — fprintd | 3 | New feature (Linux) |
| H — Re-bind race | 0.5 | Bugfix |
| I — Secure deletion | 0.5 | Defense-in-depth |
| J — mlock | 0.5 | Defense-in-depth |
