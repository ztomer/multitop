use super::*;

#[test]
fn a_series_keeps_the_newest_samples_and_drops_the_oldest() {
    let mut s = Series::default();
    assert!(s.is_empty());
    #[allow(clippy::cast_precision_loss)]
    for i in 0..(SAMPLES + 10) {
        s.push(i as f64);
    }
    #[allow(clippy::cast_precision_loss)]
    let want_first = 10.0f64;
    assert_eq!(s.tail(SAMPLES).len(), SAMPLES, "the ring changed size");
    assert_eq!(
        s.tail(SAMPLES).first().copied(),
        Some(want_first),
        "the oldest sample was not the one evicted"
    );
    #[allow(clippy::cast_precision_loss)]
    let want_last = (SAMPLES + 9) as f64;
    assert_eq!(s.latest(), Some(want_last));
}

#[test]
fn a_sample_that_is_not_a_number_cannot_poison_the_scale() {
    // A rate is two counters subtracted. Across a counter reset that arithmetic
    // produces a negative, and across a divide by a zero interval a NaN. Either
    // one reaching `peak` would wreck the autoscale for every later sample.
    let mut s = Series::default();
    s.push(f64::NAN);
    s.push(-500.0);
    s.push(f64::INFINITY);
    s.push(10.0);
    assert_eq!(s.tail(4), vec![0.0, 0.0, 0.0, 10.0]);

    // And through the view, which scales the net graph against its own peak: a
    // NaN reaching that fold makes every later comparison false and the whole
    // graph empty.
    let mut h = History::default();
    h.record(&snapshot(50.0, 50, f64::NAN, -1.0));
    h.record(&snapshot(50.0, 50, 1000.0, 1000.0));
    let text = render_graphs(&h, 20, 9, PLAIN).join("\n");
    assert!(
        text.chars().any(|c| ('\u{2801}'..='\u{28ff}').contains(&c)),
        "a bad sample emptied the graph:\n{text}"
    );
}

#[test]
fn a_tail_longer_than_the_history_returns_what_there_is() {
    // Not padded with zeroes: a graph drawn over invented zeroes reports an
    // idle machine for the minutes before anyone was watching.
    let mut s = Series::default();
    s.push(1.0);
    s.push(2.0);
    assert_eq!(s.tail(50), vec![1.0, 2.0]);
}

#[test]
fn an_empty_series_has_nothing_to_offer_and_says_so() {
    let s = Series::default();
    assert_eq!(s.latest(), None);
    assert_eq!(s.tail(10), Vec::<f64>::new());
}

#[test]
fn a_snapshot_lands_in_all_four_series() {
    let mut h = History::default();
    assert!(h.is_empty());
    h.record(&snapshot(42.0, 25, 1000.0, 2000.0));

    assert!(!h.is_empty());
    assert_eq!(h.cpu.latest(), Some(42.0));
    // Memory comes from `Usage`'s own percent, not a second calculation here.
    assert_eq!(h.mem.latest(), Some(25.0));
    assert_eq!(h.rx.latest(), Some(1000.0));
    assert_eq!(h.tx.latest(), Some(2000.0));
}

// ------------------------------------------------------------------- braille

#[test]
fn a_full_column_is_all_eight_dots() {
    // One sample at full scale, in a one-cell graph: the sample is right
    // aligned, so it fills the right-hand dot column and leaves the left blank.
    // Right column, all four rows = dots 4,5,6,8 = 0x08|0x10|0x20|0x80 = 0xB8.
    assert_eq!(braille_rows(&[100.0], 100.0, 1, 1), vec!["\u{28b8}"]);
}

#[test]
fn two_samples_fill_the_two_dot_columns_of_one_cell() {
    // Left column at half scale is its bottom two dots (7 and 3 = 0x40|0x04);
    // the right column is full (0xB8). Together 0xFC.
    assert_eq!(braille_rows(&[50.0, 100.0], 100.0, 1, 1), vec!["\u{28fc}"]);
}

#[test]
fn nothing_measured_draws_a_blank_cell_rather_than_no_cell() {
    // U+2800 is a braille cell with no dots. A graph of an idle machine is a
    // flat empty line the same width as a busy one -- not a shorter line.
    assert_eq!(braille_rows(&[0.0, 0.0], 100.0, 1, 1), vec!["\u{2800}"]);
}

#[test]
fn the_newest_sample_is_at_the_right_and_the_gap_is_on_the_left() {
    // Two cells, one sample: three of the four dot columns are empty, and the
    // one that is not is the last.
    let rows = braille_rows(&[100.0], 100.0, 2, 1);
    assert_eq!(rows, vec!["\u{2800}\u{28b8}"]);
}

#[test]
fn a_graph_taller_than_one_cell_fills_upward_from_the_bottom() {
    // Two cells tall is eight dot rows. Half scale is four of them, which is
    // exactly the lower cell -- the upper one must be untouched.
    let rows = braille_rows(&[50.0], 100.0, 1, 2);
    assert_eq!(rows, vec!["\u{2800}", "\u{28b8}"]);
}

#[test]
fn a_measurable_but_tiny_value_still_draws_one_dot() {
    // The distinction that matters: a machine doing almost nothing looks
    // different from a machine that has stopped reporting.
    assert_eq!(dots_for(0.01, 100.0, 8), 1);
    assert_eq!(dots_for(0.0, 100.0, 8), 0);
}

#[test]
fn a_value_over_the_scale_is_clamped_rather_than_overflowing_the_cell() {
    assert_eq!(dots_for(500.0, 100.0, 4), 4);
    // And the degenerate scales cannot divide by zero or index off the grid.
    assert_eq!(dots_for(5.0, 0.0, 4), 0);
    assert_eq!(dots_for(5.0, 100.0, 0), 0);
    assert_eq!(dots_for(f64::NAN, 100.0, 4), 0);
}

#[test]
fn a_graph_with_no_room_draws_nothing_rather_than_panicking() {
    assert_eq!(braille_rows(&[50.0], 100.0, 0, 3), Vec::<String>::new());
    assert_eq!(braille_rows(&[50.0], 100.0, 3, 0), Vec::<String>::new());
    // More samples than dot columns: the oldest fall off the left.
    let rows = braille_rows(&[100.0, 0.0, 0.0, 0.0], 100.0, 1, 1);
    assert_eq!(rows, vec!["\u{2800}"], "an old sample was still drawn");
}

// ------------------------------------------------------------------ the view

#[test]
fn a_panel_with_no_samples_says_so_instead_of_drawing_a_flat_line() {
    let out = render_graphs(&History::default(), 40, 9, PLAIN);
    let text = out.join("\n");
    assert!(
        text.contains("no samples yet"),
        "an empty history drew something that looks like data:\n{text}"
    );
}

#[test]
fn the_four_graphs_carry_headings_and_the_current_readings() {
    let mut h = History::default();
    h.record(&snapshot(75.0, 50, 1024.0, 4096.0));
    let text = render_graphs(&h, 40, 20, PLAIN).join("\n");

    assert!(text.contains("CPU"), "{text}");
    assert!(
        text.contains("75%"),
        "the current CPU reading is missing:\n{text}"
    );
    assert!(text.contains("MEM"), "{text}");
    assert!(
        text.contains("50%"),
        "the current MEM reading is missing:\n{text}"
    );
    // Each direction of the link on its own. One combined line could not say
    // which way the traffic was going.
    assert!(
        text.contains("NET \u{2193} down"),
        "no download graph:\n{text}"
    );
    assert!(text.contains("NET \u{2191} up"), "no upload graph:\n{text}");
    // An autoscaled graph with no number on it is a shape that could mean a
    // kilobyte or a gigabit, so the scale is named.
    assert!(
        text.contains("peak"),
        "the net graphs did not say their scale:\n{text}"
    );
}

#[test]
fn the_first_line_is_left_for_the_banner_to_overwrite() {
    // Row 0 of every pane is composed in `ui::draw` from the host name and the
    // scroll badge, over whatever the renderer put there. The CPU heading was
    // on that line and invisible because of it.
    let mut h = History::default();
    h.record(&snapshot(75.0, 50, 0.0, 0.0));

    for rows in [1usize, 2, 4, 9, 20] {
        let out = render_graphs(&h, 40, rows, PLAIN);
        assert_eq!(
            out.first().map(String::as_str),
            Some(""),
            "at {rows} rows the first line was not left for the banner: {out:?}"
        );
        assert!(
            !out.first().unwrap().contains("CPU"),
            "the CPU heading is on the row the banner overwrites"
        );
    }
}

#[test]
fn the_cpu_heading_carries_the_current_clock() {
    let mut h = History::default();
    let mut snap = snapshot(40.0, 50, 0.0, 0.0);
    snap.cpu_mhz = Some(3600.0);
    h.record(&snap);
    let text = render_graphs(&h, 40, 20, PLAIN).join("\n");
    assert!(
        text.contains("3.60 GHz"),
        "no clock on the CPU heading:\n{text}"
    );

    // Under a gigahertz it reads in megahertz rather than as `0.80 GHz`.
    let mut slow = History::default();
    let mut snap = snapshot(40.0, 50, 0.0, 0.0);
    snap.cpu_mhz = Some(800.0);
    slow.record(&snap);
    assert!(render_graphs(&slow, 40, 20, PLAIN)
        .join("\n")
        .contains("800 MHz"));
}

#[test]
fn a_machine_that_publishes_no_clock_says_so_rather_than_showing_zero() {
    // Apple Silicon exposes no current-frequency reading at all. "Not measured"
    // and "idling at nothing" must not look the same.
    let mut h = History::default();
    let mut snap = snapshot(40.0, 50, 0.0, 0.0);
    snap.cpu_mhz = None;
    h.record(&snap);

    let text = render_graphs(&h, 40, 20, PLAIN).join("\n");
    assert!(
        text.contains("-- MHz"),
        "an absent clock was not marked:\n{text}"
    );
    assert!(
        !text.contains("0 MHz"),
        "an absent clock was drawn as zero:\n{text}"
    );
}

#[test]
fn both_directions_of_the_link_share_one_scale() {
    // A link where download dwarfs upload must read as exactly that. Scaling
    // each graph to its own peak would draw them the same height.
    let mut h = History::default();
    h.record(&snapshot(10.0, 10, 1_000_000.0, 1_000.0));
    let out = render_graphs(&h, 20, 20, PLAIN);

    let dots = |from: usize| -> usize {
        out[from + 1..]
            .iter()
            .take_while(|l| !l.contains("NET") && !l.contains("MEM"))
            .map(|l| {
                l.chars()
                    .filter(|c| ('\u{2801}'..='\u{28ff}').contains(c))
                    .count()
            })
            .sum()
    };
    let down_at = out.iter().position(|l| l.contains("down")).expect("down");
    let up_at = out.iter().position(|l| l.contains("up")).expect("up");
    assert!(
        dots(down_at) > dots(up_at),
        "upload was drawn as busy as download:\n{}",
        out.join("\n")
    );
}

#[test]
fn the_graphs_fill_the_rows_they_are_given() {
    let mut h = History::default();
    for i in 0..20 {
        h.record(&snapshot(f64::from(i) * 5.0, 10, 0.0, 0.0));
    }
    for rows in 3..=20usize {
        let out = render_graphs(&h, 30, rows, PLAIN);
        assert!(
            out.len() <= rows,
            "at {rows} rows the graphs drew {} lines and would be clipped",
            out.len()
        );
    }
}

#[test]
fn a_pane_with_room_for_only_one_graph_draws_cpu_properly() {
    // Three headings and nothing else would be three labels and no graph. One
    // real graph beats three useless ones.
    let mut h = History::default();
    h.record(&snapshot(60.0, 10, 0.0, 0.0));
    let out = render_graphs(&h, 30, 4, PLAIN);
    let text = out.join("\n");
    assert!(text.contains("CPU"), "{text}");
    assert!(
        !text.contains("MEM"),
        "three graphs were squeezed in:\n{text}"
    );
    assert!(
        out.len() > 1,
        "the one graph it kept was a heading with no plot:\n{text}"
    );
}

#[test]
fn a_single_row_pane_is_a_heading_and_no_plot() {
    let mut h = History::default();
    h.record(&snapshot(60.0, 10, 0.0, 0.0));
    assert_eq!(render_graphs(&h, 30, 1, PLAIN).len(), 1);
}
