//! Docker CLI fallback helper for system docker command invocations.

use std::collections::HashMap;
use std::process::Command;

pub fn docker_cli(args: &[&str]) -> Option<String> {
    let out = Command::new("docker").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

// `parse_cli_ps` lives in `docker.rs`. A second copy here was shadowed by that
// one through the `pub use docker_cli::*` glob and so was never called — and
// it disagreed with the live copy about how many fields a row needs.

pub fn parse_cli_stats(text: &str) -> HashMap<String, (String, String)> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 3 {
                return None;
            }
            Some((f[0].to_string(), (f[1].to_string(), f[2].to_string())))
        })
        .collect()
}
