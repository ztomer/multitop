#![no_main]

use libfuzzer_sys::fuzz_target;
use multitop_agent::proc;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = proc::parse_proc_stat(text);
    }
});