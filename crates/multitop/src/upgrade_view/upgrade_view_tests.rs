use super::*;

#[cfg(test)]
mod tests_module {
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
            custom_command: None,
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
            upgradable: None,
        });
        assert!(text.contains("apt update && apt upgrade -y"), "{text}");
        assert!(text.contains("never"), "{text}");
        assert!(text.contains("ready"), "{text}");
        assert!(text.contains("u to run"), "{text}");
    }

    #[test]
    fn shows_upgradable_packages_summary_when_present() {
        let s = server(Some("apt update && apt upgrade -y"));
        let text = render(&Status {
            server: &s,
            record: HostUpdate::default(),
            credential: Credential::Stored,
            running: false,
            upgradable: Some("12 pkgs (kernel 6.8\u{2192}6.9 held)".to_string()),
        });
        assert!(
            text.contains("Packages   12 pkgs (kernel 6.8\u{2192}6.9 held)"),
            "{text}"
        );
    }

    #[test]
    fn missing_config_states_the_problem_and_the_fix() {
        let s = server(None);
        let text = render(&Status {
            server: &s,
            record: HostUpdate::default(),
            credential: Credential::Missing,
            running: false,
            upgradable: None,
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
            upgradable: None,
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
            upgradable: None,
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
            upgradable: None,
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
            upgradable: None,
        });
        assert!(text.contains("running"), "{text}");
        assert!(text.contains("do not quit"), "{text}");
        assert!(!text.contains("u to run"), "{text}");
    }

    /// The record a *genuinely* running host has, which is not the one the test
    /// above uses.
    ///
    /// `HostUpdate::default()` has no `started_at`, and the app never produces
    /// that for a running host: it writes `started_at: Some(now)` with no
    /// `finished_at` at the moment the run begins, deliberately, so that a
    /// crash leaves an interrupted record behind. A run in flight therefore has
    /// *exactly the shape of an interrupted one*, and the header read that
    /// shape as a verdict -- printing "Status running" and
    /// "Last run just now - interrupted" in consecutive lines, about the same
    /// run. "interrupted" is the word that sends an operator to check a host
    /// that is perfectly fine.
    ///
    /// The old test modelled a state the app cannot reach, which is why seven
    /// passes went by without seeing this.
    #[test]
    fn a_run_in_flight_is_not_reported_as_an_interrupted_one() {
        let s = server(Some("apt upgrade"));
        let text = render(&Status {
            server: &s,
            // What `App::mark_upgrades_started` actually writes.
            record: HostUpdate {
                started_at: Some(NOW - 120),
                finished_at: None,
                success: false,
            },
            credential: Credential::Stored,
            running: true,
            upgradable: None,
        });

        assert!(
            !text.contains("interrupted"),
            "a run that is still going has not been interrupted: {text}"
        );
        assert!(
            text.contains("in progress"),
            "and the last-run line must say what is actually true: {text}"
        );
        assert!(
            text.contains("2 min ago"),
            "including how long it has been going, which is the useful part: {text}"
        );
        assert!(text.contains("do not quit"), "{text}");
    }

    /// The other side of it: once the run is over, an interrupted record must
    /// still be reported as interrupted. The fix must not swallow the warning
    /// it was protecting.
    #[test]
    fn a_run_that_really_was_interrupted_still_says_so() {
        let s = server(Some("apt upgrade"));
        let text = render(&Status {
            server: &s,
            record: HostUpdate {
                started_at: Some(NOW - 120),
                finished_at: None,
                success: false,
            },
            credential: Credential::Stored,
            running: false,
            upgradable: None,
        });
        assert!(text.contains("interrupted"), "{text}");
        assert!(text.contains("last run never finished"), "{text}");
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

#[cfg(test)]
mod header_width_tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::{header, Credential, Status};
    use crate::config::Server;
    use crate::state::HostUpdate;

    /// The widest pane a host actually gets in the ordinary grid.
    ///
    /// At 40 columns the grid is one column wide and `ui::draw` leaves 38 cells
    /// of content after the side margins. Four panels halve that again; this is
    /// the generous case, and three of the five credential states did not fit
    /// even here.
    const PANE: usize = 38;

    fn visible_width(line: &str) -> usize {
        multitop_agent::color::strip_ansi(line).chars().count()
    }

    /// Every line of the header must fit the pane, for every credential state.
    ///
    /// `next_action`'s doc comment states the rule -- "Kept under ~40 visible
    /// columns per line ... `ui::visible` hard-truncates rather than wrapping,
    /// so a longer sentence loses exactly the part that tells the user what to
    /// do" -- and the `next_action` lines obey it. The `Sudo` row, in the same
    /// header, did not: three of the five states ran to 40, 40 and 42 cells and
    /// reached the operator as "no vault set", "this sessio" and "unlocks on r".
    ///
    /// Asserted over every variant so a sixth cannot be added over-width.
    #[test]
    fn every_credential_state_fits_the_pane_it_is_drawn_in() {
        let server = Server {
            host: "web-01".into(),
            port: 22,
            user: "admin".into(),
            upgrade_cmd: Some("sudo apt update && sudo apt upgrade -y".into()),
            custom_command: None,
        };
        for credential in [
            Credential::Stored,
            Credential::Session,
            Credential::Missing,
            Credential::VaultLocked,
            Credential::MissingNoVault,
        ] {
            for running in [false, true] {
                let status = Status {
                    server: &server,
                    record: HostUpdate::default(),
                    credential,
                    running,
                    upgradable: None,
                };
                for line in header(&status, &multitop_agent::color::ANSI, 1_800_000_000, PANE) {
                    let w = visible_width(&line);
                    // The command row is data, not guidance: it is allowed to be
                    // clipped, and the separator rule is built to the width.
                    let plain = multitop_agent::color::strip_ansi(&line);
                    if plain.contains("Command") || plain.trim_start().starts_with('\u{2500}') {
                        continue;
                    }
                    assert!(
                        w <= PANE,
                        "{credential:?} running={running}: {w} cells needs {PANE}: {plain:?}"
                    );
                }
            }
        }
    }
}
