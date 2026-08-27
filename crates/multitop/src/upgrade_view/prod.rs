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
    /// The OS credential store is being asked right now, off the loop thread.
    /// The upgrade pane must not start this host until the answer lands -- the
    /// lookup can block on a system dialog, and starting on an unread password
    /// would be guessing. Shown instead of `Missing` so an empty answer reads
    /// as "pending" rather than "absent".
    Checking,
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

/// What the header is describing, once `running` and the record are read
/// together.
///
/// They are two views of one fact and they could contradict. **A run in flight
/// has exactly the shape of an interrupted one** -- started, never finished --
/// because that is the shape written when it starts, deliberately, so that a
/// crash leaves it behind. `badge`, `badge_color` and `next_action` each
/// checked `running` before consulting the record; `last_run_text` did not. So
/// while an upgrade was genuinely running, the header said
///
/// ```text
/// Status    running
/// Last run  just now - interrupted
///           -> running - do not quit
/// ```
///
/// in consecutive lines, about the same run -- and "interrupted" is the word
/// that sends an operator to go and check a host that is perfectly fine.
///
/// Asking here is now the only way to find out, so a fifth consumer cannot
/// read the record without first learning whether it describes the present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostState {
    /// A run is in flight right now; the record describes *it*, not a last run.
    Running,
    /// No `upgrade_cmd`, so nothing can run here.
    NotConfigured,
    /// Nothing in flight: how the last run ended.
    Last(Outcome),
}

impl Status<'_> {
    const fn state(&self) -> HostState {
        if self.running {
            HostState::Running
        } else if self.server.upgrade_cmd.is_none() {
            HostState::NotConfigured
        } else {
            HostState::Last(self.record.outcome())
        }
    }
}

/// The one-word state badge shown next to the host name.
#[must_use]
pub const fn badge(status: &Status) -> &'static str {
    match status.state() {
        HostState::Running => "running",
        HostState::NotConfigured => "not configured",
        HostState::Last(Outcome::Interrupted) => "interrupted",
        HostState::Last(Outcome::Failed) => "last run failed",
        HostState::Last(Outcome::Never | Outcome::Ok) => "ready",
    }
}

fn label(pal: &Palette, text: &str) -> String {
    format!("{}{text}{}", pal.muted(), pal.reset)
}

/// Colour for the state badge: green when it is safe to go, amber when the
/// user should look before pressing again.
fn badge_color(status: &Status, pal: &Palette) -> &'static str {
    match status.state() {
        HostState::Running => pal.meter_mid(),
        HostState::NotConfigured => pal.muted(),
        HostState::Last(Outcome::Failed | Outcome::Interrupted) => pal.meter_high(),
        HostState::Last(Outcome::Never | Outcome::Ok) => pal.meter_low(),
    }
}

/// The "Last run" value: when it happened, how it ended, and how long it took.
fn last_run_text(status: &Status, pal: &Palette, now: u64) -> String {
    match status.state() {
        // The record is this run, so say how long it has been going rather than
        // reading its shape as a verdict on a run that has not ended.
        HostState::Running => {
            let when = status
                .record
                .started_at
                .map_or_else(|| "just now".to_string(), |t| fmt_ago(t, now));
            format!("{when} \u{b7} {}in progress{}", pal.meter_mid(), pal.reset)
        }
        // Nothing in flight, so the record describes a run that is over --
        // whether or not the host is still configured to start another.
        HostState::NotConfigured | HostState::Last(_) => finished_run_text(status, pal, now),
    }
}

/// The "Last run" value for a run that has ended. Only reachable once
/// [`Status::state`] has established that nothing is in flight.
fn finished_run_text(status: &Status, pal: &Palette, now: u64) -> String {
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
    let state = status.state();
    if state == HostState::Running {
        return vec![format!(
            "{}\u{2192} running \u{2014} do not quit{}",
            pal.meter_mid(),
            pal.reset
        )];
    }
    if state == HostState::NotConfigured {
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
    if state == HostState::Last(Outcome::Interrupted) {
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
            // "password" is the `Sudo` row: saying it again cost the ending.
            Credential::Session => format!("{}set for this session{}", pal.reset, pal.reset),
            Credential::Checking => {
                format!(
                    "{}checking keychain \u{b7} not started{}",
                    pal.muted(),
                    pal.reset
                )
            }
            Credential::Missing => {
                format!(
                    "{}will prompt \u{b7} {} to save{}",
                    pal.meter_high(),
                    crate::consts::SETTINGS_KEY,
                    pal.reset
                )
            }
            Credential::MissingNoVault => {
                format!(
                    "{}will prompt \u{b7} no vault{}",
                    pal.meter_high(),
                    pal.reset
                )
            }
            Credential::VaultLocked => {
                format!("{}vault \u{b7} unlocks on run{}", pal.reset, pal.reset)
            }
        };
        out.push(format!("{}  {cred}", label(pal, "Sudo     ")));
    }

    out.push(String::new());
    out.extend(next_action(status, pal));

    if width >= crate::consts::UPGRADE_RULE_MIN_WIDTH {
        out.push(format!(
            "{}{}{}",
            pal.muted(),
            "\u{2500}".repeat(width),
            pal.reset
        ));
    }

    out
}
