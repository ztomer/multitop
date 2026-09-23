//! Every local server in the suite comes from `common::local_server`.
//!
//! A local server (port 0, or a host of `localhost`/`127.0.0.1` -- see
//! `ssh::is_local`) runs the agent on this machine, and which agent runs is
//! decided by the `MULTITOP_AGENT_EXE` seam. A test that builds one by hand
//! and forgets the seam passes only when a sibling test in the same binary
//! happened to set it first, and on a dev Mac it silently tests the installed
//! agent on `PATH` instead of this build.
//! `connect_local_server_succeeds_and_streams_snapshots` did both until CI ran
//! it alone. `common::local_server` sets the seam, so a server from there
//! cannot run another agent.
//!
//! This walks `tests/` and names the file and line of every place that writes
//! a local server literal without it, or spawns the agent directly without
//! setting the seam in the same function. What it cannot see is a local server whose port or
//! host arrives as a runtime value; those helpers exist to test `is_local` and
//! the config round trip, and spawn nothing. The needles are assembled with
//! `concat!` so this file's own source never contains them.
#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

/// The opening of a `Server` struct literal.
const LITERAL_OPEN: &str = concat!("Ser", "ver {");
/// Fields that make a literal local, whatever else it says.
const LOCAL_MARKERS: [&str; 4] = [
    concat!("port", ": 0"),
    concat!("host", ": \"localhost\""),
    concat!("host", ": \"127."),
    concat!("host", ": format!(\"127."),
];
/// A literal built on this is local by construction and has the seam set.
const FROM_COMMON: &str = concat!("common::", "local_", "server(");
const DIRECT_SPAWN: &str = concat!("spawn_local_", "agent(");
const SEAM: &str = concat!("use_this_builds_", "agent()");

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "common") {
                rs_files(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `needle` occurs in `text` not followed by a digit (`port: 0`, not `port: 022`).
fn has_marker(text: &str, needle: &str) -> bool {
    text.match_indices(needle).any(|(at, _)| {
        !text[at + needle.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
    })
}

/// The literal that opens at `lines[start]`, column `col`, up to its closing brace.
fn literal_at(lines: &[&str], start: usize, col: usize) -> String {
    let mut depth = 0i32;
    let mut text = String::new();
    for (n, line) in lines.iter().enumerate().skip(start) {
        let from = if n == start { col } else { 0 };
        for c in line[from..].chars() {
            text.push(c);
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            if depth == 0 && c == '}' {
                return text;
            }
        }
        text.push('\n');
    }
    text
}

/// Every violation in one source file, as `name:line: why`.
fn scan(name: &str, src: &str) -> Vec<String> {
    let lines: Vec<&str> = src.lines().collect();
    let mut found = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        for (col, _) in line.match_indices(LITERAL_OPEN) {
            let starts_word = !line[..col]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
            let literal = literal_at(&lines, n, col);
            if starts_word
                && LOCAL_MARKERS.iter().any(|m| has_marker(&literal, m))
                && !literal.contains(FROM_COMMON)
            {
                found.push(format!(
                    "{name}:{}: a local server written by hand -- build it from common::local_server",
                    n + 1
                ));
            }
        }
        if line.contains(DIRECT_SPAWN) {
            let seamed = lines[..n]
                .iter()
                .rev()
                .take_while(|l| !l.contains("fn "))
                .any(|l| l.contains(SEAM));
            if !seamed {
                found.push(format!(
                    "{name}:{}: the agent is spawned before this function sets the seam",
                    n + 1
                ));
            }
        }
    }
    found
}

#[test]
fn every_local_server_in_the_suite_has_this_builds_agent() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rs_files(&root.join("tests"), &mut files);
    files.sort();
    assert!(
        files.len() > 1,
        "scanned {} files -- a gate over nothing passes",
        files.len()
    );
    let found: Vec<String> = files
        .iter()
        .flat_map(|path| {
            let name = path.strip_prefix(root).unwrap().display().to_string();
            scan(&name, &fs::read_to_string(path).unwrap())
        })
        .collect();
    assert!(found.is_empty(), "\n{}\n", found.join("\n"));
}

/// The scanner can see each shape it polices, and passes the ones it allows.
#[test]
fn the_scan_can_fail() {
    let by_hand = concat!(
        "let s = Ser",
        "ver {\n    host",
        ": \"x\".into(),\n    port",
        ": 0,\n};\n"
    );
    assert_eq!(scan("a.rs", by_hand).len(), 1, "a port-zero literal");
    let by_host = concat!(
        "vec![Ser",
        "ver { host",
        ": \"localhost\".into(), port: 22 }]"
    );
    assert_eq!(scan("a.rs", by_host).len(), 1, "a local host name");
    let remote = concat!("Ser", "ver { host: \"web-01\".into(), port", ": 022 }");
    assert!(scan("a.rs", remote).is_empty(), "a remote server");
    let from_common = concat!(
        "Ser",
        "ver {\n    port",
        ": 0,\n    ..common::local_",
        "server(\"h\")\n}"
    );
    assert!(
        scan("a.rs", from_common).is_empty(),
        "struct update from common"
    );

    let unseamed = concat!("fn t() {\n    let c = spawn_local_", "agent(m, s);\n}\n");
    assert_eq!(scan("a.rs", unseamed).len(), 1, "a direct spawn, no seam");
    let seamed = concat!(
        "fn t() {\n    common::use_this_builds_",
        "agent();\n    let c = spawn_local_",
        "agent(m, s);\n}\n"
    );
    assert!(
        scan("a.rs", seamed).is_empty(),
        "a direct spawn after the seam"
    );
}
