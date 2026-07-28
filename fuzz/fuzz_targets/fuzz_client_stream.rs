#![no_main]

use libfuzzer_sys::fuzz_target;
use multitop::ssh;
use multitop::ui;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        // 1. Fuzz SSH need agent marker parser
        let _ = ssh::parse_need_agent(text);

        // 2. Fuzz TUI line and header refitting at various widths
        for &width in &[0usize, 10, 40, 80, 120, 200] {
            let _ = ui::refit_line(text, width);
            let _ = ui::refit_header(text, width);
        }
    }
});
