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
        custom_command: None,
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
        custom_command: None,
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
        custom_command: None,
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
        custom_command: None,
    });
    p.show_body(vec!["line1".into(), "line2".into()]);
    assert_eq!(p.view[0], "");
    assert_eq!(p.view[1], "line1");
}

#[test]
fn a_ring_seeded_from_an_empty_vec_still_accepts_lines() {
    let mut ring = multitop::panel::RingLines::from(Vec::new());
    ring.push("first".to_string());
    assert_eq!(ring.len(), 1);
    assert_eq!(ring.last().map(String::as_str), Some("first"));
}

#[test]
fn a_short_fixture_does_not_cap_the_log_at_its_own_length() {
    let mut ring = multitop::panel::RingLines::from(vec!["seed".to_string()]);
    for i in 0..100 {
        ring.push(format!("line {i}"));
    }
    assert_eq!(ring.len(), 101);
    assert_eq!(ring.get(0).map(String::as_str), Some("seed"));
}

#[test]
fn the_oldest_line_is_what_falls_off_when_the_cap_is_reached() {
    let mut ring = multitop::panel::RingLines::new(3);
    for i in 0..5 {
        ring.push(format!("{i}"));
    }
    let got: Vec<&str> = ring.iter().map(String::as_str).collect();
    assert_eq!(got, vec!["2", "3", "4"]);
    assert_eq!(ring.last().map(String::as_str), Some("4"));
}

#[test]
fn a_window_over_a_wrapped_ring_is_in_order() {
    let mut ring = multitop::panel::RingLines::new(4);
    for i in 0..6 {
        ring.push(format!("{i}"));
    }
    let got: Vec<&str> = ring.slice(1, 2).map(String::as_str).collect();
    assert_eq!(got, vec!["3", "4"]);
    assert!(ring.slice(9, 3).next().is_none());
}

#[test]
fn shrinking_the_cap_keeps_the_newest_lines() {
    let mut ring = multitop::panel::RingLines::new(10);
    for i in 0..6 {
        ring.push(format!("{i}"));
    }
    ring.set_cap(3);
    let got: Vec<&str> = ring.iter().map(String::as_str).collect();
    assert_eq!(got, vec!["3", "4", "5"]);
    ring.push("6".to_string());
    assert_eq!(ring.len(), 3);
}

#[test]
fn a_notice_lands_in_notes_whatever_view_is_showing() {
    let mut p = multitop::panel::Panel::new(multitop::config::Server {
        host: "web-01".into(),
        port: 22,
        user: "admin".into(),
        upgrade_cmd: None,
        custom_command: None,
    });
    p.note("while monitoring".to_string());
    assert!(p.notes.iter().any(|l| l == "while monitoring"));
    assert!(!p.view.iter().any(|l| l == "while monitoring"));

    p.mode = multitop::panel::Mode::Upgrade;
    p.note("while upgrading".to_string());
    assert!(p.notes.iter().any(|l| l == "while upgrading"));
    assert!(!p.last_upgrade.iter().any(|l| l == "while upgrading"));

    p.mode = multitop::panel::Mode::Monitor;
    p.note("while upgrading".to_string());
    assert_eq!(
        p.notes.iter().filter(|l| *l == "while upgrading").count(),
        1
    );
}
