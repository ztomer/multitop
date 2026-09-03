//! Embed the prebuilt Linux agent binaries.
//!
//! Paths come from:
//! 1. `MULTITOP_AGENT_X86_64` / `MULTITOP_AGENT_AARCH64`, which `build.sh` sets
//!    after cross-compiling.
//! 2. Candidate target directories (shared `CARGO_TARGET_DIR`, workspace `target/`,
//!    `~/.cache/cargo-target`, `target/docker`, etc.) where cross-compiled binaries reside.
//! 3. Auto-compilation with `cargo zigbuild` if toolchains are available.
//!
//! When an agent binary is not found and cannot be built, the corresponding slot is `None`
//! and the binary still builds — you just cannot monitor a host of that architecture,
//! and the panel says so.

#![allow(clippy::expect_used)]

use std::fmt::Write;
use std::path::{Path, PathBuf};

/// FNV-1a. The hash keys the remote cache filename; it needs to change when
/// the bytes change, nothing more.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn try_auto_build(target_triple: &str, workspace_root: &Path) -> Option<PathBuf> {
    let out_target_dir = workspace_root.join("target").join("agent-build");
    let has_zigbuild = std::process::Command::new("cargo-zigbuild")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());

    if has_zigbuild {
        let status = std::process::Command::new("cargo")
            .args([
                "zigbuild",
                "-p",
                "multitop-agent",
                "--target",
                target_triple,
                "--release",
                "--target-dir",
            ])
            .arg(&out_target_dir)
            .current_dir(workspace_root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        if let Ok(s) = status {
            if s.success() {
                let bin = out_target_dir
                    .join(target_triple)
                    .join("release")
                    .join("multitop-agent");
                if bin.is_file() {
                    return Some(bin);
                }
            }
        }
    }
    None
}

fn find_agent_binary(
    var: &str,
    target_triple: &str,
    out_dir: &Path,
    manifest_dir: &Path,
) -> Option<PathBuf> {
    // 1. Check explicit environment variable
    let explicit = std::env::var(var).unwrap_or_default();
    if !explicit.is_empty() {
        let p = PathBuf::from(&explicit);
        if p.is_file() {
            return Some(p);
        }
    }

    // 2. Discover from candidate target roots
    let mut roots: Vec<PathBuf> = Vec::new();

    // Climb up from OUT_DIR (e.g. <target_dir>/[<triple>/]<profile>/build/<multitop-hash>/out)
    let mut curr = out_dir;
    for _ in 0..6 {
        if let Some(parent) = curr.parent() {
            roots.push(parent.to_path_buf());
            curr = parent;
        }
    }

    // CARGO_TARGET_DIR env var
    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        if !target_dir.is_empty() {
            roots.push(PathBuf::from(target_dir));
        }
    }

    let workspace_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or(manifest_dir);

    // Project target dirs
    roots.push(workspace_root.join("target"));
    roots.push(workspace_root.join("target").join("docker"));
    roots.push(workspace_root.join("target").join("agent-build"));

    // User-wide cache target
    if let Ok(home) = std::env::var("HOME") {
        roots.push(PathBuf::from(home).join(".cache").join("cargo-target"));
    }

    for root in &roots {
        for profile in ["release", "debug"] {
            let p = root
                .join(target_triple)
                .join(profile)
                .join("multitop-agent");
            if p.is_file() {
                return Some(p);
            }
        }
    }

    // 3. Try to auto-build if toolchains are present
    try_auto_build(target_triple, workspace_root)
}

fn main() {
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let out_path = PathBuf::from(&out_dir);
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let manifest_path = PathBuf::from(&manifest_dir);
    let workspace_root = manifest_path
        .parent()
        .and_then(Path::parent)
        .unwrap_or(&manifest_path);

    println!(
        "cargo:rerun-if-changed={}",
        workspace_root
            .join("crates")
            .join("agent")
            .join("src")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root
            .join("crates")
            .join("agent")
            .join("Cargo.toml")
            .display()
    );

    let mut src = String::new();

    // Workspace version that must be inside the agent binary.
    // `CARGO_PKG_VERSION` is the `multitop` crate's version, which is the
    // workspace version. An agent built from an older checkout still contains
    // the old version string, so embedding it would make every Hello mismatch
    // and loop forever uploading the same stale bytes.
    let ws_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let profile = std::env::var("PROFILE").unwrap_or_default();

    for (var, ident, triple) in [
        (
            "MULTITOP_AGENT_X86_64",
            "X86_64",
            "x86_64-unknown-linux-musl",
        ),
        (
            "MULTITOP_AGENT_AARCH64",
            "AARCH64",
            "aarch64-unknown-linux-musl",
        ),
    ] {
        println!("cargo:rerun-if-env-changed={var}");
        if let Some(path) = find_agent_binary(var, triple, &out_path, &manifest_path) {
            let path_str = path.display().to_string();
            println!("cargo:rerun-if-changed={path_str}");
            let bytes = std::fs::read(&path).expect("read agent binary");
            // Hard gate: a release binary must embed an agent built from this
            // same checkout. A stale agent (e.g. 0.44.0 bytes inside a 0.44.1
            // build) makes Hello `0.44.0 vs 0.44.1` and the `replace_agent`
            // loop uploads the same stale bytes forever.
            if !ws_version.is_empty()
                && !bytes
                    .windows(ws_version.len())
                    .any(|w| w == ws_version.as_bytes())
            {
                let msg = format!(
                    "agent {triple} version mismatch: workspace {ws_version} not found in binary at {} — rebuild with ./build.sh",
                    path.display()
                );
                if profile == "release" {
                    #[allow(clippy::panic)]
                    {
                        panic!("{msg}");
                    }
                } else {
                    println!("cargo:warning={msg}");
                }
            }
            write!(
                src,
                "pub static AGENT_{ident}: Option<&[u8]> = Some(include_bytes!(r\"{path_str}\"));\n\
                 pub static HASH_{ident}: &str = \"{:016x}\";\n\
                 pub static VERSION_{ident}: &str = \"{ws_version}\";\n",
                fnv1a(&bytes)
            )
            .expect("write to src");
        } else {
            // Release builds must have an agent; debug can run local-only.
            if profile == "release" {
                println!(
                    "cargo:warning=No {triple} agent found for release build — local-only, no remote monitoring (rebuild with ./build.sh to include it)"
                );
            }
            write!(
                src,
                "pub static AGENT_{ident}: Option<&[u8]> = None;\n\
                 pub static HASH_{ident}: &str = \"missing\";\n\
                 pub static VERSION_{ident}: &str = \"missing\";\n"
            )
            .expect("write to src");
        }
    }

    std::fs::write(out_path.join("agents.rs"), src).expect("write agents.rs");
}
