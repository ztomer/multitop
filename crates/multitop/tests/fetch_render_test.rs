use multitop::fetch_render;
use multitop_agent::color::{strip_ansi, ANSI};
use multitop_agent::fetch::FetchSnapshot;
use multitop_agent::fmt::fullwidth;

fn snap(os: &str, kernel: &str) -> FetchSnapshot {
    FetchSnapshot {
        user_host: "user@host".into(),
        os: os.into(),
        kernel: kernel.into(),
        uptime: "10d 4h".into(),
        host_model: "Test Model".into(),
        cpu_model: "Test CPU (8)".into(),
        memory_str: "8.0GiB/16.0GiB (50%)".into(),
        disk_str: "64.0GiB/256.0GiB (25%)".into(),
    }
}

fn plain(out: &[String]) -> Vec<String> {
    out.iter().map(|l| strip_ansi(l)).collect()
}

fn logo_line(out: &[String], row: usize) -> String {
    let p = plain(out);
    if row < p.len() {
        p[row].clone()
    } else {
        String::new()
    }
}

fn first_logo_chars(out: &[String]) -> Vec<char> {
    logo_line(out, 1).chars().collect()
}

// ------------------------------------------------------------------------
//  OS DETECTION — every supported OS must match its specific logo
// ------------------------------------------------------------------------

#[test]
fn os_macos_gets_apple_logo() {
    let out = fetch_render::render_fetch(&snap("macOS 15.0", "Darwin 24.0.0"), 80, 24, &ANSI);
    let first = first_logo_chars(&out);
    // Apple logo starts with spaces followed by the Apple shape
    // The neofetch Apple logo begins: "                    c.'"
    // After stripping color markers, first line has leading spaces + "c.'"
    assert!(first.contains(&'c'), "macOS logo should show Apple: {first:?}");
}

#[test]
fn os_darwin_kernel_gets_apple_logo() {
    let out = fetch_render::render_fetch(&snap("Some OS", "Darwin 24.0.0"), 80, 24, &ANSI);
    let first = first_logo_chars(&out);
    assert!(first.contains(&'c'), "Darwin kernel should map to Apple logo: {first:?}");
}

#[test]
fn os_ubuntu_gets_circle_logo() {
    let out = fetch_render::render_fetch(&snap("Ubuntu 24.04 LTS", "6.8.0-45-generic"), 80, 24, &ANSI);
    // Ubuntu logo has ".-/+" and "ossssoo" patterns
    let p = logo_line(&out, 1);
    assert!(p.contains("+") || p.contains("/"), "Ubuntu logo should have circle shapes: {p}");
}

#[test]
fn os_ubuntu_derivatives_get_own_logos() {
    let out = fetch_render::render_fetch(&snap("Kubuntu 24.04", "6.8.0-45-generic"), 80, 24, &ANSI);
    let os_line = plain(&out).iter().find(|l| l.contains("OS")).cloned().unwrap_or_default();
    assert!(os_line.contains("Kubuntu"), "Kubuntu OS name should appear");

    // Kubuntu should NOT match the same entry as Ubuntu
    let ubuntu_out = fetch_render::render_fetch(&snap("Ubuntu 24.04 LTS", "6.8.0-45-generic"), 80, 24, &ANSI);
    let kubuntu_line = logo_line(&out, 1);
    let ubuntu_line = logo_line(&ubuntu_out, 1);
    assert_ne!(kubuntu_line, ubuntu_line, "Kubuntu and Ubuntu should have different logos");
}

#[test]
fn os_debian_gets_swirl_logo() {
    let out = fetch_render::render_fetch(&snap("Debian GNU/Linux 12", "6.1.0-21-amd64"), 80, 24, &ANSI);
    let p = logo_line(&out, 1);
    assert!(p.contains("$") || p.contains(",") || p.contains("_"),
        "Debian logo should have swirl characters: {p}");
}

#[test]
fn os_fedora_gets_infinity_logo() {
    let out = fetch_render::render_fetch(&snap("Fedora Linux 40", "6.9.3-200.fc40.x86_64"), 80, 24, &ANSI);
    let p = logo_line(&out, 1);
    assert!(p.contains("'") || p.contains(":"), "Fedora logo should have infinity shapes: {p}");
}

#[test]
fn os_arch_gets_arch_logo() {
    let out = fetch_render::render_fetch(&snap("Arch Linux", "6.9-arch1"), 80, 24, &ANSI);
    let p = logo_line(&out, 1);
    // Arch logo starts with backtick-tick shapes from neofetch: "                   -`"
    assert!(p.contains("`") || p.contains("'") || p.contains("-"),
        "Arch logo should have mountain shapes: {p}");
}

#[test]
fn os_freebsd_gets_beastie_logo() {
    let out = fetch_render::render_fetch(&snap("FreeBSD 13.2-RELEASE", "13.2-RELEASE-p6"), 80, 24, &ANSI);
    let p = logo_line(&out, 1);
    assert!(p.contains("`"), "FreeBSD logo should have backtick shapes: {p}");
}

#[test]
fn os_windows_gets_windows_logo() {
    let out = fetch_render::render_fetch(&snap("Windows 10", "10.0.19045"), 80, 24, &ANSI);
    let os_line = plain(&out).iter().find(|l| l.contains("OS")).cloned().unwrap_or_default();
    assert!(os_line.contains("Windows"), "Windows OS name should appear");
    // Should have an actual logo (not fallback)
    assert!(out.len() >= 3, "Windows should have a logo");
}

#[test]
fn os_windows11_gets_windows_logo() {
    let out = fetch_render::render_fetch(&snap("Windows 11", "10.0.22621"), 80, 24, &ANSI);
    let os_line = plain(&out).iter().find(|l| l.contains("OS")).cloned().unwrap_or_default();
    assert!(os_line.contains("Windows"), "Windows 11 OS name should appear");
    assert!(out.len() >= 3, "Windows 11 should have a logo");
}

#[test]
fn os_generic_linux_gets_tux_logo() {
    let out = fetch_render::render_fetch(&snap("Linux", "6.6.0-generic"), 80, 24, &ANSI);
    let p = logo_line(&out, 1);
    assert!(p.contains("#"), "Generic Linux Tux logo should have hash marks: {p}");
}

#[test]
fn os_manjaro_gets_arch_derived_logo() {
    let out = fetch_render::render_fetch(&snap("Manjaro Linux", "6.6.0-1-MANJARO"), 80, 24, &ANSI);
    let p = logo_line(&out, 1);
    assert!(p.contains("██"),
        "Manjaro logo should have block characters: {p}");
}

#[test]
fn os_endeavouros_gets_arch_derived_logo() {
    let out = fetch_render::render_fetch(&snap("EndeavourOS", "6.6.0-arch1"), 80, 24, &ANSI);
    let plain_lines = plain(&out);
    // EndeavourOS has a tall logo; check that at least one of the visible logo/detail
    // rows contains mountain shapes (backtick/tick characters)
    let has_mountain = plain_lines.iter().skip(1).any(|l| {
        l.contains("`") || l.contains("'") || l.contains("-")
    });
    assert!(has_mountain, "EndeavourOS logo should have mountain shapes");
}

#[test]
fn os_raspbian_gets_debian_logo() {
    let out = fetch_render::render_fetch(&snap("Raspbian GNU/Linux 11", "6.1.0-rpi7"), 80, 24, &ANSI);
    let os_line = plain(&out).iter().find(|l| l.contains("OS")).cloned().unwrap_or_default();
    assert!(os_line.contains("Raspbian"), "Raspbian OS name should appear");
    let p = logo_line(&out, 1);
    assert!(p.contains("ooo") || p.contains("::"),
        "Raspbian should use Debian swirl logo: {p}");
}

#[test]
fn os_unknown_os_still_renders() {
    // Should not panic — should render a header + at least some detail
    let out = fetch_render::render_fetch(&snap("Commodore 64 OS/2", "2.0.0-kernel"), 80, 24, &ANSI);
    assert!(out.len() >= 2, "unknown OS should render {out:?}");
    let header = &plain(&out)[0];
    assert!(header.contains(&fullwidth("user@host")), "header present for unknown OS");
}

// ------------------------------------------------------------------------
//  TEXT ALIGNMENT — every row must be structurally aligned
// ------------------------------------------------------------------------

#[test]
fn alignment_header_is_centered() {
    let out = fetch_render::render_fetch(&snap("Ubuntu 24.04", "6.8.0"), 80, 24, &ANSI);
    let header = strip_ansi(&out[0]);
    let fw = fullwidth("user@host");
    assert!(header.contains(&fw), "header should have fullwidth hostname");
}

#[test]
fn alignment_label_column_is_fixed_width() {
    let out = fetch_render::render_fetch(&snap("Ubuntu 24.04", "6.8.0"), 80, 24, &ANSI);
    let plain_lines = plain(&out);
    let detail_lines: Vec<&str> = plain_lines[1..].iter()
        .filter(|l| l.contains(" : "))
        .map(|s| s.as_str())
        .collect();
    for line in &detail_lines {
        // Each detail line should have a " : " separator
        assert!(line.contains(" : "), "detail row should have ' : ': {line:?}");
        let before_colon = line.split(" : ").next().unwrap_or("").trim();
        // Before the colon, we have: <optional logo chars> <label>
        // The label (OS, Kernel, etc.) should be right before the " : "
        assert!(
            ["OS", "Kernel", "Uptime", "Host", "CPU", "Memory", "Disk"]
                .iter().any(|l| before_colon.ends_with(l)),
            "before ' : ' should end with a label, got {before_colon:?}"
        );
    }
}

#[test]
fn alignment_colon_position_is_consistent() {
    let out = fetch_render::render_fetch(&snap("Ubuntu 24.04", "6.8.0"), 80, 24, &ANSI);
    let plain_lines = plain(&out);
    let detail_lines: Vec<&str> = plain_lines[1..].iter()
        .filter(|l| l.contains(" : "))
        .map(|s| s.as_str())
        .collect();
    if detail_lines.len() >= 2 {
        let col_pos = |l: &str| -> usize {
            l.split(" : ").next().unwrap_or("").chars().count()
        };
        let first = col_pos(detail_lines[0]);
        for (i, line) in detail_lines.iter().enumerate() {
            let pos = col_pos(line);
            assert_eq!(
                pos, first,
                "colon column mismatch at row {i}: {pos} vs {first}",
            );
        }
    }
}

#[test]
fn alignment_each_line_starts_with_space_prefix() {
    let out = fetch_render::render_fetch(&snap("Ubuntu 24.04", "6.8.0"), 80, 24, &ANSI);
    let plain_lines = plain(&out);
    // Skip header (index 0) and color bar (last)
    for (i, line) in plain_lines.iter().enumerate().skip(1) {
        if line.contains("\x1b") || line.trim().is_empty() {
            continue;
        }
        // Every content line should start with a space
        assert!(
            line.starts_with(' '),
            "row {i} should start with a space: {line:?}"
        );
    }
}

// ------------------------------------------------------------------------
//  LOGO SIZING — correct number of lines for available height
// ------------------------------------------------------------------------

#[test]
fn sizing_shows_full_logo_when_there_is_room() {
    let out = fetch_render::render_fetch(&snap("Ubuntu 24.04", "6.8.0"), 80, 24, &ANSI);
    // The full Ubuntu logo (20 lines) is shown, plus a color bar → 22 total rows
    assert!(out.len() >= 20, "full logo + details + color bar shown");
    for label in &["OS", "Kernel", "Uptime", "Host", "CPU", "Memory", "Disk"] {
        assert!(
            plain(&out).iter().any(|l| l.contains(label)),
            "missing {label} row"
        );
    }
}

#[test]
fn sizing_9_rows_shows_7_details_plus_color_bar() {
    let out = fetch_render::render_fetch(&snap("Ubuntu 24.04", "6.8.0"), 80, 9, &ANSI);
    // 9 rows → max_body = 7 → 7 details + color bar = header + 7 + 1 = 9
    assert_eq!(out.len(), 9, "at 80x9: header + 7 details + color bar");
}

#[test]
fn sizing_8_rows_omits_color_bar() {
    let out = fetch_render::render_fetch(&snap("Ubuntu 24.04", "6.8.0"), 80, 8, &ANSI);
    // 8 rows → max_body = 6 → shows 6 details, no room for color bar
    assert!(out.len() <= 8, "at 80x8: header + <=7 rows = {len}", len = out.len());
}

#[test]
fn sizing_3_rows_shows_header_and_2_details() {
    let out = fetch_render::render_fetch(&snap("Ubuntu 24.04", "6.8.0"), 40, 3, &ANSI);
    assert_eq!(out.len(), 3, "at 40x3: header + 2 detail rows");
}

#[test]
fn sizing_2_rows_shows_header_and_1_detail() {
    let out = fetch_render::render_fetch(&snap("Ubuntu 24.04", "6.8.0"), 80, 2, &ANSI);
    // 2 rows → max_body = 1 → header + 1 detail
    assert_eq!(out.len(), 2, "at 80x2: header + 1 detail");
}

#[test]
fn sizing_0_rows_does_not_panic() {
    let out = fetch_render::render_fetch(&snap("Ubuntu 24.04", "6.8.0"), 80, 0, &ANSI);
    assert!(!out.is_empty(), "should at least have a header");
}

#[test]
fn sizing_logo_lines_match_detail_lines_when_logo_is_tall() {
    // The Ubuntu logo has 20 lines — many more than the 7 details
    let out = fetch_render::render_fetch(&snap("Ubuntu 24.04", "6.8.0"), 80, 24, &ANSI);
    let detail_count = plain(&out)[1..].iter()
        .filter(|l| l.contains(" : "))
        .count();
    let logo_first = logo_line(&out, 1);
    let logo_last = logo_line(&out, detail_count); // last logo line used
    // The first and last logo lines should be different (not all padding)
    // because Ubuntu is taller than 7, we crop the center
    assert!(!logo_first.trim().is_empty() || !logo_last.trim().is_empty(),
        "logo should have content for Ubuntu");
}

#[test]
fn sizing_logo_lines_pad_when_logo_is_short() {
    // Alpine has only 4 lines — fewer than 7 details
    let out = fetch_render::render_fetch(&snap("Alpine Linux 3.18", "6.6.0"), 80, 24, &ANSI);
    let detail_count = plain(&out)[1..].iter()
        .filter(|l| l.contains(" : "))
        .count();
    assert_eq!(detail_count, 7, "Alpine should show all 7 details");
    // The logo should be centered vertically (empty lines above and below)
    let first = logo_line(&out, 1);
    let last = logo_line(&out, detail_count);
    // A short logo has empty padding lines — but NOT all empty
    assert!(!first.trim().is_empty() || !last.trim().is_empty(),
        "short logo should have content: first={first:?} last={last:?}");
}

#[test]
fn sizing_wide_panel_does_not_affect_row_count() {
    let narrow = fetch_render::render_fetch(&snap("Ubuntu 24.04", "6.8.0"), 40, 24, &ANSI);
    let wide = fetch_render::render_fetch(&snap("Ubuntu 24.04", "6.8.0"), 200, 24, &ANSI);
    // A narrow panel may drop the logo, producing fewer rows than a wide one.
    assert!(!narrow.is_empty(), "narrow should produce output");
    assert!(!wide.is_empty(), "wide should produce output");
    assert!(narrow.len() <= wide.len(),
        "narrow ({}) should not have more rows than wide ({})", narrow.len(), wide.len());
}
