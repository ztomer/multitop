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
    # First: it is the one that notices this list has fallen behind the others.
    ("gate parity", "check_gate_parity.py"),
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
        and clippy_on_ci_toolchain()
        and run("tests", ["cargo", "test", "--workspace", "--all-features"])
        and run("coverage (95% floor)", ["bash", "tools/coverage_check.sh"])
    )


def clippy_on_ci_toolchain() -> bool:
    """Clippy again, under the toolchain CI uses.

    Local development runs nightly and CI runs stable, and their lint sets
    differ in *both* directions -- a lint nightly has and stable does not makes
    the name in an `#[allow]` an error there, and a lint stable has and nightly
    does not is a failure that only ever appears on the runner. Both had
    happened, and each one cost a push and a CI round trip to find.

    Skipped with a warning if stable is not installed: an absent toolchain is
    not a failing gate, but it must not be silently reported as a passing one.
    """
    probe = subprocess.run(
        ["cargo", "+stable", "--version"],
        cwd=REPO,
        capture_output=True,
        text=True,
        errors="replace",
    )
    if probe.returncode != 0:
        warn("stable toolchain not installed -- skipping CI's clippy pass")
        note("rustup toolchain install stable --component clippy")
        return True
    return run(
        "clippy (stable, as CI runs it)",
        [
            "cargo",
            "+stable",
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ],
    )


# --------------------------------------------------------------- file length
#
# Not in CI and not in the hook, because it is a shape rule rather than a
# correctness one -- but it is a rule this repo actually follows, with five
# "split X into modules" commits behind it.
#
# Delegated to the house ratchet (gates_of_heck checks/check_baseline_ratchet.py):
# tools/loc_baseline.txt holds recorded CEILINGS, shrink-only; this script only
# produces the current side -- every production file over the cap, as
# "<lines> <path>" lines. The mapping is exact:
#   * a file newly over the cap      -> NEW key      -> fail (was: `over`)
#   * a listed file above its ceiling -> grew         -> fail (was: `grown`)
#   * a listed file at/below the cap  -> vanished     -> pass (was: shrunk note)

LOC_LIMIT = 500

# Production source only. Test files are held to the same cap by the
# gates_of_heck structural gate (.gatesrc: GOH_MAX_LINES), which runs whole-tree
# on every push; this sweep stays src-scoped so its ratchet and CI's stay
# comparable.
LOC_SCOPE = "crates/*/src/**/*.rs"

LOC_BASELINE = REPO / "tools" / "loc_baseline.txt"

GOH = Path(os.environ.get("GOH_DIR", Path.home() / "Projects" / "gates_of_heck"))

# Split literal so the gate-parity scanner (which greps this file for quoted
# tools/check_*.py names) does not mistake the HOUSE checker for a local one.
_RATCHET = "check_" + "baseline_ratchet.py"

CURRENT_CMD = f"""python3 -c 'import pathlib
root = pathlib.Path(".")
print(0, "__under_cap_sentinel__")
for p in sorted(root.glob("{LOC_SCOPE}")):
    n = len(p.read_text(encoding="utf-8").splitlines())
    if n > {LOC_LIMIT}:
        print(n, p.as_posix())
'
# The sentinel keeps the current side NON-EMPTY when no file is over the cap --
# an empty current set is a precondition failure to the ratchet, and "clean"
# must not read as "cannot run". Its ceiling in loc_baseline.txt is 0, which a
# 0 value can never exceed."""


def check_file_length() -> bool:
    section("file length")
    return run(
        f"line-count ratchet ({LOC_LIMIT}-line cap, ceilings in "
        f"{LOC_BASELINE.name})",
        [
            sys.executable,
            str(GOH / "checks" / _RATCHET),
            "--baseline",
            str(LOC_BASELINE),
            "--current-from-command",
            CURRENT_CMD,
        ],
    )


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

    Invoked as `cargo +nightly fuzz` ON PURPOSE, and the sanitizer stays ON.

    rust-toolchain.toml pins the workspace to stable, because nightly ICEs on
    multitop-vault under clippy. Fuzzing is the one thing here that genuinely
    needs nightly: `-Zsanitizer=address` is a nightly-only flag, so under stable
    `cargo fuzz` dies with "1 nightly option were parsed". Stable for the
    workspace, nightly named explicitly at the single call site that requires
    it, rather than left to whatever `rustup default` happens to be.

    DO NOT "simplify" this to `cargo fuzz -s none` on stable. That does build
    (verified 2026-08-23, 48s, exit 0) and it is the wrong trade. Without
    AddressSanitizer a fuzz run only catches panics and hangs; the memory errors
    -- use-after-free, buffer overrun, uninitialised reads -- go undetected. The
    crate being fuzzed is the VAULT: aes-gcm, argon2, ed25519-dalek, keychain
    material. ASan is most of the reason to fuzz it at all, so a green fuzz gate
    without it would be worth close to nothing while looking identical.
    """
    section("fuzz targets")
    targets = fuzz_targets()
    if not targets:
        err("no fuzz targets found -- fuzz/fuzz_targets is empty")
        return False
    probe = subprocess.run(
        ["cargo", "+nightly", "fuzz", "--version"],
        cwd=REPO,
        capture_output=True,
        text=True,
        errors="replace",
    )
    if probe.returncode != 0:
        warn("cargo-fuzz is not installed -- skipping (cargo install cargo-fuzz)")
        return True
    for target in targets:
        if not run(f"build {target}", ["cargo", "+nightly", "fuzz", "build", target]):
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
