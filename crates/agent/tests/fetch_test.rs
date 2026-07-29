use multitop_agent::color::{strip_ansi, ANSI, PLAIN};
use multitop_agent::fetch::{render_fetch, FetchSnapshot};
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

fn find(out: &[String], needle: &str) -> Vec<String> {
    out.iter().filter(|l| l.contains(needle)).cloned().collect()
}

fn plain(out: &[String]) -> Vec<String> {
    out.iter().map(|l| strip_ansi(l)).collect()
}

// ------------------------------------------------------------------------

#[test]
fn fetch_ubuntu_renders_circle_logo() {
    let out = render_fetch(&snap("Ubuntu 24.04 LTS", "6.8.0-45-generic"), 80, 24, &ANSI);
    let header = &plain(&out)[0];
    assert!(header.contains(&fullwidth("user@host")), "header missing host");
    let os_line = find(&out, "OS").pop().expect("OS line");
    assert!(strip_ansi(&os_line).contains("Ubuntu 24.04"), "wrong OS");
    // Ubuntu logo first row has the underscore-dot pattern
    let plain_lines = plain(&out);
    let first_logo = &plain_lines[1];
    assert!(first_logo.contains("_.  "), "Ubuntu logo missing first arc row");
}

#[test]
fn fetch_macos_renders_apple_logo() {
    let out = render_fetch(&snap("macOS 15.0", "Darwin 24.0.0"), 80, 24, &ANSI);
    let plain_lines = plain(&out);
    let logo_line = &plain_lines[1];
    assert!(logo_line.contains(".--."), "macOS logo missing Apple shape");
    let os_line = find(&out, "OS").pop().expect("OS line");
    assert!(strip_ansi(&os_line).contains("macOS"));
}

#[test]
fn fetch_debian_renders_swirl_logo() {
    let out = render_fetch(&snap("Debian GNU/Linux 12", "6.1.0-21-amd64"), 80, 24, &ANSI);
    let plain_lines = plain(&out);
    let logo_line = &plain_lines[1];
    assert!(logo_line.contains("_____"), "Debian logo missing bar");
}

#[test]
fn fetch_fedora_renders_f_logo() {
    let out = render_fetch(&snap("Fedora Linux 40", "6.9.3-200.fc40.x86_64"), 80, 24, &ANSI);
    let plain_lines = plain(&out);
    let logo_line = &plain_lines[1];
    assert!(logo_line.contains("___"), "Fedora logo missing top bar");
}

#[test]
fn fetch_generic_linux_renders_tux_logo() {
    let out = render_fetch(&snap("Linux", "6.8.0-arch1-1"), 80, 24, &ANSI);
    let plain_lines = plain(&out);
    let logo_line = &plain_lines[1];
    assert!(logo_line.contains(".--."), "generic Linux should use Tux logo");
    let kernel_line = find(&out, "Kernel").pop().expect("Kernel line");
    assert!(strip_ansi(&kernel_line).contains("arch"), "kernel string mismatch");
}

#[test]
fn fetch_every_detail_row_appears_in_order() {
    let out = render_fetch(&snap("Ubuntu 24.04", "6.8.0-45-generic"), 80, 24, &ANSI);
    let labels = ["OS", "Kernel", "Uptime", "Host", "CPU", "Memory", "Disk"];
    for label in &labels {
        let found = find(&out, label);
        assert!(!found.is_empty(), "missing {label} row");
    }
    let plain_lines = plain(&out);
    for (i, label) in labels.iter().enumerate() {
        let idx = i + 1; // skip header at 0
        if idx < plain_lines.len() {
            assert!(
                plain_lines[idx].contains(label),
                "row {idx} should contain {label}: {:?}",
                plain_lines[idx]
            );
        }
    }
    assert_eq!(out.len(), 9, "header + 7 details + color bar = 9 lines at 80x24");
}

#[test]
fn fetch_fits_narrow_panel() {
    let out = render_fetch(&snap("Arch Linux", "6.9-arch1"), 40, 10, &ANSI);
    // Should not panic and should have at least header + some detail rows
    assert!(out.len() >= 3, "narrow panel should show {out:?}");
    let header = &plain(&out)[0];
    assert!(header.contains(&fullwidth("user@host")));
}

#[test]
fn fetch_fits_tiny_panel() {
    let out = render_fetch(&snap("Ubuntu 24.04", "6.8.0-45-generic"), 40, 3, &ANSI);
    // Bare minimum: header + at most 1 row
    assert!(out.len() >= 2, "tiny panel should show at least header: {out:?}");
    assert!(out.len() <= 4, "tiny panel should clip: {out:?}");
    let header = &plain(&out)[0];
    assert!(header.contains(&fullwidth("user@host")));
}

#[test]
fn fetch_plain_palette_has_no_escapes() {
    let out = render_fetch(&snap("Ubuntu 24.04", "6.8.0"), 80, 24, &PLAIN);
    for (i, line) in out.iter().enumerate() {
        if line.contains("\x1b[40m") {
            continue;
        }
        assert!(!line.contains('\x1b'), "plain palette escape at row {i}: {line:?}");
    }
}

#[test]
fn fetch_color_bar_row_present_when_room() {
    let out = render_fetch(&snap("Ubuntu 24.04", "6.8.0"), 80, 24, &ANSI);
    let last = out.last().expect("at least one line");
    assert!(last.contains("\x1b[40m"), "color bar missing ANSI black bg");
    assert!(last.contains("\x1b[47m"), "color bar missing ANSI white bg");
}

#[test]
fn fetch_color_bar_omitted_when_no_room() {
    let out = render_fetch(&snap("Ubuntu 24.04", "6.8.0"), 80, 8, &ANSI);
    let last = out.last().expect("at least one line");
    assert!(strip_ansi(last).trim().is_empty() || !last.contains("\x1b[40m"),
        "color bar should be omitted or clipped in small panel: {last:?}");
}