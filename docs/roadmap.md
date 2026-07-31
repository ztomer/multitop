# multitop Roadmap & Feature Specifications

## Upcoming Features

### A. Regression Tests
- Upgrade Flow Regression Tests
- SSO Credential Storage Tests
- State Persistence Tests (`state.toml`)
- Sparkline Rendering Tests
- Header Username Consistency Tests

### B. Update Confirmation Modal & Last Update Persistence
- `u`/`U` opens confirmation modal with timestamp
- `state.toml` persists last update timestamp
- Power-loss detection (incomplete upgrade warning)

### C. Single Sign-On (SSO) & Biometric Unlock
- Master password in system keychain (`__sso_master__`)
- Touch ID / fingerprint unlock (Secure Enclave on macOS, fprintd on Linux)
- Per-server password override support

### D. Sparkline Integration
- Memory (% ) and CPU (%) sparklines per server in border/header
- `M: ▂▃▅  user@server-01  C: ▃▅▇█`

### E. Username Display Consistency
- Standardize `user@host` headers across all views (Monitor, Docker, Fetch, Upgrade)

### F. UX & SSO Performance Refinements
- Keybar mode highlights
- Top border username & hostname centering
- Single-prompt SSO credential resolution
