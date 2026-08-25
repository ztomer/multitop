use super::*;

/// A terminal too small for the panel must clip, not panic.
#[tokio::test]
async fn the_panel_renders_in_a_terminal_too_small_for_it() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a", "host-b"]);
    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Enter);
    h.type_str("secret");

    for (w, hgt) in [(20u16, 5u16), (1, 1), (200, 60), (80, 2)] {
        h.terminal = Terminal::new(TestBackend::new(w, hgt)).unwrap();
        h.draw();
    }
}

/// Removing a server must leave the shorter list renderable and editable.
#[tokio::test]
async fn removing_a_server_leaves_a_renderable_panel() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a", "host-b"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Down);
    h.press(KeyCode::Char('d'));
    h.press(KeyCode::Char('y'));

    assert_eq!(h.app.panels.len(), 1, "{}", h.notice());
    h.press(KeyCode::Enter);
}

// ---------------------------------------------------------------------------
// The structural gate: every short key sequence, drawn.
// ---------------------------------------------------------------------------

/// Keys the Configuration panel binds, plus the ones that edit text.
const SWEEP_KEYS: &[KeyCode] = &[
    KeyCode::Char('a'),
    KeyCode::Char('d'),
    KeyCode::Char('e'),
    KeyCode::Char('i'),
    KeyCode::Char('q'),
    KeyCode::Char('r'),
    KeyCode::Char('s'),
    KeyCode::Char('y'),
    KeyCode::Char('n'),
    KeyCode::Char('x'),
    KeyCode::Tab,
    KeyCode::Enter,
    KeyCode::Esc,
    KeyCode::Up,
    KeyCode::Down,
    KeyCode::Backspace,
];

/// How many presses deep the sweep goes.
///
/// Depth 4 has been run by hand and is clean, but takes minutes -- too slow to
/// sit in front of every commit. Raise it here when hunting, not permanently.
const DEPTH: usize = 3;

/// Every sequence of `DEPTH` presses from the open Configuration panel must
/// produce a frame that renders.
///
/// The class is "a state the panel can reach that the renderer cannot draw",
/// and only walking the reachable states rules it out.
#[tokio::test]
async fn every_short_key_sequence_in_the_panel_renders() {
    let _guard = setup().await;

    let mut sequence = [0usize; DEPTH];
    let total = SWEEP_KEYS.len().pow(u32::try_from(DEPTH).unwrap());
    for n in 0..total {
        let mut rest = n;
        for slot in &mut sequence {
            *slot = rest % SWEEP_KEYS.len();
            rest /= SWEEP_KEYS.len();
        }

        let mut h = Harness::new(&["host-a", "host-b"]);
        h.press(KeyCode::Char('e'));
        for &index in &sequence {
            h.press(SWEEP_KEYS[index]);
            // Answering the vault-creation prompt runs Argon2id sized to a
            // quarter of system RAM. It is covered on its own above; here it
            // would turn a sweep into an out-of-memory hazard, so the offer is
            // declined the moment it appears. Declining is a real key path.
            if h.app.vault_creating() {
                h.app.cancel_vault_creation();
                h.draw();
            }
        }
    }
}

/// The credential state and the way out must survive every width.
///
/// The row was 75 fixed columns before the state cell, so at 80 the Password
/// header read `Pas` and the rows read `✓ S`; at 40 the column was gone. This
/// is the one screen where a host is deleted and its stored password edited,
/// and whether the host HAS one was the thing amputated at the right margin.
/// The hint row had the same defect with worse consequences: the panel paints
/// over the keybar, so `[Esc/Q] Return` was the only exit signage on screen and
/// it was what got shed, leaving an orphaned `[` behind.
#[tokio::test]
async fn the_settings_panel_keeps_state_and_the_exit_at_every_width() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a", "host-b"]);
    password_store::save(&server("host-a"), "stored").unwrap();
    h.app.panels[0].password_saved = true;
    h.press(KeyCode::Char('e'));

    for width in [40u16, 52, 64, 80, 120, 200] {
        h.terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
        h.draw();
        let screen = h.screen();
        assert!(
            screen.contains("\u{2713} Stored"),
            "{width} cols: the credential state must be visible, got:\n{screen}"
        );
        assert!(
            screen.contains("\u{b7} Unset"),
            "{width} cols: so must its absence, got:\n{screen}"
        );
        assert!(
            screen.contains("[Esc/Q] Return"),
            "{width} cols: the way out must never be what is dropped, got:\n{screen}"
        );
        // Nothing half-drawn: every bracket that opens, closes.
        for line in screen.lines() {
            let body = line.trim_matches(|c| c == '\u{2502}' || c == ' ');
            assert_eq!(
                body.matches('[').count(),
                body.matches(']').count(),
                "{width} cols: a hint was sliced: {body:?}"
            );
        }
    }
}

/// The Appearance section is content; the hints are signage about content.
///
/// It was added below the hint block, and on a 12-row panel the four wrapped
/// hint lines pushed it off the bottom -- so the screen showed `[B] Banner
/// style` and not the row `B` changes. That is the width rule met on the
/// vertical axis: a block with no budget crowding out the thing it describes.
#[tokio::test]
async fn appearance_survives_a_panel_too_short_for_its_own_hints() {
    let _guard = setup().await;
    let mut h = Harness::new(&["web-01", "db-02"]);
    h.press(KeyCode::Char('e'));
    h.terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    h.draw();

    let screen = h.screen();
    assert!(
        screen.contains("Appearance"),
        "the section must survive a short panel:\n{screen}"
    );
    assert!(
        screen.contains("Banner"),
        "and so must the row it heads:\n{screen}"
    );
    assert!(
        screen.contains("[Esc/Q] Return"),
        "the way out is never what gets shed:\n{screen}"
    );
}

/// A notice is the app answering something the user just did. Appended after
/// the permanent signage, it was the first thing off the bottom of a short
/// panel -- a result the user cannot see is a result they did not get.
#[tokio::test]
async fn a_notice_outranks_the_hints_for_the_space_there_is() {
    let _guard = setup().await;
    let mut h = Harness::new(&["web-01", "db-02"]);
    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Char('b'));
    h.terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    h.draw();

    let screen = h.screen();
    assert!(
        screen.contains("Banner: Wide"),
        "the answer to the press must be on screen:\n{screen}"
    );
}

/// `b` cycles the banner style, and the row says which one is in force.
#[tokio::test]
async fn b_cycles_the_banner_style_and_the_row_reports_it() {
    let _guard = setup().await;
    let mut h = Harness::new(&["web-01"]);
    h.press(KeyCode::Char('e'));
    h.draw();
    assert!(h.screen().contains("[Plain]"), "{}", h.screen());

    h.press(KeyCode::Char('b'));
    h.draw();
    assert!(h.screen().contains("[Wide]"), "{}", h.screen());
    assert_eq!(h.app.banner_style, multitop::layout::BannerStyle::Wide);

    h.press(KeyCode::Char('b'));
    h.draw();
    assert!(h.screen().contains("[Plain]"), "{}", h.screen());
}

/// A destructive prompt must not lose its own cancel instruction. The keybar
/// confirm row was rebuilt for exactly this and the Settings notice still did
/// it: at 40 columns `Paragraph` cut `[y] confirm  [Esc] cancel` off the end.
#[tokio::test]
async fn a_delete_confirmation_keeps_its_cancel_instruction_when_narrow() {
    let _guard = setup().await;
    let mut h = Harness::new(&["web-01", "db-02"]);
    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Char('d'));
    h.terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    h.draw();

    let screen = h.screen();
    assert!(
        screen.contains("[Esc] cancel"),
        "a destructive prompt keeps its way out at every width:\n{screen}"
    );
    assert!(
        screen.contains("[y] confirm"),
        "and the key that confirms it:\n{screen}"
    );
}
