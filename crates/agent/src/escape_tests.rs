//! The shared tokenizer, against the escape forms tools write.
use super::*;

use multitop_testassert::ESCAPE_FORMS as FORMS;

#[test]
fn strip_removes_every_form_whole() {
    for form in FORMS {
        assert_eq!(strip(&format!("a{form}b")), "ab", "{form:?}");
    }
}

#[test]
fn strip_keeps_layout_controls_and_unicode() {
    assert_eq!(strip("a\tb\r\nc"), "a\tb\r\nc");
    assert_eq!(
        strip("\x1b[0;36m\u{ff48}\u{2500}\x1b[0m"),
        "\u{ff48}\u{2500}"
    );
}

#[test]
fn a_sequence_cut_off_is_incomplete_and_its_bytes_are_not_text() {
    for form in FORMS {
        let bytes = form.as_bytes();
        for cut in 1..bytes.len() {
            let head = &bytes[..cut];
            assert!(!is_complete(head), "{form:?} cut at {cut}");
            let mut printed = String::new();
            scan(head, |t| {
                if let Token::Print(c) = t {
                    printed.push(c);
                }
            });
            assert_eq!(printed, "", "{form:?} cut at {cut}");
        }
        assert!(is_complete(bytes), "{form:?} whole");
    }
    assert!(!is_complete(
        "\u{2500}".as_bytes().get(..2).unwrap_or_default()
    ));
    assert!(is_complete(b""));
    assert!(is_complete(b"plain"));
}

#[test]
fn csi_parameters_default_like_a_terminal() {
    let mut seen = Vec::new();
    scan(b"\x1b[A\x1b[0A\x1b[3A\x1b[m", |t| {
        if let Token::Csi(c) = t {
            seen.push((c.action, c.count(), c.is_sgr()));
        }
    });
    assert_eq!(
        seen,
        [
            ('A', 1, false),
            ('A', 1, false),
            ('A', 3, false),
            ('m', 1, true)
        ]
    );
}

#[test]
fn colon_form_colour_reads_like_the_semicolon_form() {
    let mut seen = Vec::new();
    scan(
        b"\x1b[38:2::1:2:3m\x1b[38:2:1:2:3m\x1b[48:5:9m\x1b[1;38;2;1;2;3m",
        |t| {
            if let Token::Csi(c) = t {
                seen.push(c.sgr_params());
            }
        },
    );
    assert_eq!(
        seen,
        [
            vec![38, 2, 1, 2, 3],
            vec![38, 2, 1, 2, 3],
            vec![48, 5, 9],
            vec![1, 38, 2, 1, 2, 3]
        ]
    );
}

#[test]
fn osc_parameters_arrive_split_with_either_terminator() {
    for reply in [&b"x\x1b]11;rgb:1/2/3\x07"[..], b"x\x1b]11;rgb:1/2/3\x1b\\"] {
        let mut seen = Vec::new();
        scan(reply, |t| {
            if let Token::Osc(params) = t {
                seen.push(params.iter().map(|p| p.to_vec()).collect::<Vec<_>>());
            }
        });
        assert_eq!(
            seen,
            [vec![b"11".to_vec(), b"rgb:1/2/3".to_vec()]],
            "{reply:?}"
        );
    }
}

#[test]
fn sgr_round_trips_through_write_to() {
    for form in [
        "\x1b[0m",
        "\x1b[1;31m",
        "\x1b[38;5;196m",
        "\x1b[38:2:0:1:2:3m",
        "\x1b[?25l",
    ] {
        let mut out = String::new();
        scan(form.as_bytes(), |t| {
            if let Token::Csi(c) = t {
                c.write_to(&mut out);
            }
        });
        assert_eq!(out, form);
    }
}
