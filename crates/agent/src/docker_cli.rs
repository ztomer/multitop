//! Docker CLI fallback helper for system docker command invocations.

use std::collections::HashMap;
use std::process::Command;

use crate::docker::Container;

pub fn docker_cli(args: &[&str]) -> Option<String> {
    let out = Command::new("docker").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

pub fn parse_cli_ps(text: &str) -> Vec<Container> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 4 {
                return None;
            }
            Some(Container {
                name: f[0].to_string(),
                status: f[1].to_string(),
                image: f[2].to_string(),
                id: f[3].to_string(),
            })
        })
        .collect()
}

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
