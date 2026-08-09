//! Edges the ordinary paths never reach: a zero width, a key pressed with
//! nothing open to receive it, a tie in a sort, a pool that ran out.
//!
//! Small branches, but each is a `return` or a `continue` that only runs when
//! something is degenerate — which is exactly when a panic or a wrong answer
//! is least welcome and least likely to be noticed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::KeyCode;
use multitop::app::App;
use multitop::config::Server;
use multitop::layout::share_width;
use multitop::password_store;
use multitop::passwords::{handle_key, open, PasswordAction};
use multitop_agent::color::PLAIN;
use multitop_agent::docker::{render, Row};
use multitop_agent::SortBy;

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: Some("true".to_string()),
    }
}

async fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

// ------------------------------------------------------ the settings editor

#[tokio::test]
async fn a_key_pressed_with_settings_closed_does_nothing() {
    // The dispatcher checks before touching the manager, so a key that arrives
    // after the screen has closed cannot unwrap a `None`.
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    assert!(app.password_manager.is_none());
    assert!(matches!(
        handle_key(&mut app, KeyCode::Char('x')),
        PasswordAction::None
    ));
}

#[tokio::test]
async fn typing_into_the_password_field_accumulates_and_corrects() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    open(&mut app, 0, false);
    // Open the row for editing, which is what puts a field on screen.
    handle_key(&mut app, KeyCode::Enter);

    for c in "hunter2".chars() {
        handle_key(&mut app, KeyCode::Char(c));
    }
    handle_key(&mut app, KeyCode::Backspace);
    // Keys the field has no use for leave it alone rather than being swallowed
    // by the row navigation underneath.
    handle_key(&mut app, KeyCode::Insert);
    handle_key(&mut app, KeyCode::F(5));

    // Whatever the field holds, none of that may have escaped the editor.
    assert!(
        app.password_manager.is_some(),
        "the editor closed on a stray key"
    );
}

#[tokio::test]
async fn answering_a_deletion_nobody_asked_about_does_nothing() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    open(&mut app, 0, false);

    // No pending confirmation: `y` is just a key.
    assert!(matches!(
        handle_key(&mut app, KeyCode::Char('y')),
        PasswordAction::None
    ));
    assert!(app.password_manager.is_some());
}

// ----------------------------------------------------------- the vault state

#[tokio::test]
async fn dismissing_a_prompt_that_is_not_up_leaves_the_state_alone() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    let before = app.vault_epoch;

    app.set_show_vault_password_prompt(false);
    assert!(!app.show_vault_password_prompt());
    assert_eq!(
        app.vault_epoch, before,
        "a no-op retired an in-flight attempt"
    );

    // Asking for it twice is also a no-op rather than a second prompt.
    app.set_show_vault_password_prompt(true);
    app.set_vault_password_error(Some("wrong password".into()));
    app.set_show_vault_password_prompt(true);
    assert_eq!(
        app.vault_password_error().map(String::as_str),
        Some("wrong password"),
        "asking again cleared the reason the user is being asked"
    );
}

#[tokio::test]
async fn a_second_create_attempt_while_one_is_running_is_refused() {
    // Argon2id is already running; starting another would race two writes to
    // the same file.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(vec![test_server("alpha")]);
    app.config_path = Some(dir.path().join("config.toml"));
    assert!(app.begin_vault_creation());

    app.vault_password_input_mut().push_str("hunter2");
    assert!(app.begin_vault_create_attempt().is_some());
    assert!(app.vault_create_in_flight());
    assert!(
        app.begin_vault_create_attempt().is_none(),
        "a second attempt started while the first was still running"
    );
}

// ------------------------------------------------------------------- layout

#[tokio::test]
async fn wrapping_to_a_width_of_zero_yields_nothing_rather_than_looping() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    assert_eq!(
        multitop::layout::wrap_words("some text here", 0),
        [] as [std::string::String; 0]
    );
    assert_eq!(
        multitop::layout::wrap_words("", 10),
        [] as [std::string::String; 0]
    );
}

#[tokio::test]
async fn a_grid_with_no_room_left_to_share_still_returns_a_row_per_panel() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    // The flexible rows ask for more than there is; whoever is left when the
    // pool empties gets nothing rather than a wrapped negative.
    for panels in 1usize..=12 {
        for height in [1u16, 2, 3, 5, 8, 13, 40] {
            let (areas, _) =
                multitop::ui::regions(ratatui::layout::Rect::new(0, 0, 80, height), panels);
            assert_eq!(areas.len(), panels, "panels={panels} height={height}");
            // A grid, so panels share rows — what has to hold is that no pane
            // is placed outside the screen it is drawn on.
            for a in &areas {
                assert!(
                    a.y + a.height <= height,
                    "panels={panels} height={height}: a pane runs off the bottom: {a:?}"
                );
                assert!(
                    a.x + a.width <= 80,
                    "panels={panels}: a pane runs off the side: {a:?}"
                );
            }
        }
    }
}

// ------------------------------------------------------------------- refit

#[tokio::test]
async fn a_header_with_no_rule_in_it_is_left_as_it_was() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    // Nothing to refit: the caller keeps the line it had.
    assert_eq!(multitop::refit::refit_header("plain text", 80), None);
    // A rule but no name is the same answer.
    assert_eq!(
        multitop::refit::refit_header("\u{2500}\u{2500}\u{2500}", 80),
        None
    );
}

#[tokio::test]
async fn a_header_wider_than_the_pane_drops_its_rules_rather_than_the_name() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    let header = format!(
        "\u{2500}\u{2500} {} \u{2500}\u{2500}",
        multitop_agent::fmt::fullwidth("web-01")
    );
    // Narrower than the name: no room for a rule either side, so the name is
    // all that comes back.
    let tight = multitop::refit::refit_header(&header, 4).expect("a header must be produced");
    assert!(
        !tight.contains('\u{2500}'),
        "a rule was drawn with no room for it: {tight:?}"
    );
    assert!(
        tight.contains('\u{FF57}'),
        "the name was dropped: {tight:?}"
    );

    // Exactly the name's width, and one under the two-space budget: both take
    // the same "name only" answer.
    for cols in [11, 12, 13] {
        let out = multitop::refit::refit_header(&header, cols).expect("a header");
        assert!(out.contains('\u{FF57}'), "cols={cols}: {out:?}");
    }

    // Wide enough, and the rules come back.
    let roomy = multitop::refit::refit_header(&header, 60).expect("a header");
    assert!(
        roomy.contains('\u{2500}'),
        "no rule at a roomy width: {roomy:?}"
    );
}

// ------------------------------------------------------------- ssh commands

#[tokio::test]
async fn the_cleanup_command_keeps_the_agents_this_build_ships() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    let cmd = multitop::ssh::cleanup_old_agents_command();

    let kept: Vec<String> = [multitop::ssh::Arch::X86_64, multitop::ssh::Arch::Aarch64]
        .iter()
        .map(|a| a.hash().to_string())
        .filter(|h| h != "missing" && !h.is_empty())
        .collect();

    // The invariant, and the reason this test was rewritten: a catch-all delete
    // must never appear without something to keep.
    //
    // It used to assert the command *always* contained `agent-*) rm -f`, which
    // is the dangerous shape stated as the requirement. In a build with no
    // agent embedded -- a plain `cargo build` rather than `./build.sh` -- both
    // hashes are "missing", every keep-arm is filtered out, and what was left
    // was a loop that deleted every agent in the cache including the one in
    // use. That was unreachable only because the sweep runs after a successful
    // upload and an upload needs an embedded agent, so the safety lived in a
    // different function's ordering. The test asserted the landmine was there.
    if kept.is_empty() {
        assert!(
            !cmd.contains("rm -f"),
            "a build that ships no agent generated a command that deletes them:\n{cmd}"
        );
        return;
    }

    // Structure, folded in from a weaker duplicate in `app_test.rs` that
    // asserted `rm -f` unconditionally -- with a comment noting that the keep
    // list "may be empty" in a debug build, which is precisely the case where
    // that assertion demands the destructive command.
    assert!(cmd.starts_with("cd ~/.cache/multitop"), "{cmd}");
    assert!(cmd.contains("for f in agent-*"), "{cmd}");
    assert!(cmd.contains("case") && cmd.contains("esac"), "{cmd}");
    assert!(cmd.contains("done"), "{cmd}");
    assert!(cmd.contains("agent-*) rm -f"), "{cmd}");
    // Every hash this build has is spared.
    for hash in &kept {
        assert!(
            cmd.contains(&format!("agent-{hash}) continue")),
            "the current agent {hash} would be deleted:\n{cmd}"
        );
    }
}

// ------------------------------------------------------------ docker sorting

#[tokio::test]
async fn a_memory_tie_is_broken_by_cpu_and_then_by_name() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;

    let row = |name: &str, cpu: f64, mem: u64| Row {
        name: name.into(),
        status: "Up".into(),
        image: "nginx:latest".into(),
        cpu: format!("{cpu:.1}%"),
        cpu_pct: cpu,
        mem: "-".into(),
        mem_bytes: mem,
    };
    // All three use the same memory, so the order is decided by CPU; the last
    // two also tie on CPU, so their names decide.
    let rows = vec![
        row("charlie", 1.0, 1000),
        row("bravo", 1.0, 1000),
        row("alpha", 9.0, 1000),
    ];

    let frame = render("h", 100, 24, &rows, &PLAIN, SortBy::Mem);
    let body: Vec<&String> = frame
        .iter()
        .filter(|l| l.contains("alpha") || l.contains("bravo") || l.contains("charlie"))
        .collect();
    assert_eq!(body.len(), 3, "{frame:?}");
    assert!(
        body[0].contains("alpha"),
        "cpu did not break the memory tie: {body:?}"
    );
    assert!(
        body[1].contains("bravo"),
        "name did not break the cpu tie: {body:?}"
    );
    assert!(body[2].contains("charlie"), "{body:?}");
}

// ------------------------------------------------------ the rotation prompt

#[tokio::test]
async fn the_rotation_prompt_takes_two_passwords_in_turn() {
    // `r` asks for the current password, then the new one. Neither is checked
    // here: verifying means running the KDF, which belongs off the event loop.
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("vault.bin");
    // A vault file only has to exist for `r` to be offered.
    std::fs::write(&vault_path, b"placeholder").unwrap();

    let mut app = App::new(vec![test_server("alpha")]);
    app.config_path = Some(dir.path().join("config.toml"));
    app.vault = Some(std::sync::Arc::new(multitop_vault::Vault::new(
        multitop::vault::config_for(vault_path),
    )));
    open(&mut app, 0, false);

    handle_key(&mut app, KeyCode::Char('r'));
    for c in "oldd".chars() {
        handle_key(&mut app, KeyCode::Char(c));
    }
    // A typo is corrected in place rather than by starting over.
    handle_key(&mut app, KeyCode::Backspace);
    // Keys the field has no use for leave it alone.
    handle_key(&mut app, KeyCode::Up);
    assert!(matches!(
        handle_key(&mut app, KeyCode::Enter),
        PasswordAction::None
    ));

    for c in "new-master".chars() {
        handle_key(&mut app, KeyCode::Char(c));
    }
    let action = handle_key(&mut app, KeyCode::Enter);
    let PasswordAction::RotateVaultPassword { current, new } = action else {
        panic!("the second Enter must start the rotation, got {action:?}");
    };
    assert_eq!(
        current, "old",
        "the corrected first password was not carried"
    );
    assert_eq!(new, "new-master");
}

#[tokio::test]
async fn an_empty_answer_to_the_rotation_prompt_changes_nothing() {
    let _g = isolate().await;
    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("vault.bin");
    std::fs::write(&vault_path, b"placeholder").unwrap();

    let mut app = App::new(vec![test_server("alpha")]);
    app.config_path = Some(dir.path().join("config.toml"));
    app.vault = Some(std::sync::Arc::new(multitop_vault::Vault::new(
        multitop::vault::config_for(vault_path),
    )));
    open(&mut app, 0, false);

    handle_key(&mut app, KeyCode::Char('r'));
    assert!(matches!(
        handle_key(&mut app, KeyCode::Enter),
        PasswordAction::None
    ));
    let notice = app
        .password_manager
        .as_ref()
        .and_then(|m| m.notice.clone())
        .unwrap_or_default();
    assert!(notice.contains("not changed"), "{notice}");

    // Enter with nothing staged at all is also inert rather than a panic.
    assert!(matches!(
        handle_key(&mut app, KeyCode::Enter),
        PasswordAction::None
    ));
}

#[tokio::test]
async fn rotation_is_offered_only_when_there_is_a_vault_and_no_run_in_flight() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    open(&mut app, 0, false);

    handle_key(&mut app, KeyCode::Char('r'));
    let notice = app
        .password_manager
        .as_ref()
        .and_then(|m| m.notice.clone())
        .unwrap_or_default();
    assert!(notice.contains("No vault to rotate"), "{notice}");

    // And while one is running, `r` says so rather than starting a second.
    app.password_manager.as_mut().unwrap().rotating = true;
    handle_key(&mut app, KeyCode::Char('r'));
    let notice = app
        .password_manager
        .as_ref()
        .and_then(|m| m.notice.clone())
        .unwrap_or_default();
    assert!(notice.contains("already being changed"), "{notice}");
}

// -------------------------------------------------------------- config saves

#[tokio::test]
async fn writing_the_server_list_creates_the_directory_it_needs() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    // First run: the config directory does not exist yet, and refusing to make
    // it would mean the settings screen could not save anything.
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("nested").join("deeper").join("config.toml");
    assert!(!config.parent().unwrap().exists());

    multitop::config::save_servers(
        &config,
        &[Server {
            host: "web-01".into(),
            port: 2222,
            user: "root".into(),
            upgrade_cmd: None,
        }],
    )
    .expect("the directory must be created");

    let written = std::fs::read_to_string(&config).unwrap();
    assert!(written.contains("web-01"), "{written}");
    assert!(written.contains("2222"), "{written}");
}

// ----------------------------------------------------------- width sharing

#[tokio::test]
async fn a_surplus_too_small_to_go_round_leaves_the_last_cells_at_their_minimum() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;

    // Two spare cells, three cells that want more: the first two take one each
    // and the third gets nothing. Handing it a share anyway would overrun the
    // budget the caller has to draw inside.
    let out = share_width(0, &[0, 0, 0], &[10, 10, 10], 2);
    assert_eq!(
        out.iter().sum::<usize>(),
        2,
        "the budget was overrun: {out:?}"
    );
    assert_eq!(out.iter().filter(|w| **w == 0).count(), 1, "{out:?}");

    // No surplus at all: everyone gets their minimum and nothing more.
    assert_eq!(share_width(0, &[2, 3], &[10, 10], 5), vec![2, 3]);
    // Not even the minimum fits: the row is allowed to be wider than the
    // terminal rather than losing the alignment with its own header.
    assert_eq!(share_width(0, &[4, 4], &[10, 10], 3), vec![4, 4]);
    // Nothing flexible at all.
    assert_eq!(share_width(0, &[], &[], 80), [] as [usize; 0]);
}

#[tokio::test]
async fn a_cell_that_wants_little_takes_little_and_leaves_the_rest() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;

    // A short `user` column must not hoard space a long `upgrade command`
    // could use.
    let out = share_width(0, &[1, 1], &[2, 100], 40);
    assert_eq!(out[0], 2, "a cell took more than it asked for: {out:?}");
    assert!(out[1] > 30, "the surplus was not handed on: {out:?}");
    assert_eq!(out.iter().sum::<usize>(), 40);
}

// ------------------------------------------------------------- pane lookup

#[tokio::test]
async fn asking_for_a_pane_that_is_not_there_yields_nothing() {
    // A stale index from a task started for the previous panel list.
    let _g = isolate().await;
    let app = App::new(vec![test_server("alpha")]);
    let (lines, offset) = multitop::ui::pane_lines(&app, 9, 20, 80, 0);
    assert_eq!(lines, [] as [std::string::String; 0]);
    assert_eq!(offset, 0);
}

// ------------------------------------------------------- refitting a header

#[tokio::test]
async fn a_two_word_host_name_is_measured_by_cells_not_by_characters() {
    // The mock store is process-global, so every test in this file holds
    // the same guard for its whole body — including the ones that only
    // touch the filesystem, so none can run while another has diverted it.
    let _g = isolate().await;
    // The name is fullwidth, but the space between the words is not, so the
    // width is not simply twice the character count — measuring it that way
    // puts the rule in the wrong place.
    let name = multitop_agent::fmt::fullwidth("web 01");
    let header = format!("\u{2500}\u{2500} {name} \u{2500}\u{2500}");

    let out = multitop::refit::refit_header(&header, 60).expect("a header must be produced");
    let visible = multitop_agent::color::strip_ansi(&out);
    let cells: usize = visible
        .chars()
        .map(|c| usize::from((0xFF01..=0xFF5E).contains(&(c as u32))) + 1)
        .sum();
    assert_eq!(
        cells, 60,
        "the refitted header does not fill its pane: {visible:?}"
    );
}
