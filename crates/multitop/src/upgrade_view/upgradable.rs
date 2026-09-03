//! Parsing for `apt list --upgradable` output.
//!
//! Pure string parsing: turns the terminal output of `apt list --upgradable`
//! into a compact one-line summary for the upgrade view header.

/// Parse the stdout of `apt list --upgradable` into a human-readable summary.
///
/// Returns:
/// - `None` if the output does not look like `apt` output at all.
/// - `Some("0 pkgs (up to date)")` if no upgradable packages are listed.
/// - `Some("N pkgs (...)")` describing package count, held packages, and kernel status.
#[must_use]
pub fn parse_upgradable_output(output: &str) -> Option<String> {
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    if lines.is_empty() {
        return Some("0 pkgs (up to date)".to_string());
    }

    let mut pkg_count = 0usize;
    let mut held_count = 0usize;
    let mut kernel_held = false;
    let mut kernel_diff: Option<(String, String)> = None;

    for line in lines {
        if line.starts_with("Listing...") || line.starts_with("WARNING:") {
            continue;
        }

        // An upgradable package line has the shape:
        // `name/release new_version arch [upgradable from: old_version] ...`
        if line.contains("[upgradable from:") || line.contains('/') {
            pkg_count += 1;
            let is_held = line.contains("[held]") || line.contains("held");
            if is_held {
                held_count += 1;
            }

            let pkg_name = line.split('/').next().unwrap_or("");
            if pkg_name.starts_with("linux-image")
                || pkg_name.starts_with("linux-generic")
                || pkg_name.starts_with("linux-modules")
            {
                if is_held {
                    kernel_held = true;
                }
                if let Some(pos) = line.find("[upgradable from:") {
                    let rest = &line[pos + 17..];
                    if let Some(end) = rest.find(']') {
                        let old_ver = rest[..end].trim();
                        let before = line[..pos].trim();
                        let parts: Vec<&str> = before.split_whitespace().collect();
                        if parts.len() >= 2 {
                            let new_ver = parts[1];
                            let short_old = short_version(old_ver);
                            let short_new = short_version(new_ver);
                            if !short_old.is_empty() && !short_new.is_empty() {
                                kernel_diff = Some((short_old, short_new));
                            }
                        }
                    }
                }
            }
        }
    }

    if pkg_count == 0 {
        return Some("0 pkgs (up to date)".to_string());
    }

    let mut notes = Vec::new();
    if let Some((old_v, new_v)) = kernel_diff {
        if kernel_held {
            notes.push(format!("kernel {old_v}\u{2192}{new_v} held"));
        } else {
            notes.push(format!("kernel {old_v}\u{2192}{new_v}"));
        }
    } else if kernel_held {
        notes.push("kernel held".to_string());
    } else if held_count > 0 {
        notes.push(format!("{held_count} held"));
    }

    if notes.is_empty() {
        Some(format!(
            "{pkg_count} pkg{}",
            if pkg_count == 1 { "" } else { "s" }
        ))
    } else {
        Some(format!(
            "{pkg_count} pkg{} ({})",
            if pkg_count == 1 { "" } else { "s" },
            notes.join(", ")
        ))
    }
}

/// Simplify a package version for compact display: `6.8.0-45.45` -> `6.8`.
fn short_version(ver: &str) -> String {
    let clean = ver.trim_start_matches(|c: char| !c.is_ascii_digit());
    let parts: Vec<&str> = clean.split('.').collect();
    if parts.len() >= 2 {
        format!(
            "{}.{}",
            parts[0],
            parts[1].split('-').next().unwrap_or(parts[1])
        )
    } else {
        clean.to_string()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn empty_output_is_up_to_date() {
        assert_eq!(
            parse_upgradable_output("").as_deref(),
            Some("0 pkgs (up to date)")
        );
        assert_eq!(
            parse_upgradable_output("Listing... Done\n").as_deref(),
            Some("0 pkgs (up to date)")
        );
    }

    #[test]
    fn plain_packages_counted() {
        let text = "Listing... Done\n\
                    curl/noble-updates 8.5.0-2 amd64 [upgradable from: 8.5.0-1]\n\
                    libcurl4/noble-updates 8.5.0-2 amd64 [upgradable from: 8.5.0-1]\n";
        assert_eq!(parse_upgradable_output(text).as_deref(), Some("2 pkgs"));
    }

    #[test]
    fn held_kernel_with_version_diff() {
        let text = "Listing... Done\n\
                    curl/noble-updates 8.5.0-2 amd64 [upgradable from: 8.5.0-1]\n\
                    linux-generic/noble-updates 6.9.0-1 amd64 [upgradable from: 6.8.0-45] [held]\n";
        let res = parse_upgradable_output(text).unwrap();
        assert!(res.starts_with("2 pkgs"));
        assert!(res.contains("kernel 6.8\u{2192}6.9 held"), "res: {res}");
    }

    #[test]
    fn held_non_kernel_packages() {
        let text = "Listing... Done\n\
                    nginx/noble 1.24.0 amd64 [upgradable from: 1.22.0] [held]\n";
        assert_eq!(
            parse_upgradable_output(text).as_deref(),
            Some("1 pkg (1 held)")
        );
    }
}
