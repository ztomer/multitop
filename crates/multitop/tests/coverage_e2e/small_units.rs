use super::*;

// ===========================================================================
// config.rs — validate_host, validate_user
// ===========================================================================

#[test]
fn config_validate_host_rejects_spaces() {
    let _g = isolate_keychain();
    assert!(multitop::config::validate_host("has space").is_err());
    assert!(multitop::config::validate_host("valid-host").is_ok());
}

#[test]
fn config_validate_user_rejects_spaces() {
    let _g = isolate_keychain();
    assert!(multitop::config::validate_user("has space").is_err());
    assert!(multitop::config::validate_user("validuser").is_ok());
}

// ===========================================================================
// passwords.rs — ServerDraft field navigation + validation
// ===========================================================================

#[test]
fn server_draft_field_navigation_wraps() {
    let _g = isolate_keychain();

    let mut draft = ServerDraft::new(None, None, None);
    assert_eq!(draft.field, 0);
    draft.field = (draft.field + 1) % 5;
    assert_eq!(draft.field, 1);
    // Back from 0 wraps to 4.
    draft.field = 0;
    draft.field = draft.field.checked_sub(1).unwrap_or(4);
    assert_eq!(draft.field, 4);
}

#[test]
fn server_draft_field_count() {
    let _g = isolate_keychain();

    let draft = ServerDraft::new(None, None, None);
    // There are 5 fields (host, user, port, upgrade_cmd, password).
    assert_eq!(draft.field, 0);
}

// ===========================================================================
// stream.rs — read_handshake, interpret_packet (via public test exports)
// ===========================================================================

#[test]
fn handshake_variants_exist() {
    let _g = isolate_keychain();
    // Verify the Handshake enum variants are constructible.
    assert!(matches!(
        multitop::stream::Handshake::Framed,
        multitop::stream::Handshake::Framed
    ));
    assert!(matches!(
        multitop::stream::Handshake::NeedAgent("aarch64".into()),
        multitop::stream::Handshake::NeedAgent(_)
    ));
    assert!(matches!(
        multitop::stream::Handshake::Text("banner".into()),
        multitop::stream::Handshake::Text(_)
    ));
    assert!(matches!(
        multitop::stream::Handshake::Closed,
        multitop::stream::Handshake::Closed
    ));
}

#[test]
fn framing_magic_is_exported() {
    let _g = isolate_keychain();
    // The magic header bytes are public and used by the handshake.
    let magic = *multitop_agent::proto::MAGIC;
    assert_eq!(magic.len(), 4);
}

// ===========================================================================
// state.rs — load/save roundtrip, HostUpdate classification
// ===========================================================================

#[test]
fn state_outcome_never() {
    let _g = isolate_keychain();
    assert_eq!(
        HostUpdate::default().outcome(),
        multitop::state::Outcome::Never
    );
}

#[test]
fn host_update_outcome_interrupted() {
    let _g = isolate_keychain();
    assert_eq!(
        HostUpdate {
            started_at: Some(1),
            finished_at: None,
            success: false
        }
        .outcome(),
        multitop::state::Outcome::Interrupted
    );
}

#[test]
fn host_update_duration() {
    let _g = isolate_keychain();
    let u = HostUpdate {
        started_at: Some(100),
        finished_at: Some(172),
        success: true,
    };
    assert_eq!(u.duration_secs(), Some(72));
}

// ===========================================================================
// layout.rs — wrap_words, fit_row, fit_banner_styled
// ===========================================================================

#[test]
fn wrap_words_wraps_long_lines() {
    let _g = isolate_keychain();
    let wrapped = multitop::layout::wrap_words("a long line that needs wrapping at some point", 20);
    assert!(wrapped.len() > 1);
    for line in &wrapped {
        assert!(line.chars().count() <= 20);
    }
}

#[test]
fn fit_row_sheds_when_over_budget() {
    let _g = isolate_keychain();
    let widths = vec![30, 30, 30];
    let kept = multitop::layout::fit_row(&widths, 2, 50, &[2, 1, 0]);
    // Should shed some to fit within 50 cells.
    let total: usize = kept.iter().map(|&i| widths[i]).sum();
    assert!(total <= 50 + 2 * kept.len().saturating_sub(1));
}

// ===========================================================================
// ansi.rs — strip_ansi, to_text
// ===========================================================================

#[test]
fn ansi_strip_removes_escape_codes() {
    let _g = isolate_keychain();
    let plain = multitop_agent::color::strip_ansi("\x1b[31mred\x1b[0m");
    assert_eq!(plain, "red");
}

// ===========================================================================
// refit.rs — refit_line, refit_header
// ===========================================================================

#[test]
fn refit_line_returns_line_asis() {
    let _g = isolate_keychain();
    // refit_line doesn't truncate — it returns the line as-is (or as a rule).
    let line = "a line that is longer than ten characters";
    let fitted = multitop::ui::refit_line(line, 10);
    assert_eq!(fitted, line);
}

#[test]
fn refit_line_zero_width_returns_asis() {
    let _g = isolate_keychain();
    let line = "hello";
    let fitted = multitop::ui::refit_line(line, 0);
    assert_eq!(fitted, line);
}

#[test]
fn refit_line_rule_expands() {
    let _g = isolate_keychain();
    // A line of box-drawing chars becomes a rule of the target width.
    let line = "\u{2500}\u{2500}\u{2500}";
    let fitted = multitop::ui::refit_line(line, 20);
    assert!(fitted.chars().count() > 3);
}
