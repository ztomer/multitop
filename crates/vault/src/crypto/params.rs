//! Argon2id parameters: the bounds they must fall inside, and how they are
//! chosen for a machine.
//!
//! The bounds are load-bearing in both directions. Below the floor the vault
//! can be brute-forced; above the ceiling an unlock allocates more memory than
//! the machine has, and the vault cannot be opened at all.

use argon2::{Argon2, Params};

/// Memory tiers, in MiB, that decide how many Argon2id passes to make. More
/// memory means each pass costs an attacker more, so fewer passes buy the same
/// resistance; below the lower tier the passes have to make up for it.
const HIGH_MEMORY_MIB: u64 = 256;
const MID_MEMORY_MIB: u64 = 128;
/// Passes at each tier.
const PASSES_HIGH: u8 = 10;
const PASSES_MID: u8 = 8;
const PASSES_LOW: u8 = 6;
/// Lanes Argon2id runs in parallel. Four is the common core count floor; more
/// buys little once memory is the limiting factor.
const DEFAULT_PARALLELISM: u8 = 4;

/// Argon2id parameters
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Argon2Params {
    pub t: u8,      // iterations
    pub m_kib: u32, // memory in KiB
    pub p: u8,      // parallelism
}

/// Minimum Argon2 memory cost (32 MiB).
pub const MIN_M_KIB: u32 = 32_768;
/// Maximum Argon2 memory cost (4 GiB).
pub const MAX_M_KIB: u32 = 4_194_304;
/// Minimum Argon2 iterations.
pub const MIN_T: u8 = 1;
/// Maximum Argon2 iterations.
pub const MAX_T: u8 = 20;
/// Minimum Argon2 parallelism.
pub const MIN_P: u8 = 1;
/// Maximum Argon2 parallelism.
pub const MAX_P: u8 = 8;

impl Default for Argon2Params {
    fn default() -> Self {
        Self::auto_detect()
    }
}

impl Argon2Params {
    /// Auto-detect reasonable parameters for current hardware.
    ///
    /// # Panics
    /// Panics if system memory detection fails and falls back to defaults.
    #[must_use]
    pub fn auto_detect() -> Self {
        let mem_kib = get_available_memory_kib().unwrap_or(8_388_608); // 8 GiB default
        let target_mib = (mem_kib / 1024 / 4).clamp(64, 1024);
        let t = if target_mib >= HIGH_MEMORY_MIB {
            PASSES_HIGH
        } else if target_mib >= MID_MEMORY_MIB {
            PASSES_MID
        } else {
            PASSES_LOW
        };
        // Through `from_config`, which is the only constructor that clamps.
        // Building the struct here instead meant the clamps applied to a
        // constructor nothing called, while the values the vault actually used
        // went round them.
        #[allow(clippy::cast_possible_truncation)]
        Self::from_config(t, target_mib as u32, DEFAULT_PARALLELISM)
    }

    /// Create from config values.
    ///
    /// Values are clamped to valid ranges:
    /// - `t` (iterations): 1–20
    /// - `m_kib` (memory): 32 MiB – 4 GiB
    /// - `p` (parallelism): 1–8
    #[must_use]
    pub fn from_config(t: u8, m_mib: u32, p: u8) -> Self {
        // The MiB -> KiB conversion is done in u64. Doing it in u32 and
        // clamping the result inverted the clamp: `m_mib = 4_194_304` overflows
        // to 0, which then clamps *up* to the 32 MiB floor, so asking for the
        // documented maximum silently produced the weakest KDF the vault
        // allows. Release builds do not check overflow, so it was silent.
        let memory_kib = u64::from(m_mib)
            .saturating_mul(1024)
            .clamp(u64::from(MIN_M_KIB), u64::from(MAX_M_KIB));
        Self {
            t: t.clamp(MIN_T, MAX_T),
            // Clamped to MAX_M_KIB above, so this always fits.
            m_kib: u32::try_from(memory_kib).unwrap_or(MAX_M_KIB),
            p: p.clamp(MIN_P, MAX_P),
        }
    }

    /// Check that these parameters are within the supported ranges.
    ///
    /// # Errors
    /// Returns `VaultError::Argon2Params` if any value is out of range.
    pub fn validate(&self) -> Result<(), crate::VaultError> {
        if !(MIN_M_KIB..=MAX_M_KIB).contains(&self.m_kib) {
            return Err(crate::VaultError::Argon2Params(format!(
                "memory cost {} KiB is outside {MIN_M_KIB}..={MAX_M_KIB}",
                self.m_kib
            )));
        }
        if !(MIN_T..=MAX_T).contains(&self.t) {
            return Err(crate::VaultError::Argon2Params(format!(
                "iterations {} is outside {MIN_T}..={MAX_T}",
                self.t
            )));
        }
        if !(MIN_P..=MAX_P).contains(&self.p) {
            return Err(crate::VaultError::Argon2Params(format!(
                "parallelism {} is outside {MIN_P}..={MAX_P}",
                self.p
            )));
        }
        Ok(())
    }

    /// Create Argon2 instance.
    ///
    /// # Errors
    /// Returns `VaultError::Argon2Params` if the Argon2 parameters are invalid.
    pub fn to_argon2(&self) -> Result<argon2::Argon2<'static>, crate::VaultError> {
        // Every derivation -- wrap and unwrap -- funnels through here, which
        // makes this the one place that sees parameters read out of a vault
        // header. That header is parsed before any signature over it can be
        // checked (the key that would verify it is the one this KDF is about
        // to derive), so `m_kib` from a corrupt or hostile file would size a
        // multi-gigabyte allocation unvouched-for. `argon2::Params` accepts
        // anything up to u32::MAX KiB -- 4 TiB -- so it is not the backstop.
        self.validate()?;
        let params = Params::new(self.m_kib, u32::from(self.t), u32::from(self.p), None)
            .map_err(|e| crate::VaultError::Argon2Params(e.to_string()))?;
        Ok(Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            params,
        ))
    }
}

/// Get available system memory in KiB
fn get_available_memory_kib() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        use std::fs;
        let content = fs::read_to_string("/proc/meminfo").ok()?;
        for line in content.lines() {
            if line.starts_with("MemAvailable:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    return parts[1].parse().ok();
                }
            }
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        // Use vm_page_free_count to get actual available pages instead of total RAM
        let free_pages = Command::new("sysctl")
            .args(["-n", "vm.page_free_count"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok());

        let page_size = Command::new("sysctl")
            .args(["-n", "hw.pagesize"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok());

        if let (Some(pages), Some(psize)) = (free_pages, page_size) {
            Some(pages * psize / 1024)
        } else {
            // Fallback: use half of total RAM as conservative estimate
            let total = Command::new("sysctl")
                .args(["-n", "hw.memsize"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.trim().parse::<u64>().ok())?;
            Some(total / 1024 / 2)
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    None
}
