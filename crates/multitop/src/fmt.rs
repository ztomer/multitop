//! Formatting helpers for display and status lines.

/// Format text as an error line (meter-high palette).
pub fn error_line(text: impl std::fmt::Display) -> String {
    let pal = &multitop_agent::color::ANSI;
    format!("{}{text}{}", pal.meter_high(), pal.reset)
}

/// Format text as a status line (meter-mid palette).
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
pub fn unixtime_to_str(secs: u64) -> String {
    let days = secs / 86400;
    let rem_secs = secs % 86400;
    let hours = rem_secs / 3600;
    let minutes = (rem_secs % 3600) / 60;
    let seconds = rem_secs % 60;

    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02} {hours:02}:{minutes:02}:{seconds:02} UTC")
}
