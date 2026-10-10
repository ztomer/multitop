//! Refitting lines and headers dynamically to panel width.

/// Reflow line 0 header if `target_cols` differs from the line's pre-rendered width.
#[must_use]
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
        .map(|c| {
            if (0xFF01..=0xFF5E).contains(&(c as u32)) {
                2
            } else {
                1
            }
        })
        .sum();

    // The agent's own roles for this header (`center_header`): the title in
    // primary and bold, the rule in secondary. These were `ESC[36;1m` and
    // `ESC[90m` -- terminal-decided cyan and grey, adapted by nothing.
    let pal = crate::palette_store::current();
    let title = format!("{}{}{fw_trimmed}{}", pal.primary(), pal.bold, pal.reset);
    let space_needed = disp_w + 2;
    if target_cols < space_needed {
        return Some(title);
    }
    let rem = target_cols - space_needed;
    let left_len = rem / 2;
    let right_len = rem - left_len;

    let rule = |n: usize| format!("{}{}{}", pal.secondary(), "\u{2500}".repeat(n), pal.reset);
    Some(format!("{} {title} {}", rule(left_len), rule(right_len)))
}

#[must_use]
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
        let pal = crate::palette_store::current();
        return format!(
            " {}{}{}",
            pal.secondary(),
            "\u{2500}".repeat(rule_w),
            pal.reset
        );
    }
    line.to_string()
}

#[cfg(test)]
#[path = "refit_tests.rs"]
mod tests;
