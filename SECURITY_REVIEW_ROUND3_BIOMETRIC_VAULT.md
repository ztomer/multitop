# Security Review Round 3: Final Pragmatic Biometric Vault Plan

**Reviewer**: Security Expert Persona
**Date**: 2026-07-30
**Verdict**: **PASS WITH CONDITIONS** — Strong design, 3 P2 items and 2 P3 items to address during implementation

---

## Executive Summary

The final plan is **cryptographically sound** and addresses all critical and high-severity findings from previous rounds. The independent key wrapping architecture correctly eliminates downgrade attacks. The plan is ready for implementation with minor conditions.

---

## P2 Issues (Address During Implementation)

### 1. HKDF-Derived Ed25519 Signing Key: Key Separation

**Issue**: Plan uses `signing_key = HKDF(vault_key, "signing")` for Ed25519 signatures. While this avoids separate key storage, it creates **key reuse** across two algorithms:
- `vault_key` → AES-256-GCM (encryption)
- `HKDF(vault_key, "signing")` → Ed25519 (signing)

**Risk**: Theoretical cross-protocol attacks if AES-GCM and Ed25519 interact unexpectedly. NIST SP 800-108 recommends distinct keys for distinct algorithms.

**Fix**: Use separate HKDF labels for each purpose:
```rust
let vault_enc_key = HKDF(vault_key, "enc");
let signing_key   = HKDF(vault_key, "sig");
let auth_key      = HKDF(vault_key, "auth");  // if adding HMAC later
```
And: `vault_key` itself should be `HKDF(master_secret, "vault")` where `master_secret` is the output of the unwrap operation (SE unwrap / Argon2id / TPM2).

### 2. Argon2id Auto-Detection Heuristic Needs Threat Model

**Issue**: Auto-detection based on "device class" (desktop/laptop/CI/embedded) is heuristic and may select weak parameters on:
- VMs with memory limits (e.g., 1GB RAM but 256MiB Argon2id works)
- CI runners with burstable memory
- Older hardware where 256MiB causes swap (defeating memory-hardness)

**Fix**: 
- **Measure available memory at runtime** (`sysinfo` crate) not device class
- **Cap at 50% of available RAM** (or 256MiB, whichever is lower)
- **Minimum floor**: 64 MiB (below this, memory-hardness is ineffective)
- **Log selected params** for user visibility: "Vault: using Argon2id(t=8, m=128MiB, p=4) — estimated 2.3s"

```rust
fn auto_argon2_params() -> (u32, u32, u32) {
    let total_mem = sysinfo::System::new_all().total_memory(); // bytes
    let available = total_mem.min(1_073_741_824); // cap at 1GB for calculation
    let m_kib = (available / 2 / 1024).clamp(65536, 262144) as u32; // 64-256 MiB
    let t = match m_kib {
        65536..=98303 => 4,
        98304..=163839 => 6,
        163840..=229375 => 8,
        _ => 10,
    };
    (t, m_kib, 4)
}
```

### 3. Secure Enclave Key Handle Persistence Across Reboots

**Issue**: Plan says "store SE key handle in vault header". But `SecKey` persistent references (`SecKeyCreatePersistentReference`) can become invalid after:
- macOS major version upgrade
- Secure Enclave firmware update
- User disabling/re-enabling Touch ID in System Settings

**Current handling**: Plan detects `errSecInvalidKey` on unwrap and falls back. But this means:
- First unlock after macOS upgrade fails biometric → password fallback
- Re-bind creates NEW SE key → old wrapper orphaned in header (harmless but accumulates)

**Fix**: 
- Store **key creation timestamp** in wrapper
- On unwrap failure, check if SE key is > 1 year old → proactively re-bind
- Limit wrapper count per type to 1 (replace on re-bind, don't accumulate)

---

## P3 Issues (Nice to Have)

### 4. No Vault Compromise Detection (Canary)

**Issue**: If attacker copies vault file and later obtains vault_key (memory dump, cold boot), they can decrypt all passwords. No detection mechanism.

**Fix**: Add **canary entry** to vault JSON:
```json
{
  "canary": "multitop-vault-canary-<random-uuid>",
  "server-a.example.com": "password1",
  ...
}
```
On unlock, verify canary matches expected value. If attacker modifies vault (adds/removes entries), canary mismatch → alert user.

**Better**: Store canary hash in Keychain/TPM (separate from vault). On unlock, verify vault canary matches stored hash. Detects vault file replacement.

### 5. No Secure Deletion of Old Vault Versions on Rotation

**Issue**: Key rotation creates new vault file via atomic rename. Old vault file is replaced but **not securely deleted** — data may remain on SSD (wear leveling) or in filesystem journal.

**Fix**: 
- On rotation: `shred` old file before rename (Linux: `shred -n 3`, macOS: `srm` or `diskutil secureErase`)
- Or: Accept that modern SSDs with TRIM + FileVault/APFS encryption make this moot
- Document: "Key rotation replaces vault file; old data may persist on disk until overwritten"

---

## Implementation Validation Checklist

Before merging implementation, verify:

| Test | Expected Result |
|------|-----------------|
| `test_hkdf_key_separation` | Encryption key ≠ Signing key ≠ Auth key |
| `test_argon2_auto_detect` | Params scale with available RAM; min 64MiB |
| `test_se_key_invalidation_rebind` | After simulated SE error → password unlock → new SE wrapper works |
| `test_vault_canary_verification` | Tampered vault (added entry) → unlock fails with canary mismatch |
| `test_rotation_secure_delete` | Old vault file overwritten (or documented as not required) |
| `test_concurrent_unlock_flock` | Two processes unlock same vault → no corruption, no lost re-binds |
| `test_migration_v2_v3` | Add TPM2 wrapper type → v2 vault migrates to v3 |

---

## Architecture Decision Log (For Future Reviewers)

| Decision | Rationale | Alternative Considered |
|----------|-----------|------------------------|
| Independent wrapped keys (not shared derivation) | Eliminates downgrade attack | Shared HKDF — rejected (Round 1) |
| Argon2id always present | Guarantees fallback on all platforms | Platform-specific only — rejected (Round 1) |
| HKDF-derived signing key | No separate key storage | Static embedded key — rejected (Round 2) |
| Auto-detect Argon2 params | Usable on constrained devices | Fixed params — rejected (Round 2) |
| Rate limiting persisted in header | Survives process restart | In-memory only — rejected (Round 2) |
| SE key invalidation handling | macOS updates break SE keys | Ignore — rejected (Round 2) |
| TPM2 deferred | Complex, distro-dependent | Include in MVP — rejected (Round 2) |
| Ed25519 sig derived from vault_key | Simpler than separate key mgmt | Separate Keychain key — rejected (Round 3) |

---

## Remaining Threat Model Gaps (Accepted)

| Gap | Why Accepted |
|-----|--------------|
| Malicious multitop binary | Cannot defend against compromised application binary |
| Evil maid (hardware keylogger) | Out of scope for software vault |
| Cold boot attack on vault_key in RAM | `mlock` + zeroize mitigates; full mitigation requires TPM-sealed RAM |
| Vault file backup/restore by user | User responsibility; document "vault is device-local" |
| Quantum attack on AES-256-GCM | 256-bit symmetric = 128-bit quantum (Grover) — sufficient |

---

## Final Sign-Off

**This plan PASSES security review with conditions:**

1. **MUST** implement HKDF key separation (P2-1)
2. **MUST** use runtime memory detection for Argon2id (P2-2)  
3. **MUST** handle SE key invalidation gracefully (P2-3)
4. **SHOULD** add canary for tamper detection (P3-4)
5. **SHOULD** document secure deletion on rotation (P3-5)

**No blocking issues.** Ready for implementation.