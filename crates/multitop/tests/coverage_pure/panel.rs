// ===========================================================================
// panel.rs — RingLines (public)
// ===========================================================================

#[test]
fn ring_lines_zero_capacity() {
    let mut ring = multitop::panel::RingLines::new(0);
    ring.push("won't be stored".into());
    assert_eq!(ring.len(), 0);
}

#[test]
fn ring_lines_one_capacity() {
    let mut ring = multitop::panel::RingLines::new(1);
    ring.push("first".into());
    ring.push("second".into());
    assert_eq!(ring.len(), 1);
    assert_eq!(ring.last().map(String::as_str), Some("second"));
}

#[test]
fn ring_lines_clear_and_empty() {
    let mut ring = multitop::panel::RingLines::new(5);
    ring.push("a".into());
    ring.clear();
    assert_eq!(ring.len(), 0);
    assert!(ring.is_empty());
}

#[test]
fn ring_lines_get() {
    let mut ring = multitop::panel::RingLines::new(5);
    ring.push("a".into());
    ring.push("b".into());
    assert_eq!(ring.get(0).map(String::as_str), Some("a"));
    assert!(ring.get(9).is_none());
}

#[test]
fn ring_lines_set_cap_shrinks() {
    let mut ring = multitop::panel::RingLines::new(10);
    for i in 0..6 {
        ring.push(format!("line{i}"));
    }
    ring.set_cap(2);
    assert_eq!(ring.len(), 2);
    assert_eq!(ring.get(0).map(String::as_str), Some("line4"));
}

#[test]
fn ring_lines_slice() {
    let mut ring = multitop::panel::RingLines::new(5);
    for i in 0..5 {
        ring.push(format!("{i}"));
    }
    let slice: Vec<&str> = ring.slice(2, 2).map(String::as_str).collect();
    assert_eq!(slice, vec!["2", "3"]);
}

// ===========================================================================
// panel.rs — Panel (public)
// ===========================================================================

#[test]
fn panel_show_last_frame_with_cached_frame() {
    let mut p = multitop::panel::Panel::new(multitop::config::Server {
        host: "h1".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: None,
    });
    p.show_frame(vec!["cached".into()]);
    p.show_last_frame();
    // show_last_frame overwrites view with last_frame (which was set by show_frame).
    assert!(p
        .view
        .iter()
        .any(|l| l.contains("cached") || l.contains("waiting")));
}

#[test]
fn panel_note_dedup() {
    let mut p = multitop::panel::Panel::new(multitop::config::Server {
        host: "h1".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: None,
    });
    p.note("hello".into());
    p.note("hello".into()); // duplicate
    assert_eq!(p.notes.iter().filter(|n| *n == "hello").count(), 1);
}

#[test]
fn panel_note_bounded() {
    let mut p = multitop::panel::Panel::new(multitop::config::Server {
        host: "h1".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: None,
    });
    for i in 0..10 {
        p.note(format!("note{i}"));
    }
    assert!(p.notes.len() <= 4); // MAX_NOTES = 4
}

#[test]
fn panel_show_body_reserves_row0() {
    let mut p = multitop::panel::Panel::new(multitop::config::Server {
        host: "h1".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: None,
    });
    p.show_body(vec!["line1".into(), "line2".into()]);
    assert_eq!(p.view[0], "");
    assert_eq!(p.view[1], "line1");
}
