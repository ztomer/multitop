//! The `G` view: CPU, memory and network drawn as braille area graphs.
//!
//! Braille gives four times the vertical resolution and twice the horizontal
//! resolution of block characters, in one cell, in a font every terminal
//! already has. A pane four rows tall becomes a graph sixteen steps deep,
//! which is the difference between a shape and a staircase.
//!
//! The graphs are drawn from `history::History`, which every panel fills from
//! the Monitor packets it is already receiving. Nothing here talks to the
//! agent.

use multitop_agent::color::Palette;

use crate::history::History;

/// Dot columns per braille cell, and dot rows per braille cell.
const DOT_COLS: usize = 2;
const DOT_ROWS: usize = 4;

/// The braille block starts here; the low eight bits of the offset are the
/// dots.
const BRAILLE_BASE: u32 = 0x2800;

/// Bit for each dot, indexed `[column][row]`.
///
/// Braille numbers its dots 1-6 down the two columns and adds 7 and 8 at the
/// bottom afterwards, which is why the fourth row is not `0x08` and `0x80` is
/// not where the pattern suggests. Writing the table out is the only way this
/// reads correctly.
const DOT_BITS: [[u8; DOT_ROWS]; DOT_COLS] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

/// How many dots tall a bar is, for `value` against a full scale of `max`.
///
/// A sample that is present but tiny still gets one dot: a graph that draws
/// nothing for a machine doing a little work is indistinguishable from one
/// that has lost its connection, and those are very different things.
#[must_use]
pub fn dots_for(value: f64, max: f64, dot_rows: usize) -> usize {
    if dot_rows == 0 || max <= 0.0 || !value.is_finite() || value <= 0.0 {
        return 0;
    }
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    #[allow(clippy::cast_sign_loss)]
    let scaled = ((value / max) * dot_rows as f64).ceil() as usize;
    scaled.clamp(1, dot_rows)
}

/// An area graph as `height` rows of braille, `width` cells wide.
///
/// The newest sample is at the right, which is the direction every other
/// history display in this program runs. Fewer samples than the graph is wide
/// leaves the left end blank rather than stretching what there is: a stretched
/// graph claims history the panel does not have.
#[must_use]
pub fn braille_rows(samples: &[f64], max: f64, width: usize, height: usize) -> Vec<String> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let dot_rows = height * DOT_ROWS;
    let dot_cols = width * DOT_COLS;
    let shown = &samples[samples.len().saturating_sub(dot_cols)..];
    // Right-aligned: the blank is on the left, where the past would be.
    let pad = dot_cols - shown.len();

    // `[row][cell]` of dot bits, filled from the bottom up.
    let mut cells = vec![vec![0u8; width]; height];
    for (i, value) in shown.iter().enumerate() {
        let dot_col = pad + i;
        let filled = dots_for(*value, max, dot_rows);
        for d in 0..filled {
            // `d` counts up from the bottom of the graph; rows count down from
            // the top, so the two have to be turned around here exactly once.
            let from_top = dot_rows - 1 - d;
            let (row, dot_row) = (from_top / DOT_ROWS, from_top % DOT_ROWS);
            let (cell, dot) = (dot_col / DOT_COLS, dot_col % DOT_COLS);
            if let Some(slot) = cells.get_mut(row).and_then(|r| r.get_mut(cell)) {
                *slot |= DOT_BITS[dot][dot_row];
            }
        }
    }

    cells
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|bits| char::from_u32(BRAILLE_BASE + u32::from(bits)).unwrap_or(' '))
                .collect()
        })
        .collect()
}

/// One labelled graph.
struct Plot<'a> {
    /// What is being measured, and the scale if it is not a percentage.
    heading: &'a str,
    /// The newest value, in words.
    reading: &'a str,
    samples: &'a [f64],
    /// The value a full-height bar stands for.
    max: f64,
    colour: &'a str,
}

/// A heading with the current reading, then the plot underneath it.
fn plot(spec: &Plot<'_>, cols: usize, rows: usize, pal: &Palette) -> Vec<String> {
    let Plot {
        heading,
        reading,
        samples,
        max,
        colour,
    } = *spec;
    let mut out = Vec::with_capacity(rows);
    out.push(format!(
        "{}{heading}{} {}{reading}{}",
        pal.muted(),
        pal.reset,
        colour,
        pal.reset
    ));
    let plot_rows = rows.saturating_sub(1);
    if plot_rows == 0 {
        return out;
    }
    if samples.is_empty() {
        // Rule: an empty state says why, and does not look like data.
        out.push(format!(
            "{}(waiting for the first sample){}",
            pal.muted(),
            pal.reset
        ));
        return out;
    }
    for line in braille_rows(samples, max, cols, plot_rows) {
        out.push(format!("{colour}{line}{}", pal.reset));
    }
    out
}

/// The whole graph view for one panel.
#[must_use]
pub fn render_graphs(history: &History, cols: usize, rows: usize, pal: &Palette) -> Vec<String> {
    if history.is_empty() {
        return vec![format!(
            "{}\u{2192} Graphs: no samples yet -- the first arrives with the next refresh{}",
            pal.meter_mid(),
            pal.reset
        )];
    }

    let window = cols * DOT_COLS;
    let cpu = history.cpu.tail(window);
    let mem = history.mem.tail(window);
    // One line for the link, because two series in one braille grid are one
    // series with extra dots -- there is no second colour inside a cell.
    let rx = history.rx.tail(window);
    let tx = history.tx.tail(window);
    let net: Vec<f64> = rx.iter().zip(tx.iter()).map(|(r, t)| r + t).collect();
    let net_peak = net.iter().copied().fold(0.0f64, f64::max);
    let cpu_reading = pct(history.cpu.latest());

    // Three stacked plots. Whatever does not divide evenly goes to CPU, which
    // is the one people look at.
    let each = rows / 3;
    if each < 2 {
        // Below two rows a plot is a heading and nothing else, so the pane
        // gets one graph properly rather than three uselessly.
        return plot(&cpu_plot(history, &cpu, &cpu_reading, pal), cols, rows, pal);
    }
    let cpu_rows = rows - each * 2;

    let mut out = plot(
        &cpu_plot(history, &cpu, &cpu_reading, pal),
        cols,
        cpu_rows,
        pal,
    );
    let mem_reading = pct(history.mem.latest());
    out.extend(plot(
        &Plot {
            heading: "MEM",
            reading: &mem_reading,
            samples: &mem,
            max: 100.0,
            colour: pal.cpu_bar(history.mem.latest().unwrap_or(0.0)),
        },
        cols,
        each,
        pal,
    ));
    // The heading names the scale, because an autoscaled graph with no number
    // on it is a shape that could mean a kilobyte or a gigabit.
    let net_heading = format!("NET total, peak {}/s", rate(net_peak));
    let net_now = format!(
        "\u{2193}{}/s \u{2191}{}/s",
        rate(history.rx.latest().unwrap_or(0.0)),
        rate(history.tx.latest().unwrap_or(0.0))
    );
    out.extend(plot(
        &Plot {
            heading: &net_heading,
            reading: &net_now,
            samples: &net,
            max: net_peak,
            colour: pal.primary(),
        },
        cols,
        each,
        pal,
    ));
    out
}

/// The CPU plot, named once because both layouts draw it.
///
/// `reading` is owned by the caller: this is on the redraw path, so building a
/// `String` here and handing out a reference to it means either a lifetime the
/// caller cannot satisfy or a leak on every refresh tick.
fn cpu_plot<'a>(
    history: &History,
    samples: &'a [f64],
    reading: &'a str,
    pal: &Palette,
) -> Plot<'a> {
    Plot {
        heading: "CPU",
        reading,
        samples,
        max: 100.0,
        colour: pal.cpu_bar(history.cpu.latest().unwrap_or(0.0)),
    }
}

fn pct(value: Option<f64>) -> String {
    value.map_or_else(|| "--".to_string(), |v| format!("{v:.0}%"))
}

fn rate(bytes_per_sec: f64) -> String {
    multitop_agent::fmt::fmt_rate(bytes_per_sec)
}
