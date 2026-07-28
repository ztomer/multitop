//! Embed the prebuilt Linux agent binaries.
//!
//! Paths come from `MULTITOP_AGENT_X86_64` / `MULTITOP_AGENT_AARCH64`, which
//! `build.sh` sets after cross-compiling. When a variable is unset the
//! corresponding slot is `None` and the binary still builds — you just cannot
//! monitor a host of that architecture, and the panel says so.

use std::path::Path;

/// FNV-1a. The hash keys the remote cache filename; it needs to change when
/// the bytes change, nothing more.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn main() {
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let mut src = String::new();

    for (var, ident) in [
        ("MULTITOP_AGENT_X86_64", "X86_64"),
        ("MULTITOP_AGENT_AARCH64", "AARCH64"),
    ] {
        println!("cargo:rerun-if-env-changed={var}");
        let path = std::env::var(var).unwrap_or_default();
        if !path.is_empty() && Path::new(&path).is_file() {
            println!("cargo:rerun-if-changed={path}");
            let bytes = std::fs::read(&path).expect("read agent binary");
            src.push_str(&format!(
                "pub static AGENT_{ident}: Option<&[u8]> = Some(include_bytes!(r\"{path}\"));\n\
                 pub static HASH_{ident}: &str = \"{:016x}\";\n",
                fnv1a(&bytes)
            ));
        } else {
            src.push_str(&format!(
                "pub static AGENT_{ident}: Option<&[u8]> = None;\n\
                 pub static HASH_{ident}: &str = \"missing\";\n"
            ));
        }
    }

    std::fs::write(Path::new(&out_dir).join("agents.rs"), src).expect("write agents.rs");
}
