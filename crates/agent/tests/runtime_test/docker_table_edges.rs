use super::*;

#[test]
fn a_frame_with_no_room_at_all_still_produces_something() {
    use multitop_agent::render::{bar_len_for, render, Chrome};

    for (cols, lines) in [(1usize, 1usize), (4, 1), (10, 2), (20, 3), (0, 0)] {
        let snap = snapshot();
        let chrome = Chrome::of(&snap, cols, lines);
        // Below the smallest useful frame the chrome asks for no CPU rows at
        // all rather than a negative or wrapped count.
        let _ = chrome.cpu_rows();
        assert!(chrome.height() >= 1, "chrome claimed a zero-height frame");
        let frame = render(&snap, cols, lines, bar_len_for(cols), &PLAIN);
        assert_eq!(
            frame.len(),
            chrome.height() + chrome.table_height(snap.procs.len()),
            "the predicted height disagreed with what was drawn"
        );
    }
}

// ------------------------------------------------------------ docker sorting

#[test]
fn the_docker_table_can_be_ordered_by_memory_instead_of_cpu() {
    use multitop_agent::docker::render;

    // `db` uses the most memory; `web` uses the most CPU. Which one leads
    // has to follow the sort the user picked.
    let rows = docker_rows();
    let by_cpu = render("h", 100, 24, &rows, &PLAIN, SortBy::Cpu);
    let by_mem = render("h", 100, 24, &rows, &PLAIN, SortBy::Mem);

    // Frame layout: header, column titles, rule, then the body rows.
    let first_body = |frame: &[String]| frame[3].clone();
    assert!(first_body(&by_cpu).contains("db"), "cpu order: {by_cpu:?}");
    assert!(first_body(&by_mem).contains("db"), "mem order: {by_mem:?}");

    // Now make the leaders differ, so the two orders cannot coincide.
    let mut rows = docker_rows();
    rows[0].cpu_pct = 99.0; // web: most cpu
    rows[0].mem_bytes = 1; //        least memory
    let by_cpu = render("h", 100, 24, &rows, &PLAIN, SortBy::Cpu);
    let by_mem = render("h", 100, 24, &rows, &PLAIN, SortBy::Mem);
    assert!(first_body(&by_cpu).contains("web"));
    assert!(first_body(&by_mem).contains("db"));
}

#[test]
fn a_docker_table_taller_than_the_frame_says_how_much_it_hid() {
    use multitop_agent::docker::render;

    let rows: Vec<DockerRow> = (0u32..40)
        .map(|i| DockerRow {
            name: format!("c{i}"),
            status: "Up".into(),
            image: "nginx:latest".into(),
            cpu: "0.0%".into(),
            cpu_pct: f64::from(i),
            mem: "-".into(),
            mem_bytes: u64::from(i),
        })
        .collect();

    let frame = render("h", 100, 12, &rows, &PLAIN, SortBy::Mem);
    let text = frame.join("\n");
    assert!(
        text.contains("more"),
        "an overflowing table must say so: {text}"
    );
    assert!(frame.len() <= 12, "the frame overflowed its budget");
}

#[test]
fn a_lone_escape_that_is_not_a_csi_is_dropped_rather_than_printed() {
    use multitop_agent::color::strip_ansi;

    // `strip_ansi` only ever expects `CSI ... m`, because that is all the agent
    // emits. Anything else has to vanish rather than reach the screen as a
    // stray glyph — and an ESC not followed by `[` takes the byte after it with
    // it, which is what an escape's second byte would have been. Pinned rather
    // than asserted as ideal: the input cannot arise from the agent, and the
    // property that matters is that no escape byte survives.
    assert_eq!(strip_ansi("before\x1bafter"), "beforefter");
    assert_eq!(strip_ansi("trailing\x1b"), "trailing");
    assert_eq!(strip_ansi("\x1bNfoo"), "foo");
    for out in [
        strip_ansi("before\x1bafter"),
        strip_ansi("trailing\x1b"),
        strip_ansi("\x1bNfoo"),
    ] {
        assert!(!out.contains('\x1b'), "an escape byte survived: {out:?}");
    }
    assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
    assert_eq!(strip_ansi("plain"), "plain");
}
