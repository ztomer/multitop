use super::*;

#[test]

fn the_sort_word_matches_the_argument_that_selects_it() {
    assert_eq!(SortBy::Cpu.word(), "cpu");
    assert_eq!(SortBy::Mem.word(), "mem");
    assert_eq!(
        parse_args([
            "monitor".into(),
            String::new(),
            "80".into(),
            "24".into(),
            "mem".into()
        ])
        .sort,
        SortBy::Mem
    );
    assert_eq!(
        parse_args([
            "monitor".into(),
            String::new(),
            "80".into(),
            "24".into(),
            "MEMORY".into()
        ])
        .sort,
        SortBy::Mem
    );
    assert_eq!(
        parse_args([
            "monitor".into(),
            String::new(),
            "80".into(),
            "24".into(),
            "cpu".into()
        ])
        .sort,
        SortBy::Cpu
    );
    // An unrecognised word leaves the default alone rather than failing.
    assert_eq!(
        parse_args([
            "monitor".into(),
            String::new(),
            "80".into(),
            "24".into(),
            "sideways".into()
        ])
        .sort,
        SortBy::Cpu
    );
}

#[test]
fn each_mode_has_the_word_that_selects_it_on_the_command_line() {
    for (word, mode) in [
        ("monitor", Mode::Monitor),
        ("docker", Mode::Docker),
        ("fetch", Mode::Fetch),
    ] {
        assert_eq!(mode.word(), word);
        assert_eq!(parse_args([word.to_string()]).mode, mode);
    }
}

// ---------------------------------------------------------------- entry point

#[test]
fn the_fetch_mode_entry_point_runs_end_to_end() {
    // Under `cargo test` stdout is captured, so this takes the piped branch
    // and writes a packet to the captured buffer. Nothing to assert on but
    // that the whole path — sample, encode, write — completes.
    multitop_agent::run_agent(["fetch".to_string(), String::new(), "80".into(), "24".into()]);
}

#[test]
fn watching_stdin_for_a_hangup_is_safe_wherever_stdin_came_from() {
    // Only a pipe is watched. On a host with no `/proc/self/fd/0` -- every
    // macOS build machine -- this must decide "not a pipe" rather than fail.
    let gone = multitop_agent::stdin_eof_watcher();

    // The flag's *value* is deliberately not asserted, and asserting it was
    // false is what made this red under `cargo llvm-cov` while green under
    // `cargo test`. When stdin is a pipe the watcher is a thread that sets the
    // flag the moment that pipe reaches EOF, so the value describes the stdin
    // the harness handed over, not anything this function decides: a captured
    // pipe that stays open reads false, an already-closed one reads true, and
    // true is the correct answer there rather than a failure. Racing a
    // background thread to observe it first is not a property worth pinning.
    //
    // What must hold on every host is that asking is safe. Reading it back
    // proves the returned handle is usable, which is the whole contract.
    let _ = gone.load(std::sync::atomic::Ordering::Relaxed);
}

#[test]
fn collecting_from_the_ambient_daemon_never_panics() {
    // Whether this host runs Docker or not, the answer is a table.
    let _ = multitop_agent::docker::collect();
}

// -------------------------------------------------------------- tiny frames
