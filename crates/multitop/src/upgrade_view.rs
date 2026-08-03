//! The status header shown at the top of a panel's Upgrade view.
//!
//! Pressing `u` once switches every panel here without starting anything, so
//! this is the screen the user reads to decide whether to press `u` again. It
//! has to answer, per host: what command will run, when it last ran and how it
//! went, whether credentials are ready, and — if the host cannot be upgraded at
//! all — exactly what to change to fix that.
//!
//! Pure string building: no terminal, no clock, no I/O. `now` is passed in so
//! the relative times are testable.

use multitop_agent::color::Palette;

use crate::config::Server;
use crate::state::{HostUpdate, Outcome};

/// Whether a sudo password is available, and where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credential {
    /// Loaded from the vault or the OS keychain.
    Stored,
    /// Held for this session only.
    Session,
    /// Nothing available; the upgrade will prompt for this host.
    Missing,
    /// A vault exists but is locked, so what it holds for this host is not
    /// known yet. Reading the OS credential store to find out would raise a
    /// system credential dialog on a screen the user only came to *read* --
    /// which is one of the two password prompts a single upgrade used to cost.
    VaultLocked,
    /// Nothing available and there is no vault at all, so every host will
    /// prompt separately. Called out on its own because the fix differs: one
    /// vault removes every prompt, whereas saving one host's password removes
    /// only that host's.
    MissingNoVault,
}

/// Everything the header needs, gathered by the caller.
pub struct Status<'a> {
    pub server: &'a Server,
    pub record: HostUpdate,
    pub credential: Credential,
    pub running: bool,
}

/// Render a duration as a compact human string: `1m 12s`, `2h 5m`, `45s`.
#[must_use]
pub fn fmt_duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {}s", secs / 60, secs % 60),
        _ => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// Render how long ago `then` was, relative to `now`: `4 days ago`.
#[must_use]
pub fn fmt_ago(then: u64, now: u64) -> String {
    if then > now {
        return "in the future".to_string();
    }
    let d = now - then;
    match d {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86399 => {
            let h = d / 3600;
            format!("{h} hour{} ago", if h == 1 { "" } else { "s" })
        }
        _ => {
            let days = d / 86400;
            format!("{days} day{} ago", if days == 1 { "" } else { "s" })
        }
    }
}

/// The one-word state badge shown next to the host name.
#[must_use]
pub const fn badge(status: &Status) -> &'static str {
    if status.running {
        return "running";
    }
    if status.server.upgrade_cmd.is_none() {
        return "not configured";
    }
    match status.record.outcome() {
        Outcome::Interrupted => "interrupted",
        Outcome::Failed => "last run failed",
        Outcome::Never | Outcome::Ok => "ready",
    }
}

fn label(pal: &Palette, text: &str) -> String {
    format!("{}{text}{}", pal.muted(), pal.reset)
}

/// Colour for the state badge: green when it is safe to go, amber when the
/// user should look before pressing again.
fn badge_color(status: &Status, pal: &Palette) -> &'static str {
    if status.running {
        return pal.meter_mid();
    }
    if status.server.upgrade_cmd.is_none() {
        return pal.muted();
    }
    match status.record.outcome() {
        Outcome::Failed | Outcome::Interrupted => pal.meter_high(),
        Outcome::Never | Outcome::Ok => pal.meter_low(),
    }
}

/// The "Last run" value: when it happened, how it ended, and how long it took.
fn last_run_text(status: &Status, pal: &Palette, now: u64) -> String {
    match status.record.outcome() {
        Outcome::Never => format!("{}never{}", pal.muted(), pal.reset),
        Outcome::Interrupted => {
            let when = status
                .record
                .started_at
                .map_or_else(String::new, |t| format!("{} \u{b7} ", fmt_ago(t, now)));
            format!("{when}{}interrupted{}", pal.meter_high(), pal.reset)
        }
        Outcome::Ok | Outcome::Failed => {
            let when = status
                .record
                .finished_at
                .map_or_else(String::new, |t| fmt_ago(t, now));
            let verdict = if status.record.outcome() == Outcome::Ok {
                format!("{}ok{}", pal.meter_low(), pal.reset)
            } else {
                format!("{}failed{}", pal.meter_high(), pal.reset)
            };
            let dur = status
                .record
                .duration_secs()
                .map_or_else(String::new, |d| format!(" ({})", fmt_duration(d)));
            format!("{when} \u{b7} {verdict}{dur}")
        }
    }
}

/// The "what to do next" block at the bottom of the header.
///
/// Kept under ~40 visible columns per line: with four panels the grid is two
/// columns wide, and `ui::visible` hard-truncates rather than wrapping, so a
/// longer sentence loses exactly the part that tells the user what to do.
fn next_action(status: &Status, pal: &Palette) -> Vec<String> {
    if status.running {
        return vec![format!(
            "{}\u{2192} running \u{2014} do not quit{}",
            pal.meter_mid(),
            pal.reset
        )];
    }
    if status.server.upgrade_cmd.is_none() {
        return vec![
            format!(
                "{}\u{26a0} no upgrade_cmd \u{2014} host is skipped{}",
                pal.meter_high(),
                pal.reset
            ),
            format!(
                "{}  set upgrade_cmd in config.toml{}",
                pal.muted(),
                pal.reset
            ),
        ];
    }
    let mut out = Vec::new();
    if status.record.outcome() == Outcome::Interrupted {
        out.push(format!(
            "{}\u{26a0} last run never finished \u{2014} check host{}",
            pal.meter_high(),
            pal.reset
        ));
    }
    out.push(format!(
        "{}\u{2192} u to run \u{b7} s to go back{}",
        pal.meter_mid(),
        pal.reset
    ));
    out
}

/// Build the header lines for one panel.
///
/// `now` is a Unix timestamp in seconds. `width` is the panel's inner width,
/// used only to decide whether the separator rule is worth drawing.
///
/// # The sacrificial first line
///
/// `ui::draw` unconditionally replaces `view[0]` with the panel's
/// `user@host` banner rule. Anything put there is destroyed every frame, so
/// line 0 is deliberately a placeholder and the real content starts at line 1.
/// Putting the host name there too would have been invisible and redundant
/// with the banner.
#[must_use]
pub fn header(status: &Status, pal: &Palette, now: u64, width: usize) -> Vec<String> {
    // Overwritten by the panel banner; see the note above.
    let mut out = vec![String::new()];

    out.push(format!(
        "{}  {}{}{}",
        label(pal, "Status   "),
        badge_color(status, pal),
        badge(status),
        pal.reset
    ));

    // Command — the single most useful thing to see before pressing u again.
    match status.server.upgrade_cmd.as_deref() {
        Some(cmd) => out.push(format!("{}  {cmd}{}", label(pal, "Command  "), pal.reset)),
        None => out.push(format!(
            "{}  {}(none){}",
            label(pal, "Command  "),
            pal.muted(),
            pal.reset
        )),
    }

    out.push(format!(
        "{}  {}",
        label(pal, "Last run "),
        last_run_text(status, pal, now)
    ));

    // Credentials, only when the host can actually be upgraded.
    if status.server.upgrade_cmd.is_some() {
        let cred = match status.credential {
            Credential::Stored => format!("{}password stored{}", pal.meter_low(), pal.reset),
            Credential::Session => {
                format!("{}password set for this session{}", pal.reset, pal.reset)
            }
            Credential::Missing => {
                format!(
                    "{}will prompt \u{b7} p to save{}",
                    pal.meter_high(),
                    pal.reset
                )
            }
            Credential::MissingNoVault => {
                format!(
                    "{}will prompt \u{b7} no vault set up{}",
                    pal.meter_high(),
                    pal.reset
                )
            }
            Credential::VaultLocked => {
                format!(
                    "{}from the vault \u{b7} unlocks on run{}",
                    pal.reset, pal.reset
                )
            }
        };
        out.push(format!("{}  {cred}", label(pal, "Sudo     ")));
    }

    out.push(String::new());
    out.extend(next_action(status, pal));

    if width >= 20 {
        out.push(format!(
            "{}{}{}",
            pal.muted(),
            "\u{2500}".repeat(width),
            pal.reset
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const DAY: u64 = 86400;
    const NOW: u64 = 1_800_000_000;

    fn server(cmd: Option<&str>) -> Server {
        Server {
            host: "web-01".into(),
            port: 22,
            user: "admin".into(),
            upgrade_cmd: cmd.map(str::to_string),
        }
    }

    fn plain(lines: &[String]) -> String {
        let joined = lines.join("\n");
        // Strip ANSI so assertions read against the visible text.
        let mut out = String::new();
        let mut chars = joined.chars();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for c2 in chars.by_ref() {
                    if c2 == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    fn render(status: &Status) -> String {
        plain(&header(status, &multitop_agent::color::ANSI, NOW, 40))
    }

    #[test]
    fn shows_the_command_and_that_it_never_ran() {
        let s = server(Some("apt update && apt upgrade -y"));
        let text = render(&Status {
            server: &s,
            record: HostUpdate::default(),
            credential: Credential::Missing,
            running: false,
        });
        assert!(text.contains("apt update && apt upgrade -y"), "{text}");
        assert!(text.contains("never"), "{text}");
        assert!(text.contains("ready"), "{text}");
        assert!(text.contains("u to run"), "{text}");
    }

    #[test]
    fn missing_config_states_the_problem_and_the_fix() {
        let s = server(None);
        let text = render(&Status {
            server: &s,
            record: HostUpdate::default(),
            credential: Credential::Missing,
            running: false,
        });
        assert!(text.contains("not configured"), "{text}");
        assert!(text.contains("host is skipped"), "{text}");
        assert!(
            text.contains("set upgrade_cmd in config.toml"),
            "the fix must be spelled out: {text}"
        );
        // Nothing to run, so it must not invite the user to press u.
        assert!(!text.contains("u to run"), "{text}");
    }

    #[test]
    fn successful_run_shows_when_and_how_long() {
        let s = server(Some("apt upgrade"));
        let text = render(&Status {
            server: &s,
            record: HostUpdate {
                started_at: Some(NOW - 4 * DAY - 72),
                finished_at: Some(NOW - 4 * DAY),
                success: true,
            },
            credential: Credential::Stored,
            running: false,
        });
        assert!(text.contains("4 days ago"), "{text}");
        assert!(text.contains("ok"), "{text}");
        assert!(text.contains("1m 12s"), "{text}");
        assert!(text.contains("password stored"), "{text}");
    }

    #[test]
    fn failed_run_is_called_out() {
        let s = server(Some("apt upgrade"));
        let text = render(&Status {
            server: &s,
            record: HostUpdate {
                started_at: Some(NOW - 3600),
                finished_at: Some(NOW - 3500),
                success: false,
            },
            credential: Credential::Stored,
            running: false,
        });
        assert!(text.contains("last run failed"), "{text}");
        assert!(text.contains("failed"), "{text}");
    }

    #[test]
    fn interrupted_run_warns_before_retrying() {
        let s = server(Some("apt upgrade"));
        let text = render(&Status {
            server: &s,
            record: HostUpdate {
                started_at: Some(NOW - 2 * DAY),
                finished_at: None,
                success: false,
            },
            credential: Credential::Stored,
            running: false,
        });
        assert!(text.contains("interrupted"), "{text}");
        assert!(text.contains("never finished"), "{text}");
        assert!(text.contains("2 days ago"), "{text}");
    }

    #[test]
    fn running_host_says_do_not_quit() {
        let s = server(Some("apt upgrade"));
        let text = render(&Status {
            server: &s,
            record: HostUpdate::default(),
            credential: Credential::Stored,
            running: true,
        });
        assert!(text.contains("running"), "{text}");
        assert!(text.contains("do not quit"), "{text}");
        assert!(!text.contains("u to run"), "{text}");
    }

    #[test]
    fn durations_and_relative_times_read_naturally() {
        assert_eq!(fmt_duration(45), "45s");
        assert_eq!(fmt_duration(72), "1m 12s");
        assert_eq!(fmt_duration(7500), "2h 5m");
        assert_eq!(fmt_ago(NOW, NOW), "just now");
        assert_eq!(fmt_ago(NOW - 120, NOW), "2 min ago");
        assert_eq!(fmt_ago(NOW - 3600, NOW), "1 hour ago");
        assert_eq!(fmt_ago(NOW - 2 * 3600, NOW), "2 hours ago");
        assert_eq!(fmt_ago(NOW - DAY, NOW), "1 day ago");
        assert_eq!(fmt_ago(NOW - 5 * DAY, NOW), "5 days ago");
    }
}
