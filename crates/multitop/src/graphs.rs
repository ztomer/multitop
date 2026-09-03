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

/// Where a clock reads better in gigahertz than in megahertz.
const GHZ_IN_MHZ: f64 = 1000.0;

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
///
/// The first line is a placeholder. Row 0 of every pane is the banner, composed
/// in `ui::draw` from the host name and the scroll badge and written over
/// whatever the renderer put there -- which is why the CPU heading vanished the
/// first time round. Every other renderer emits a throwaway first line for the
/// same reason; this one now says so out loud.
#[must_use]
pub fn render_graphs(history: &History, cols: usize, rows: usize, pal: &Palette) -> Vec<String> {
    render_graphs_with_zoom(history, cols, rows, pal, 1)
}

#[must_use]
pub fn render_graphs_with_zoom(
    history: &History,
    cols: usize,
    rows: usize,
    pal: &Palette,
    zoom: u8,
) -> Vec<String> {
    let mut out = vec![String::new()];
    if history.is_empty() {
        out.push(format!(
            "{}\u{2192} Graphs: no samples yet -- the first arrives with the next refresh{}",
            pal.meter_mid(),
            pal.reset
        ));
        return out;
    }
    let body = rows.saturating_sub(1);
    if body == 0 {
        return out;
    }

    let window = cols * DOT_COLS * zoom.max(1) as usize;
    let cpu = history.cpu.tail(window);
    let mem = history.mem.tail(window);
    let rx = history.rx.tail(window);
    let tx = history.tx.tail(window);

    // Four plots: CPU, memory, and each direction of the link on its own. One
    // combined line could not say which way the traffic was going, which is the
    // first thing anyone wants to know about a busy host.
    let each = body / 4;
    let cpu_reading = format!(
        "{} {}",
        pct(history.cpu.latest()),
        clock(history.mhz.latest())
    );
    if each < 2 {
        // No room for four. One graph drawn properly beats four headings with
        // nothing under them.
        return with_head(
            out,
            plot(&cpu_plot(history, &cpu, &cpu_reading, pal), cols, body, pal),
        );
    }
    // The remainder goes to CPU, which is the one people look at.
    let cpu_rows = body - each * 3;

    // One scale for both directions, so a link where transmit dwarfs receive
    // reads as exactly that rather than as two equally busy graphs.
    let net_peak = rx.iter().chain(tx.iter()).copied().fold(0.0f64, f64::max);

    let mut lines = plot(
        &cpu_plot(history, &cpu, &cpu_reading, pal),
        cols,
        cpu_rows,
        pal,
    );
    let mem_reading = pct(history.mem.latest());
    lines.extend(plot(
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
    // on it is a shape that could mean a kilobyte or a gigabit. Both directions
    // name the same scale, which is what makes them comparable by eye.
    let scale = format!("peak {}/s", rate(net_peak));
    let down_reading = format!("{}/s", rate(history.rx.latest().unwrap_or(0.0)));
    lines.extend(plot(
        &Plot {
            heading: &format!("NET \u{2193} down, {scale}"),
            reading: &down_reading,
            samples: &rx,
            max: net_peak,
            colour: pal.primary(),
        },
        cols,
        each,
        pal,
    ));
    let up_reading = format!("{}/s", rate(history.tx.latest().unwrap_or(0.0)));
    lines.extend(plot(
        &Plot {
            heading: &format!("NET \u{2191} up, {scale}"),
            reading: &up_reading,
            samples: &tx,
            max: net_peak,
            colour: pal.secondary(),
        },
        cols,
        each,
        pal,
    ));
    with_head(out, lines)
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

fn with_head(mut head: Vec<String>, body: Vec<String>) -> Vec<String> {
    head.extend(body);
    head
}

fn pct(value: Option<f64>) -> String {
    value.map_or_else(|| "--".to_string(), |v| format!("{v:.0}%"))
}

/// The current core clock, or a dash.
///
/// A dash rather than a zero or an omission: Apple Silicon publishes no
/// current-frequency reading at all, and "not measured" and "idling at nothing"
/// must not look the same.
fn clock(mhz: Option<f64>) -> String {
    let Some(mhz) = mhz else {
        return "-- MHz".to_string();
    };
    if mhz >= GHZ_IN_MHZ {
        format!("{:.2} GHz", mhz / GHZ_IN_MHZ)
    } else {
        format!("{mhz:.0} MHz")
    }
}

fn rate(bytes_per_sec: f64) -> String {
    multitop_agent::fmt::fmt_rate(bytes_per_sec)
}
