#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = multitop_agent::fetch::sample_fetch("test-host");
        let lines: Vec<&str> = text.lines().collect();
        for line in lines {
            let _ = line.strip_prefix("PRETTY_NAME=");
            let _ = line.strip_prefix("NAME=");
            let _ = line.strip_prefix("model name=");
            let _ = line.strip_prefix("Hardware=");
            let _ = line.strip_prefix("processor");
        }
    }
});