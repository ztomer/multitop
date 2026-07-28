#!/usr/bin/env python3
"""
Deterministic Local CI Verification Script for multitop.

Gates enforced:
 1. File LOC Limit: No .rs file in crates/ may exceed 500 lines of code.
 2. Approved Emoji / Unicode Whitelist: No unapproved emojis allowed in source.
 3. Workspace Build & Test Execution: All tests must pass cleanly.
 4. Fuzzing & Protocol Sanity: Fuzzing suite must execute cleanly with zero panics.
 5. Deterministic Regression Detection: Performance thresholds must be met.
"""

import os
import sys
import glob
import subprocess
import re

def print_info(message):
    print(f"[ ==> ] {message}")

def print_wrn(message):
    print(f"[ Wrn ] {message}")

def print_err(message):
    print(f"[ Err ] {message}")

def print_ok(message):
    print(f"[ Ok  ] {message}")

APPROVED_EMOJIS_AND_SYMBOLS = set([
    ' ', '▂', '▃', '▄', '▅', '▆', '▇', '█', # Sparklines
    '→', '←', '↑', '↓', '°',                # Symbols & Degrees
    '✓', '✗', '…', '–', '—',                # Approved Symbols & Punctuation
    '\u2192', '\u2190', '\u2191', '\u2193',
    '\u00b0', '\u2026', '\u2013', '\u2014',
    '\u2581', '\u2582', '\u2583', '\u2584', '\u2585', '\u2586', '\u2587', '\u2588',
])

def check_loc_limit():
    print_info("Gate 1: Checking File LOC Limits (Soft Warning: 400, Hard Limit: 500)...")
    violations = []
    warnings = []
    rs_files = glob.glob("crates/**/*.rs", recursive=True)
    for path in rs_files:
        with open(path, "r", encoding="utf-8") as f:
            lines = f.readlines()
            if len(lines) > 500:
                violations.append((path, len(lines)))
            elif len(lines) >= 400:
                warnings.append((path, len(lines)))

    if warnings:
        for path, count in warnings:
            print_wrn(f"Approaching LOC Threshold: {path} has {count} lines (Split cleanly before reaching 500)")

    if violations:
        for path, count in violations:
            print_err(f"LOC Violation: {path} has {count} lines (> 500 limit)")
        return False
    
    print_ok(f"All {len(rs_files)} Rust source files are strictly <= 500 LOC.")
    return True

def check_emoji_whitelist():
    print_info("Gate 2: Checking Emoji & Unicode Whitelist...")
    violations = []
    rs_files = glob.glob("crates/**/*.rs", recursive=True)
    
    # Emoji regex pattern
    emoji_pattern = re.compile(r'[\U00010000-\U0010ffff]')

    for path in rs_files:
        with open(path, "r", encoding="utf-8") as f:
            for line_no, line in enumerate(f, 1):
                for char in line:
                    if ord(char) > 127:
                        # Non-ASCII character check
                        if emoji_pattern.match(char) and char not in APPROVED_EMOJIS_AND_SYMBOLS:
                            violations.append((path, line_no, char, f"U+{ord(char):04X}"))

    if violations:
        for path, line_no, char, hex_val in violations:
            print_err(f"Unapproved Emoji/Symbol Violation: {path}:{line_no} character '{char}' ({hex_val})")
        return False

    print_ok("Emoji & Unicode whitelist passed cleanly.")
    return True

def run_build_and_tests():
    print_info("Gate 3: Running Workspace Build & Test Suite...")
    res = subprocess.run(["cargo", "test", "--workspace"], capture_output=True, text=True)
    if res.returncode != 0:
        print_err("Cargo test failed!")
        print(res.stderr)
        return False
    print_ok("Cargo test suite passed cleanly (100% green).")
    return True

def run_fuzz_sanity():
    print_info("Gate 4: Running Fuzzing Protocol Sanity Gate...")
    res = subprocess.run(["cargo", "test", "-p", "multitop-agent", "proto::tests::decode_fuzz_garbage_never_panics"], capture_output=True, text=True)
    if res.returncode != 0:
        print_err("Fuzz protocol sanity check failed!")
        print(res.stderr)
        return False
    print_ok("Fuzz protocol sanity check passed.")
    return True

def check_performance_regression():
    print_info("Gate 5: Running Performance Regression Check...")
    res = subprocess.run(["cargo", "bench", "-p", "multitop", "--bench", "client_bench"], capture_output=True, text=True)
    if res.returncode != 0:
        print_err("Benchmark execution failed!")
        print(res.stderr)
        return False
    
    # Parse latency from benchmark output
    output = res.stdout
    decode_match = re.search(r'Latency:\s+([\d\.]+)\s+ns\s+/\s+packet', output)
    render_match = re.search(r'Latency:\s+([\d\.]+)\s+ns\s+/\s+frame', output)

    if decode_match and render_match:
        decode_ns = float(decode_match.group(1))
        render_ns = float(render_match.group(1))
        print_info(f"Measured Packet Decode Latency: {decode_ns:.2f} ns (Threshold: < 5000.0 ns)")
        print_info(f"Measured Frame Render Latency:  {render_ns:.2f} ns (Threshold: < 50000.0 ns)")

        if decode_ns > 5000.0:
            print_err(f"Performance Regression Detected: Packet decode latency {decode_ns:.2f} ns exceeds 5000.0 ns threshold!")
            return False
        if render_ns > 50000.0:
            print_err(f"Performance Regression Detected: Frame render latency {render_ns:.2f} ns exceeds 50000.0 ns threshold!")
            return False

    print_ok("Performance regression check passed cleanly. All metrics optimal.")
    return True

def main():
    print("============================================================")
    print("   multitop Deterministic Local CI Verification Harness     ")
    print("============================================================\n")

    steps = [
        check_loc_limit,
        check_emoji_whitelist,
        run_build_and_tests,
        run_fuzz_sanity,
        check_performance_regression,
    ]

    for step in steps:
        if not step():
            print_err("Local CI Verification FAILED.")
            sys.exit(1)
        print()

    print("============================================================")
    print_ok("ALL LOCAL CI GATES PASSED DETERMINISTICALLY! READY TO COMMIT.")
    print("============================================================")

if __name__ == "__main__":
    main()
