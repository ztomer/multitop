use super::*;

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
            custom_command: None,
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
