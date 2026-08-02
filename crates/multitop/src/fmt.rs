//! Formatting helpers for display and status lines.

/// Format text as an error line (meter-high palette).
pub fn error_line(text: impl std::fmt::Display) -> String {
    let pal = &multitop_agent::color::ANSI;
    format!("{}{text}{}", pal.meter_high(), pal.reset)
}

/// Format text as a status line (meter-mid palette).
/// Mask a secret for display: one asterisk per CHARACTER.
///
/// Not per byte. The vault prompt used `len()`, so a password containing any
/// non-ASCII character drew more asterisks than the user had typed -- which
/// both looks wrong mid-typing and quietly reveals that the secret is not
/// plain ASCII. One function so the three places that mask cannot disagree.
#[must_use]
pub fn mask_secret(secret: &str) -> String {
    "*".repeat(secret.chars().count())
}

pub fn status_line(text: impl std::fmt::Display) -> String {
    let pal = &multitop_agent::color::ANSI;
    format!("{}{text}{}", pal.meter_mid(), pal.reset)
}

/// Format text as a header line (primary + bold palette).
pub fn header_line(text: impl std::fmt::Display) -> String {
    let pal = &multitop_agent::color::ANSI;
    format!("{}{}{text}{}", pal.primary(), pal.bold, pal.reset)
}

/// Convert a Unix timestamp (seconds since epoch) to `YYYY-MM-DD HH:MM:SS UTC`.
#[must_use]
pub fn unixtime_to_str(secs: u64) -> String {
    let days = secs / 86400;
    let rem_secs = secs % 86400;
    let hours = rem_secs / 3600;
    let minutes = (rem_secs % 3600) / 60;
    let seconds = rem_secs % 60;

    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02} {hours:02}:{minutes:02}:{seconds:02} UTC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_line_prefix() {
        let result = error_line("test error");
        assert!(result.contains("test error"));
        assert!(result.contains("\x1b[")); // ANSI escape
    }

    #[test]
    fn error_line_contains_input() {
        let result = error_line("my error message");
        // Uses RGB color for meter_high
        assert!(result.contains("my error message"));
        assert!(result.contains("\x1b["));
        assert!(result.ends_with("\x1b[0m"));
    }

    #[test]
    fn status_line_prefix() {
        let result = status_line("test status");
        assert!(result.contains("test status"));
        assert!(result.contains("\x1b["));
    }

    #[test]
    fn status_line_contains_input() {
        let result = status_line("my status");
        // Uses RGB color for meter_mid
        assert!(result.contains("my status"));
        assert!(result.contains("\x1b["));
        assert!(result.ends_with("\x1b[0m"));
    }

    #[test]
    fn header_line_prefix() {
        let result = header_line("test header");
        assert!(result.contains("test header"));
        assert!(result.contains("\x1b["));
        assert!(result.contains("\x1b[1m")); // bold
    }

    #[test]
    fn unixtime_to_str_zero() {
        let result = unixtime_to_str(0);
        assert_eq!(result, "1970-01-01 00:00:00 UTC");
    }

    #[test]
    fn unixtime_to_str_recent() {
        let result = unixtime_to_str(1_704_067_200); // 2024-01-01 00:00:00 UTC
        assert_eq!(result, "2024-01-01 00:00:00 UTC");
    }

    #[test]
    fn unixtime_to_str_old() {
        let result = unixtime_to_str(86400); // 1970-01-02 00:00:00 UTC
        assert_eq!(result, "1970-01-02 00:00:00 UTC");
    }
}

#[cfg(test)]
mod mask_tests {
    use super::mask_secret;

    /// One asterisk per character, never per byte.
    ///
    /// The vault prompt masked with `len()`, so "pässwörd" (8 characters, 10
    /// bytes) drew ten asterisks. That is wrong on screen while typing and
    /// leaks that the secret contains non-ASCII.
    #[test]
    fn secrets_are_masked_by_character_not_byte() {
        assert_eq!(mask_secret("hunter2"), "*******");
        assert_eq!(mask_secret(""), "");

        let multibyte = "pässwörd";
        assert_eq!(multibyte.chars().count(), 8);
        assert_eq!(multibyte.len(), 10, "it really is wider in bytes");
        assert_eq!(
            mask_secret(multibyte).chars().count(),
            8,
            "the mask must not reveal the byte width"
        );

        // Emoji and combining marks are characters too.
        assert_eq!(mask_secret("a\u{4e16}b").chars().count(), 3);
    }
}
