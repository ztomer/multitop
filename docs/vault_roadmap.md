# Vault Implementation Roadmap

## Status: COMPLETE

All phases implemented. See git history and test suite for implementation details.

## Remaining Work (Low Priority)

| Item | Status |
|------|--------|
| TPM2 Support | Deferred — optional, waiting for fprintd stability |
| Post-quantum KEM | Not needed for local file threat model |
| [R] Rotate Key button in Settings UI | Not implemented |

## Acceptance Criteria

| Phase | Must Work |
|-------|-----------|
| 1 | `vault.initialize("pass")` → `vault.unlock_password("pass")` → get/set passwords → save → reload |
| 2 | On Mac with Touch ID: `unlock_biometric(true)` → Touch ID prompt → success without password |
| 3 | On Linux with fingerprint reader: `unlock_biometric(true)` → fingerprint prompt → success |
| 4 | Wrong password 5x → 32s lockout; vault file replaced with old copy → rejected |
| 5 | 'p' → Vault tab shows status; 'u' uses vault passwords automatically |
| 6 | All tests pass; CI with coverage gate |

## Security Status

**Resolved:** Rate limiting, rollback protection, HKDF key separation, Secure Enclave, fprintd, migration framework, re-bind race condition, secure deletion, mlock.

## Test Coverage

112 vault tests across all modules. ~95% line coverage.

## Notes

- Vault is device-local — no sync, no cloud.
- Biometric (Secure Enclave/fprintd) is fully implemented.
- Multitop integration: upgrade flow and settings UI are complete.
- Migration framework handles v1→v2 upgrades.
