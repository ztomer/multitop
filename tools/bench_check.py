#!/usr/bin/env python3
"""Benchmark thresholds: decoding a packet and rendering a frame stay under their
INSTRUCTION budgets.

    python3 tools/bench_check.py              # count the two loops, check the budgets
    python3 tools/bench_check.py --self-test  # prove the parser and the comparison bite

The figure is instructions retired per operation, not nanoseconds. Until
2026-10-14 this gated wall-clock latency (frame render under 50 us), and on this
shared box that is a gate on whoever else is running: the pre-push run at load
~35-65 read 54976 ns on all three tries, while the same tree alone read 33919 ns.
"Best of three" only lowers the odds; it does not change what is measured.

Instructions retired measure the code. The bench binary runs ONE loop
(`client_bench --only=render --iters=N`) under `/usr/bin/time -l`, which reports
the instructions its direct child retired; it runs at N and at 2N, and the
difference is N iterations' worth, with process start-up and set-up cancelled.
Calibrated 2026-10-14 (the numbers are in the commit that introduced this):
across a 2x synthetic load the per-frame count moved well under 1% while wall
time per frame moved by tens of percent, and a deliberate 2x slowdown inside
`render::render` turned the gate red. macOS only, like the pre-push list that
runs it; a `time` that does not report instructions fails closed.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# Measured 2026-10-14: decode 23,064 / render 566,614 instructions (the mean of three, 0.1% apart),
# and 566,469-566,878 under a 4x busy-loop load -- 0.07% spread. Load-independence is this gate's
# whole claim, so it is re-measured here rather than remembered.
#
# Each budget is ~1.7x its measurement: a real regression costs instructions every run, so the
# margin is for honest growth (a new column, a wider bar), not for noise -- there is no load noise
# left to absorb. A 2x slowdown is over both, and a loop that stopped measuring is under the floor
# in `verdict`. Both were proved on 2026-10-14: the first by a `render` that ran twice (1,134,630,
# red -- which is how that doubled path was found at all), the second by the parser's own cases.
#
# An earlier figure in this file said 627 k per frame. It was measured with a `render` that did the
# work TWICE, so it is not another measurement of the same code -- it is the bug, mid-flight, and
# the number it left behind was the only thing that made the error look like a tuned constant.
MAX_DECODE_INSTRUCTIONS = 40_000
MAX_RENDER_INSTRUCTIONS = 940_000
BUDGETS = {"decode": MAX_DECODE_INSTRUCTIONS, "render": MAX_RENDER_INSTRUCTIONS}
UNITS = {"decode": "packet", "render": "frame"}

# Iterations per count. The 2N run minus the N run is N iterations; a few
# thousand keeps the slower (render) pair near a second on a quiet machine.
ITERS = 5_000

INSTRUCTIONS = re.compile(r"^\s*(\d+)\s+instructions retired\s*$", re.MULTILINE)


def instructions(time_output: str) -> int | None:
    """The `instructions retired` figure from `/usr/bin/time -l`, or None."""
    m = INSTRUCTIONS.search(time_output)
    return int(m.group(1)) if m else None


def per_op(at_n: int, at_2n: int, n: int) -> float:
    """Instructions per iteration from a run at N and one at 2N."""
    return (at_2n - at_n) / n


def verdict(counts: dict[str, float]) -> tuple[bool, str]:
    """Each loop's per-op count against its budget."""
    missing = sorted(set(BUDGETS) - set(counts))
    if missing:
        return False, f"no instruction count for: {', '.join(missing)}"
    parts = [
        f"{name} {counts[name]:,.0f} instructions / {UNITS[name]} (budget {budget:,})"
        for name, budget in BUDGETS.items()
    ]
    line = "; ".join(parts)
    if any(counts[name] > budget for name, budget in BUDGETS.items()):
        return False, f"over an instruction budget: {line}"
    # The other direction: a loop the optimiser deleted, or a --only that ran
    # nothing, retires ~0 per iteration and would sit under any budget.
    if any(counts[name] < budget / 10 for name, budget in BUDGETS.items()):
        return False, f"a loop retired almost nothing, so it no longer measures the code: {line}"
    return True, line


def self_test() -> int:
    time_out = "        0.52 real         0.49 user\n   6289676430  instructions retired\n   2  cycles elapsed\n"
    failures = []
    if instructions(time_out) != 6289676430:
        failures.append("the `instructions retired` line was not parsed")
    if instructions("        0.52 real\n") is not None:
        failures.append("a `time` with no instruction count was read as a number")
    if per_op(3_000, 5_000, 1_000) != 2.0:
        failures.append("the N / 2N difference is not per iteration")
    if not verdict({"decode": 23_064, "render": 566_614})[0]:
        failures.append("today's counts were reported as over budget")
    if verdict({"decode": 2 * 23_064, "render": 2 * 566_614})[0]:
        failures.append("a 2x render was NOT reported")
    if verdict({"decode": 2 * 23_064, "render": 566_614})[0]:
        failures.append("a 2x decode was NOT reported")
    if verdict({"decode": 23_064, "render": 0})[0]:
        failures.append("a loop that retired nothing was NOT reported")
    if verdict({"render": 566_614})[0]:
        failures.append("a missing count was NOT reported")
    for f in failures:
        print(f"bench-check self-test: {f}", file=sys.stderr)
    if failures:
        return 1
    print("bench-check self-test: passed")
    return 0


def bench_binary() -> Path:
    """Build client_bench and return its executable (cargo prints it as JSON)."""
    proc = subprocess.run(
        ["cargo", "bench", "-p", "multitop", "--bench", "client_bench", "--no-run", "--message-format=json"],
        cwd=REPO,
        capture_output=True,
        text=True,
        errors="replace",
        stdin=subprocess.DEVNULL,
        check=False,
    )
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr[-2000:])
        raise SystemExit("bench-check: client_bench did not build")
    for line in proc.stdout.splitlines():
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("reason") == "compiler-artifact" and msg.get("target", {}).get("name") == "client_bench":
            exe = msg.get("executable")
            if exe:
                return Path(exe)
    raise SystemExit("bench-check: cargo built client_bench but named no executable")


def count(exe: Path, which: str, iters: int) -> int:
    """Instructions the bench binary retires running ONE loop `iters` times."""
    proc = subprocess.run(
        ["/usr/bin/time", "-l", str(exe), f"--only={which}", f"--iters={iters}"],
        capture_output=True,
        text=True,
        errors="replace",
        stdin=subprocess.DEVNULL,
        check=False,
    )
    if proc.returncode != 0:
        sys.stderr.write(proc.stdout[-1000:] + proc.stderr[-2000:])
        raise SystemExit(f"bench-check: the {which} loop did not run")
    got = instructions(proc.stderr)
    if got is None:
        raise SystemExit(
            "bench-check: `/usr/bin/time -l` reported no `instructions retired` -- this gate "
            "needs macOS's time(1); it cannot judge a latency budget from wall time"
        )
    return got


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    exe = bench_binary()
    counts = {name: per_op(count(exe, name, ITERS), count(exe, name, 2 * ITERS), ITERS) for name in BUDGETS}
    passed, line = verdict(counts)
    if passed:
        print("bench-check: " + line)
        return 0
    print("bench-check FAILED: " + line, file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
