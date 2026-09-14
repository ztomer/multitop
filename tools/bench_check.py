#!/usr/bin/env python3
"""Benchmark thresholds: the client_bench latencies stay under their limits.

    python3 tools/bench_check.py              # run the bench, check the two latencies
    python3 tools/bench_check.py --self-test  # prove the parser and the comparison bite

Needs a quiet machine to mean anything; the numbers are generous for that
reason. Lives here (it used to be the one gate only scripts/local-ci.py ran)
so the pre-push list in .gatesrc names it like everything else.
"""
from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MAX_DECODE_NS = 5_000.0
MAX_RENDER_NS = 50_000.0
DECODE = re.compile(r"Latency:\s+([\d.]+)\s+ns\s+/\s+packet")
RENDER = re.compile(r"Latency:\s+([\d.]+)\s+ns\s+/\s+frame")


def verdict(output: str) -> tuple[bool, str]:
    decode, render = DECODE.search(output), RENDER.search(output)
    if not decode or not render:
        return False, "the benchmark ran but its output did not carry the two latencies"
    d, r = float(decode.group(1)), float(render.group(1))
    line = f"packet decode {d:.0f} ns (limit {MAX_DECODE_NS:.0f}); frame render {r:.0f} ns (limit {MAX_RENDER_NS:.0f})"
    if d > MAX_DECODE_NS or r > MAX_RENDER_NS:
        return False, f"a latency is over its threshold: {line}"
    return True, line


def self_test() -> int:
    ok_out = "Latency: 1200.5 ns / packet\nLatency: 30000 ns / frame\n"
    slow_out = "Latency: 9000 ns / packet\nLatency: 30000 ns / frame\n"
    if not verdict(ok_out)[0]:
        print("bench-check self-test: a passing run was reported as failing", file=sys.stderr)
        return 1
    if verdict(slow_out)[0]:
        print("bench-check self-test: an over-threshold run was NOT reported", file=sys.stderr)
        return 1
    if verdict("no numbers here")[0]:
        print("bench-check self-test: missing latencies were NOT reported", file=sys.stderr)
        return 1
    print("bench-check self-test: passed")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    proc = subprocess.run(
        ["cargo", "bench", "-p", "multitop", "--bench", "client_bench"],
        cwd=REPO, capture_output=True, text=True, errors="replace",
    )
    if proc.returncode != 0:
        print("bench-check: the benchmark did not run", file=sys.stderr)
        sys.stderr.write(proc.stderr[-2000:])
        return 1
    passed, line = verdict(proc.stdout)
    print(("bench-check: " if passed else "bench-check FAILED: ") + line, file=None if passed else sys.stderr)
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
