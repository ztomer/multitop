//! History spill to `~/.cache/multitop/history/<host>.zst` — `1h/1d` via `zstd`.
//!
//! `History` is currently RAM-only: a rebuilt panel starts with an empty
//! `G` view. Spilling the ring to disk lets `G` draw `1d` not `200` ticks
//! without new agent fields — the `Monitor` packets already feed `History`.

use crate::history::History;
use std::path::PathBuf;

fn history_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".cache/multitop/history"))
}

fn path_for(host: &str) -> Option<PathBuf> {
    let dir = history_dir()?;
    // Sanitize `user@host:port` into a filename.
    let safe: String = host
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    Some(dir.join(format!("{safe}.zst")))
}

/// Load history for `host` if it exists on disk.
#[must_use]
pub fn load(host: &str) -> Option<History> {
    let path = path_for(host)?;
    let bytes = std::fs::read(path).ok()?;
    // zstd or plain JSON for backwards compat.
    let decoded = if bytes.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        // zstd magic
        zstd::decode_all(&bytes[..]).ok()?
    } else {
        bytes
    };
    serde_json::from_slice(&decoded).ok()
}

/// Save `history` for `host` (best-effort, no error surface).
pub fn save(host: &str, history: &History) {
    let Some(path) = path_for(host) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Keep only the last `SAMPLES` on disk as well.
    let Ok(json) = serde_json::to_vec(history) else {
        return;
    };
    // zstd level 3 is the default in `zstd` crate when no level given — small
    // and fast, good for a few KiB per host.
    let Ok(compressed) = zstd::encode_all(json.as_slice(), 3) else {
        return;
    };
    let tmp = path.with_extension(format!("zst.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, compressed).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
        let _ = std::fs::remove_file(&tmp);
    }
}
