#!/usr/bin/env python3
"""Run everything CI runs, before pushing.

# Why this file does almost nothing itself

It used to carry its own copies of the gates: its own emoji whitelist, its own
clippy invocation, its own idea of what "clean" meant. Every copy had drifted,
and each drift made the script *weaker* than the thing it was standing in for:

  * the emoji check matched only `[\\U00010000-\\U0010ffff]`, so it missed every
    emoji in the Basic Multilingual Plane and every escaped codepoint -- the
    exact hole `check_no_emoji.py` documents having closed;
  * clippy ran as `--all-targets` with no `--workspace` and no `--all-features`,
    against CI's `--workspace --all-targets --all-features`;
  * six of the eight real gates were simply absent;
  * the fuzz gate named a test that does not exist. `cargo test <filter>` with no
    match prints `running 0 tests` and exits 0, so it reported success for
    running nothing, every time, for as long as anyone had been running it;
  * and the whole script had been failing at its first gate, so nothing after
    that gate had run at all.

So it now *delegates*. The gates live in `tools/` and in `.githooks/pre-commit`;
this runs those, and adds only the two things neither of them covers -- the fuzz
targets and the benchmark thresholds. There is no second definition of clean to
keep in step by hand.

Usage:
    python3 scripts/local-ci.py [--fast]

`--fast` skips the fuzz and benchmark gates, which are the slow ones.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# ----------------------------------------------------------------- Kare output
#
# The house style: `→ · ✓ ✗ ⚠`, restrained colour, and nothing when the output
# is not a terminal. `tui/lib.sh` is the shell original; this is the same
# vocabulary, because a python script cannot source it.

_TTY = sys.stdout.isatty() and not os.environ.get("NO_COLOR")


def _c(code: str) -> str:
    return code if _TTY else ""


DIM, GRN, RED, YEL, BLD, OFF = (
    _c("\033[2m"),
    _c("\033[32m"),
    _c("\033[31m"),
    _c("\033[33m"),
    _c("\033[1m"),
    _c("\033[0m"),
)


def section(text: str) -> None:
    print(f"\n{BLD}{text}{OFF}")


def info(text: str) -> None:
    print(f"  {DIM}→{OFF} {text}")


def note(text: str) -> None:
    print(f"  {DIM}·{OFF} {text}")


def ok(text: str) -> None:
    print(f"  {GRN}✓{OFF} {text}")


def warn(text: str) -> None:
    print(f"  {YEL}⚠{OFF} {text}", file=sys.stderr)


def err(text: str) -> None:
    print(f"  {RED}✗{OFF} {text}", file=sys.stderr)


# --------------------------------------------------------------------- running


def run(label: str, cmd: list[str]) -> bool:
    """Run a command, printing its output only when it fails."""
    info(label)
    # `errors="replace"`: the suite prints raw terminal escapes on purpose --
    # the repaint tests feed them through a real shell -- so the captured output
    # is not valid UTF-8 and decoding it strictly crashed the runner rather than
    # reporting on the tests it had just run.
    proc = subprocess.run(
        cmd, cwd=REPO, capture_output=True, text=True, errors="replace"
    )
    if proc.returncode != 0:
        err(f"{label} failed")
        sys.stderr.write(proc.stdout[-4000:])
        sys.stderr.write(proc.stderr[-4000:])
        return False
    ok(label)
    return True


# ----------------------------------------------------------------- the gates
#
# The same list the pre-commit hook runs, in the same order, each preceded by
# its own self-test. Adding a checker to `tools/` means adding it here and in
# `.github/workflows/ci.yml`; the three lists must not disagree.

CHECKERS = [
    ("no emoji", "check_no_emoji.py"),
    ("test-only code", "check_test_only_code.py"),
    ("key hints", "check_key_hints.py"),
    ("keychain isolation", "check_keychain_isolation.py"),
    ("row 0 owner", "check_row0_owner.py"),
    ("magic numbers", "check_magic_numbers.py"),
]


def structural_gates() -> bool:
    section("structural gates")
    for label, script in CHECKERS:
        path = f"tools/{script}"
        # The self-test first, always. A checker that has quietly stopped
        # detecting reports a clean tree exactly like a clean tree does.
        if not run(f"{label} (self-test)", ["python3", path, "--self-test"]):
            return False
        if not run(label, ["python3", path]):
            return False
    return True


def rust_gates() -> bool:
    section("rust")
    return (
        run("formatting", ["cargo", "fmt", "--all", "--", "--check"])
        and run(
            "clippy",
            # Character for character what CI runs.
            [
                "cargo",
                "clippy",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--",
                "-D",
                "warnings",
            ],
        )
        and run("tests", ["cargo", "test", "--workspace", "--all-features"])
        and run("coverage (95% floor)", ["bash", "tools/coverage_check.sh"])
    )


# --------------------------------------------------------------- file length
#
# Not in CI and not in the hook, because it is a shape rule rather than a
# correctness one -- but it is a rule this repo actually follows, with five
# "split X into modules" commits behind it.

LOC_LIMIT = 500

# Production source only. A test file is a list of independent cases, and
# splitting one because it crossed a line count moves cases between files
# without making any of them clearer; `coverage_e2e.rs` is 2332 lines on
# purpose.
LOC_SCOPE = "crates/*/src/**/*.rs"

# A shrink-only ratchet, the same shape as `tools/test_only_baseline.txt`: what
# is already over the limit is recorded, and may only get smaller. A gate that
# is red the day it is written is a gate nobody runs -- which is precisely what
# happened to this one, and why nothing below it had executed in months.
LOC_BASELINE = REPO / "tools" / "loc_baseline.txt"


def read_loc_baseline() -> dict[str, int]:
    if not LOC_BASELINE.exists():
        return {}
    out = {}
    for line in LOC_BASELINE.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        path, _, count = line.rpartition(" ")
        out[path.strip()] = int(count)
    return out


def check_file_length() -> bool:
    section("file length")
    baseline = read_loc_baseline()
    over, grown, shrunk = [], [], []

    for path in sorted(REPO.glob(LOC_SCOPE)):
        rel = str(path.relative_to(REPO))
        count = len(path.read_text(encoding="utf-8").splitlines())
        allowed = baseline.get(rel)
        if allowed is None:
            if count > LOC_LIMIT:
                over.append((rel, count))
        elif count > allowed:
            grown.append((rel, count, allowed))
        elif count < allowed:
            shrunk.append((rel, count, allowed))

    for rel, count in over:
        err(f"{rel} is {count} lines (limit {LOC_LIMIT}) -- split it")
    for rel, count, allowed in grown:
        err(f"{rel} grew to {count} lines, over its recorded {allowed}")
    for rel, count, allowed in shrunk:
        note(f"{rel} is down to {count} from {allowed} -- lower it in {LOC_BASELINE.name}")

    if over or grown:
        return False
    ok(f"no production file over {LOC_LIMIT} lines that was not already")
    return True


# ------------------------------------------------------- fuzz and benchmarks


def fuzz_targets() -> list[str]:
    d = REPO / "fuzz" / "fuzz_targets"
    return sorted(p.stem for p in d.glob("*.rs")) if d.is_dir() else []


def check_fuzz() -> bool:
    """Build every fuzz target.

    Building, not running: a fuzz run has no natural end, and this gate exists
    to catch a target that stopped compiling because the thing it fuzzes changed
    shape underneath it. That is the failure that actually happens -- the
    protocol gained a field and nobody rebuilt the protocol fuzzer.

    `cargo fuzz` needs a nightly toolchain and the subcommand installed. Absent,
    this says so and is skipped: an unavailable tool is not a failing gate, but
    it must not be silently reported as a passing one either.
    """
    section("fuzz targets")
    targets = fuzz_targets()
    if not targets:
        err("no fuzz targets found -- fuzz/fuzz_targets is empty")
        return False
    probe = subprocess.run(
        ["cargo", "fuzz", "--version"],
        cwd=REPO,
        capture_output=True,
        text=True,
        errors="replace",
    )
    if probe.returncode != 0:
        warn("cargo-fuzz is not installed -- skipping (cargo install cargo-fuzz)")
        return True
    for target in targets:
        if not run(f"build {target}", ["cargo", "fuzz", "build", target]):
            return False
    return True


# Thresholds, in nanoseconds. Generous: this catches an order-of-magnitude
# regression, not a percentage point of noise on a loaded laptop.
MAX_DECODE_NS = 5_000.0
MAX_RENDER_NS = 50_000.0


def check_benchmarks() -> bool:
    section("benchmarks")
    info("client_bench")
    proc = subprocess.run(
        ["cargo", "bench", "-p", "multitop", "--bench", "client_bench"],
        cwd=REPO,
        capture_output=True,
        text=True,
        errors="replace",
    )
    if proc.returncode != 0:
        err("the benchmark did not run")
        sys.stderr.write(proc.stderr[-2000:])
        return False

    import re

    decode = re.search(r"Latency:\s+([\d.]+)\s+ns\s+/\s+packet", proc.stdout)
    render = re.search(r"Latency:\s+([\d.]+)\s+ns\s+/\s+frame", proc.stdout)
    if not decode or not render:
        # Say so rather than passing. A benchmark whose output stopped matching
        # is a threshold nobody is checking, which is the same failure as the
        # fuzz gate that named a test that did not exist.
        err("the benchmark ran but its output did not carry the two latencies")
        return False

    decode_ns, render_ns = float(decode.group(1)), float(render.group(1))
    note(f"packet decode {decode_ns:.0f} ns (limit {MAX_DECODE_NS:.0f})")
    note(f"frame render  {render_ns:.0f} ns (limit {MAX_RENDER_NS:.0f})")
    if decode_ns > MAX_DECODE_NS or render_ns > MAX_RENDER_NS:
        err("a latency is over its threshold")
        return False
    ok("within thresholds")
    return True


def main() -> int:
    # The suite's own isolation, in case a test is reached that forgot its
    # guard. Belt and braces: `check_keychain_isolation.py` is what actually
    # enforces this.
    os.environ["MULTITOP_MOCK_KEYCHAIN"] = "1"
    os.environ["CI"] = "1"
    fast = "--fast" in sys.argv

    steps = [structural_gates, check_file_length, rust_gates]
    if not fast:
        steps += [check_fuzz, check_benchmarks]

    for step in steps:
        if not step():
            print()
            err("local CI failed")
            return 1

    print()
    ok("all gates green" + (" (fuzz and benchmarks skipped)" if fast else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
