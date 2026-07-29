//! Refitting lines and headers dynamically to panel width.

/// Reflow line 0 header if target_cols differs from the line's pre-rendered width.
pub fn refit_header(line: &str, target_cols: usize) -> Option<String> {
    if !line.contains('\u{2500}') {
        return None;
    }
    let fw: String = line
        .chars()
        .filter(|c| (0xFF01..=0xFF5E).contains(&(*c as u32)) || *c == ' ')
        .collect();
    let fw_trimmed = fw.trim();
    if fw_trimmed.is_empty() {
        return None;
    }
    let disp_w: usize = fw_trimmed
        .chars()
        .map(|c| if (0xFF01..=0xFF5E).contains(&(c as u32)) { 2 } else { 1 })
        .sum();

    if target_cols <= disp_w {
        return Some(format!("\x1b[36;1m{fw_trimmed}\x1b[0m"));
    }
    let space_needed = disp_w + 2;
    if target_cols < space_needed {
        return Some(format!("\x1b[36;1m{fw_trimmed}\x1b[0m"));
    }
    let rem = target_cols - space_needed;
    let left_len = rem / 2;
    let right_len = rem - left_len;

    Some(format!(
        "\x1b[90m{}\x1b[0m\x1b[36;1m {} \x1b[0m\x1b[90m{}\x1b[0m",
        "\u{2500}".repeat(left_len),
        fw_trimmed,
        "\u{2500}".repeat(right_len)
    ))
}

pub fn refit_line(line: &str, target_cols: usize) -> String {
    if target_cols == 0 {
        return line.to_string();
    }
    if let Some(h) = refit_header(line, target_cols) {
        return h;
    }
    let plain = multitop_agent::color::strip_ansi(line);
    let trimmed = plain.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|c| c == '\u{2500}') {
        let rule_w = target_cols.saturating_sub(2);
        return format!(" \x1b[90m{}\x1b[0m", "\u{2500}".repeat(rule_w));
    }
    line.to_string()
}
