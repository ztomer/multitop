//! The keybar's confirmation rows - upgrade, kill, quit - each assembled
//! from whole chunks that shed in a declared order.

use crate::app::App;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// Assemble a keybar row from whole chunks, shedding in a declared order.
///
/// The same rule as every other row: a chunk is drawn whole or not at all, and
/// the shed order is a priority list, never "drop from the right". The way out
/// is never in the shed list.
///
/// Each chunk's width is measured from the spans that will be drawn, never
/// declared alongside them. A hand-written number is a second copy of the
/// string's length that drifts the moment the string is edited -- `[Esc] stay`
/// was declared as 11 cells and is 10 -- and the whole point of the budget is
/// that it is describing what actually goes on screen.
fn chunk_row(
    chunks: &[Vec<Span<'static>>],
    keybar_width: u16,
    shed: &[usize],
    sep_style: Style,
) -> Line<'static> {
    let widths: Vec<usize> = chunks
        .iter()
        .map(|spans| spans.iter().map(Span::width).sum())
        .collect();
    let kept = crate::layout::fit_row(&widths, 2, keybar_width as usize, shed);
    let mut out = Vec::new();
    for (n, index) in kept.iter().enumerate() {
        if n > 0 {
            out.push(Span::styled("  ", sep_style));
        }
        out.extend(chunks[*index].clone());
    }
    Line::from(out)
}

/// The confirmation that replaces the keybar while an upgrade is armed.
///
/// Kare's ruling, review round B: a keybar row rather than a box -- the box
/// was 38 cells wide at 40 columns and clipped its own cancel line to `Esc t`,
/// while the filter prompt renders every word whole at the same size. State
/// left, keys right, two spaces between. Shed order: the `· M skipped` tail
/// first (the ⚠ is already in those panes), then the count itself before the
/// keys; `[Esc] cancel` is last and in practice never -- it is the only thing
/// on the line the operator cannot guess.
///
/// The count is the alarm: with the run scoped to the filter, a grid showing
/// one host says "Upgrade 1 host", never a sentence long enough to hide the
/// others.
///
/// # The interrupted-run warning
///
/// The box this replaced also said "Previous upgrade was interrupted! Check
/// server state." when a run started and no completion followed. Rams
/// condemned the box's aggregate `Last update` *timestamp* and the ruling
/// dropped it; the warning is a different thing and dropping it with the box
/// was an accident. It is back, and it sheds **after** the count: how many
/// machines are about to be touched is a number the operator can recover by
/// looking at the grid, whereas "one of these has a half-finished dpkg
/// transaction on it" appears nowhere else on the screen.
pub(super) fn upgrade_confirm_row(
    app: &App,
    accent: Color,
    key_hi: Style,
    label: Style,
    keybar_width: u16,
) -> Line<'static> {
    let scope = app.filtered_indices();
    let skipped = app.upgrade_skip_hosts();
    let runnable = scope.len().saturating_sub(skipped.len());

    // The count is styled as the alarm: a grid showing one host that says
    // "Upgrade 8 hosts" must be louder than any sentence that would fit.
    let count = format!(
        "Upgrade {runnable} host{}",
        if runnable == 1 { "" } else { "s" }
    );
    let mut chunks: Vec<Vec<Span<'static>>> = vec![vec![Span::styled(
        count,
        Style::default()
            .fg(accent)
            .add_modifier(ratatui::style::Modifier::BOLD),
    )]];
    let count_at = 0;
    // Shed order is built by identity, not by position: which index holds what
    // depends on whether the optional chunks are present at all.
    let mut skipped_at = None;
    let mut interrupted_at = None;
    if !skipped.is_empty() {
        skipped_at = Some(chunks.len());
        chunks.push(vec![Span::styled(
            format!("\u{b7} {} skipped", skipped.len()),
            label,
        )]);
    }
    if app.previous_upgrade_interrupted() {
        interrupted_at = Some(chunks.len());
        chunks.push(vec![Span::styled(
            "\u{26a0} previous run interrupted",
            Style::default().fg(Color::Yellow),
        )]);
    }
    chunks.push(vec![
        Span::styled("[", label),
        Span::styled("U", key_hi),
        Span::styled("] go", label),
    ]);
    // Neither key ever sheds. Together they are 20 cells; a terminal too narrow
    // for that is too narrow for the grid underneath, and a confirmation with
    // no stated way out is the defect this row exists to remove.
    chunks.push(vec![
        Span::styled("[", label),
        Span::styled("Esc", key_hi),
        Span::styled("] cancel", label),
    ]);
    let shed: Vec<usize> = skipped_at
        .into_iter()
        .chain(std::iter::once(count_at))
        .chain(interrupted_at)
        .collect();
    chunk_row(&chunks, keybar_width, &shed, label)
}

pub(super) fn kill_confirm_row(
    app: &App,
    accent: Color,
    key_hi: Style,
    label: Style,
    keybar_width: u16,
) -> Line<'static> {
    let Some(ec) = &app.kill_confirm else {
        return Line::from(vec![Span::styled("Kill ?", label)]);
    };
    let host = app
        .panels
        .get(ec.panel)
        .map_or("?", |p| p.server.host.as_str());
    let (action, key) = match ec.kind {
        crate::app::ExecKind::Kill => ("Kill", "K"),
        crate::app::ExecKind::Journal => ("Journal", "O"),
        crate::app::ExecKind::Renice => ("Renice", "R"),
    };
    let target = format!("{action} {host}:{}:{}", ec.pid, ec.name);
    let mut chunks: Vec<Vec<Span<'static>>> = vec![vec![Span::styled(
        target,
        Style::default()
            .fg(accent)
            .add_modifier(ratatui::style::Modifier::BOLD),
    )]];
    chunks.push(vec![
        Span::styled("[", label),
        Span::styled(key, key_hi),
        Span::styled(format!("] {}", action.to_lowercase()), label),
    ]);
    chunks.push(vec![
        Span::styled("[", label),
        Span::styled("Esc", key_hi),
        Span::styled("] cancel", label),
    ]);
    let shed = vec![0usize];
    chunk_row(&chunks, keybar_width, &shed, label)
}

/// The confirmation that replaces the keybar once Esc/q/Ctrl-C asked to quit
/// while upgrades were in flight. Names the hosts, states the cost, and gives
/// the two ways out.
pub(super) fn quit_confirm_row(
    app: &App,
    key_hi: Style,
    label: Style,
    keybar_width: u16,
) -> Line<'static> {
    let hosts = app.running_upgrade_hosts();
    let n = hosts.len();
    let mut chunks: Vec<Vec<Span<'static>>> = vec![vec![Span::styled(
        format!("{n} upgrade{} running", if n == 1 { "" } else { "s" }),
        Style::default().fg(Color::Yellow),
    )]];
    // The host list is the first thing to go: it is long, and every one of
    // those names is already on the grid behind this row. The count is not --
    // it is what says the quit has a cost at all.
    let mut shed = Vec::new();
    let host_list = hosts.join(", ");
    if !host_list.is_empty() {
        shed.push(chunks.len());
        chunks.push(vec![Span::styled(format!("\u{b7} {host_list}"), label)]);
    }
    chunks.push(vec![
        Span::styled("[", label),
        Span::styled("Q", key_hi),
        Span::styled("] quit anyway", label),
    ]);
    chunks.push(vec![
        Span::styled("[", label),
        Span::styled("Esc", key_hi),
        Span::styled("] stay", label),
    ]);
    chunk_row(&chunks, keybar_width, &shed, label)
}
