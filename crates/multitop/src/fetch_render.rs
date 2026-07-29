use std::sync::OnceLock;

use multitop_agent::color::Palette;
use multitop_agent::fetch::FetchSnapshot;
use multitop_agent::fmt::center_header;

/// A logo from the neofetch database.
#[derive(Debug)]
struct Logo {
    patterns: Vec<String>,
    colors: Vec<u8>,
    lines: Vec<String>,
}

/// Parsed logo database.
struct LogoDb {
    logos: Vec<Logo>,
}

static LOGO_DB: OnceLock<LogoDb> = OnceLock::new();

fn load_db() -> &'static LogoDb {
    LOGO_DB.get_or_init(|| {
        let compressed = include_bytes!("../data/logos.bin.zst");
        let raw = zstd::decode_all(&compressed[..]).expect("zstd decompress logo db");
        parse_db(&raw)
    })
}

fn parse_db(bytes: &[u8]) -> LogoDb {
    assert!(bytes.len() >= 8, "logo db too short");
    assert_eq!(&bytes[..4], b"MTLG", "bad magic");
    let _version = u16::from_le_bytes([bytes[4], bytes[5]]);
    let count = u16::from_le_bytes([bytes[6], bytes[7]]);
    let mut pos = 8;

    let mut logos = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let entry_size = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        let entry_end = pos + 2 + entry_size;
        pos += 2;

        let num_patterns = bytes[pos] as usize;
        pos += 1;

        let mut patterns = Vec::with_capacity(num_patterns);
        for _ in 0..num_patterns {
            let plen = bytes[pos] as usize;
            pos += 1;
            patterns.push(String::from_utf8_lossy(&bytes[pos..pos + plen]).to_string());
            pos += plen;
        }

        let num_colors = bytes[pos] as usize;
        pos += 1;
        let colors: Vec<u8> = bytes[pos..pos + num_colors].to_vec();
        pos += num_colors;

        let num_lines = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        pos += 2;

        let mut lines = Vec::with_capacity(num_lines);
        for _ in 0..num_lines {
            let llen = bytes[pos] as usize;
            pos += 1;
            lines.push(String::from_utf8_lossy(&bytes[pos..pos + llen]).to_string());
            pos += llen;
        }

        logos.push(Logo {
            patterns,
            colors,
            lines,
        });

        pos = entry_end;
    }

    LogoDb { logos }
}

fn color_for(ci: u8, pal: &Palette) -> &'static str {
    match ci {
        1 => pal.red,
        2 => pal.green,
        3 => pal.yellow,
        4 => pal.blue,
        5 => pal.purple,
        6 => pal.cyan,
        7 => pal.white,
        _ => pal.white,
    }
}

fn find_logo<'a>(db: &'a LogoDb, os: &str, kernel: &str) -> Option<&'a Logo> {
    let o = os.to_ascii_lowercase();
    let k = kernel.to_ascii_lowercase();

    for logo in &db.logos {
        for pat in &logo.patterns {
            let pl = pat.as_str();
            if !pl.is_empty() && (o.starts_with(pl) || k.starts_with(pl)) {
                return Some(logo);
            }
        }
    }
    None
}

/// Pick the best N lines from a logo to fit available height.
///
/// If the logo has ≤ `target` lines, pad with empty lines (centered).
/// If it has more, take the center `target` lines so the most recognizable
/// part of the art is shown.
fn pick_lines(logo: &Logo, target: usize) -> Vec<&str> {
    if logo.lines.is_empty() {
        return vec![""; target];
    }
    if logo.lines.len() <= target {
        let out: Vec<&str> = logo.lines.iter().map(|s| s.as_str()).collect();
        let pad_top = (target - out.len()) / 2;
        let pad_bot = target - out.len() - pad_top;
        let mut result = Vec::with_capacity(target);
        result.extend(std::iter::repeat_n("", pad_top));
        result.extend(out);
        result.extend(std::iter::repeat_n("", pad_bot));
        return result;
    }
    // Logo is taller than target: crop center portion
    let start = (logo.lines.len() - target) / 2;
    logo.lines[start..start + target]
        .iter()
        .map(|s| s.as_str())
        .collect()
}

pub fn render_fetch(
    snap: &FetchSnapshot,
    cols: usize,
    max_rows: usize,
    pal: &Palette,
) -> Vec<String> {
    let mut out = Vec::with_capacity(12);
    out.push(center_header(&snap.user_host, cols, pal));

    let db = load_db();
    let logo = find_logo(db, &snap.os, &snap.kernel);

    let details: [(&str, &str); 7] = [
        ("OS", &snap.os),
        ("Kernel", &snap.kernel),
        ("Uptime", &snap.uptime),
        ("Host", &snap.host_model),
        ("CPU", &snap.cpu_model),
        ("Memory", &snap.memory_str),
        ("Disk", &snap.disk_str),
    ];

    let colors_row = format!(
        "\x1b[40m  \x1b[41m  \x1b[42m  \x1b[43m  \x1b[44m  \x1b[45m  \x1b[46m  \x1b[47m  {}",
        pal.reset
    );

    let max_body = max_rows.saturating_sub(1);
    let mut row_idx = 0;

    if let Some(lg) = logo {
        let accent = color_for(lg.colors.first().copied().unwrap_or(7), pal);
        let n = details.len().min(max_body);
        let logo_lines = pick_lines(lg, n);
        let logo_width = logo_lines.iter().map(|l| l.len()).max().unwrap_or(0);

        for (i, &(label, val)) in details.iter().enumerate().take(n) {
            let logo_part = logo_lines.get(i).copied().unwrap_or("");
            out.push(format!(
                " {}{:logo_width$}{}{} {}{:<7}{} : {}{}{}",
                accent,
                logo_part,
                pal.reset,
                "",
                pal.bold,
                label,
                pal.reset,
                pal.white,
                val,
                pal.reset,
                logo_width = logo_width,
            ));
            row_idx += 1;
        }
    } else {
        // No logo — just list details without the left column
        for (label, val) in &details {
            if row_idx >= max_body {
                break;
            }
            out.push(format!(
                "   {}{:<7}{} : {}{}{}",
                pal.bold,
                label,
                pal.reset,
                pal.white,
                val,
                pal.reset
            ));
            row_idx += 1;
        }
    }

    if row_idx < max_body {
        out.push(format!(
            "   {}",
            colors_row
        ));
    }

    out
}
