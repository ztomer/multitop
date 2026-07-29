use crate::color::Palette;
use crate::docker::{truncate, Row, CPU_W, MEM_W, NAME_W, STATUS_W};
use crate::fmt::center_header;

pub fn render(
    host: &str,
    cols: usize,
    max_rows: usize,
    rows: &[Row],
    pal: &Palette,
    sort_by: crate::SortBy,
) -> Vec<String> {
    let mut out = Vec::with_capacity(rows.len() + 4);
    out.push(center_header(host, cols, pal));

    if rows.is_empty() {
        out.push(format!(" {}No running containers{}", pal.muted(), pal.reset));
        return out;
    }

    let mut sorted_rows = rows.to_vec();
    match sort_by {
        crate::SortBy::Cpu => sorted_rows.sort_unstable_by(|a, b| {
            b.cpu_pct
                .partial_cmp(&a.cpu_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.mem_bytes.cmp(&a.mem_bytes))
                .then_with(|| a.name.cmp(&b.name))
        }),
        crate::SortBy::Mem => sorted_rows.sort_unstable_by(|a, b| {
            b.mem_bytes
                .cmp(&a.mem_bytes)
                .then_with(|| b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap_or(std::cmp::Ordering::Equal))
                .then_with(|| a.name.cmp(&b.name))
        }),
    }

    out.push(format!(
        " {}{}{:<NAME_W$}  {:<STATUS_W$}  {:<CPU_W$}  {:<MEM_W$}{}",
        pal.secondary(), pal.bold, "NAME", "STATUS", "CPU", "MEM", pal.reset,
    ));
    let rule = format!(
        " {}{}{}",
        pal.primary(),
        "\u{2500}".repeat(cols.saturating_sub(2)),
        pal.reset
    );
    out.push(rule.clone());

    let chrome_rows = 4;
    let body_budget = max_rows.saturating_sub(chrome_rows);
    let (visible, overflow) = if body_budget >= sorted_rows.len() || max_rows == 0 {
        (&sorted_rows[..], 0)
    } else {
        let show = body_budget.saturating_sub(1);
        (&sorted_rows[..show], sorted_rows.len() - show)
    };

    for r in visible {
        let cpu_c = if r.cpu_pct >= 80.0 {
            pal.meter_high()
        } else if r.cpu_pct >= 20.0 {
            pal.meter_mid()
        } else {
            pal.meter_low()
        };
        out.push(format!(
            " {}{}{:<NAME_W$}{}  {}{:<STATUS_W$}{}  {}{:<CPU_W$}{}  {}{:<MEM_W$}{}",
            pal.bold,
            pal.text(),
            truncate(&r.name, NAME_W),
            pal.reset,
            pal.status_color(&r.status),
            truncate(&r.status, STATUS_W),
            pal.reset,
            cpu_c,
            r.cpu,
            pal.reset,
            pal.primary(),
            r.mem,
            pal.reset,
        ));
    }

    if overflow > 0 {
        out.push(format!(
            " {}\u{2026}+{} more{}",
            pal.muted(), overflow, pal.reset
        ));
    }

    out.push(rule);
    out
}