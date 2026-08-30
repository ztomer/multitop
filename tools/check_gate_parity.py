#!/usr/bin/env python3
"""Keep the three lists of gates identical.

The gates are named in three places -- the pre-commit hook, the CI workflow, and
`scripts/local-ci.py` -- and each place is a list somebody has to remember to
add to. They had already drifted, in the direction that matters:

  * `DEVELOPMENT.md` said "three gates" and listed four; there were six;
  * `scripts/local-ci.py` ran two of them, one with a weaker command than CI's,
    and a third that named a test which does not exist;
  * a gate present locally and absent from CI is a merge gate that does less
    than the hook it is meant to backstop, and a gate present in CI and absent
    locally is a red build nobody saw coming.

A comment saying "these lists must not disagree" is not a gate. This is.

Usage:
    python3 tools/check_gate_parity.py [--self-test]

Exit status is 1 when the lists differ.
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

HOOK = REPO / ".githooks" / "pre-commit"
# ci.yml is renamed ci.yml.disabled when GitHub Actions is unavailable (no
# credits on this account). The parity requirement does not go away when the
# workflow stops running -- it is how the local gate keeps its shape, and it is
# what tells you what to re-enable. Resolve either name; fail loudly if neither
# is present, because a parity check that silently finds nothing to compare is
# worse than no parity check.
def _workflow() -> Path:
    workflows = REPO / ".github" / "workflows"
    for name in ("ci.yml", "ci.yml.disabled"):
        candidate = workflows / name
        if candidate.is_file():
            return candidate
    raise SystemExit(
        f"check_gate_parity: no ci.yml or ci.yml.disabled under {workflows}"
    )


WORKFLOW = _workflow()
LOCAL_CI = REPO / "scripts" / "local-ci.py"

# This checker cannot sensibly require itself to be listed the same way by the
# thing that runs it, and `coverage_check.sh` is a shell script rather than one
# of the python checkers. Both are still run everywhere; they are just not part
# of the comparison.
EXEMPT = {"check_gate_parity.py"}

# `[a-z0-9_]` -- with `[a-z_]` this silently missed `check_row0_owner.py`,
# whose name has a digit in it, and reported it as absent from all three
# lists that do in fact run it. A checker whose pattern cannot match the
# names it is looking for reports drift that is not there, which is the
# fastest way to get a gate switched off.
CHECKER = re.compile(r"tools/(check_[a-z0-9_]+\.py)")
# `("label", "check_x.py")` -- how `local-ci.py` names them.
LOCAL_CI_CHECKER = re.compile(r'"(check_[a-z0-9_]+\.py)"')

# The end-to-end suites are matched as a *directory*, not as a list of files.
#
# A per-file list is one more thing to drift: someone adds `tests/test_new.py`,
# names it in two of the three places, and the third quietly stops covering it.
# Requiring `pytest tests/` instead means a suite is covered the moment the file
# exists, and there is nothing to keep in step.
PYTEST_DIR = re.compile(r"pytest[^\n]*[\"' ]tests/")

# The two gates that used to live only in `scripts/local-ci.py`, each with a
# stated reason, and each outlived by an event: the ratchet went red on a commit
# the hook passed, and `fuzz_proto` stopped compiling and reached a release.
#
# Matched by the thing that does the work rather than by a word, so a mention in
# a comment cannot satisfy them. The fuzz pattern accepts either form on
# purpose: the hook runs a plain `cargo check` of the crate (seconds) and CI
# runs the full sanitizer build (minutes), which is a deliberate difference, not
# drift -- what matters is that each of the three compiles the fuzz crate
# somehow.
RATCHET_RUN = re.compile(r"ratchet_check\.py")
# A *command*, not a path. The first version of this also accepted the bare
# string `fuzz_targets`, which the CI workflow contains in the glob it loops
# over -- so deleting the build command left the rule satisfied by the loop that
# no longer built anything. A matcher loose enough to be safe is loose enough to
# see nothing; this one was proven to fail before it was trusted to pass.
# Three spellings, because the three places genuinely invoke it three ways:
# a shell line in CI, a shell line in the hook, and a python argument list in
# local-ci where the words are separate quoted strings. Matching only the shell
# form reported local-ci as not running a gate it has always run.
FUZZ_RUN = re.compile(
    r"cargo\s+(?:\+\S+\s+)?fuzz\s+build"          # shell: full sanitizer build
    r"|cargo\s+check[^\n]*fuzz/Cargo\.toml"          # shell: cheap compile check
    r"|\"fuzz\",\s*\"build\""                        # python argv list
)


def named_in(text: str, pattern: re.Pattern[str]) -> set[str]:
    return {m for m in pattern.findall(text)} - EXEMPT


def gates() -> dict[str, set[str]]:
    return {
        "pre-commit hook": named_in(HOOK.read_text(encoding="utf-8"), CHECKER),
        "CI workflow": named_in(WORKFLOW.read_text(encoding="utf-8"), CHECKER),
        "local-ci.py": named_in(LOCAL_CI.read_text(encoding="utf-8"), LOCAL_CI_CHECKER),
    }


def on_disk() -> set[str]:
    """Every checker that exists, whether or not anything runs it."""
    return {p.name for p in REPO.glob("tools/check_*.py")} - EXEMPT


def suites_on_disk() -> set[str]:
    """Every python end-to-end suite that exists."""
    return {p.name for p in REPO.glob("tests/test_*.py")}


def runs_pytest(text: str) -> bool:
    return PYTEST_DIR.search(text) is not None


def always_run_problems() -> list[str]:
    """Gates that must be in all three, matched by their command.

    A gate in one list and not the others is the thing this checker exists for.
    These two were exceptions with reasons; the reasons expired, and an
    expired exception that nothing re-checks is just a hole.
    """
    problems = []
    places = (
        ("pre-commit hook", HOOK),
        ("CI workflow", WORKFLOW),
        ("local-ci.py", LOCAL_CI),
    )
    for label, pattern in (("the line-count ratchet", RATCHET_RUN),
                           ("the fuzz targets", FUZZ_RUN)):
        for name, path in places:
            if not pattern.search(path.read_text(encoding="utf-8")):
                problems.append(f"{name} does not run {label}")
    return problems


def suite_problems() -> list[str]:
    """Whether all three run the python suites at all.

    These are the layer that drives the built binary rather than a function --
    the live exec channel against real hosts, and the app itself in a real
    terminal. Nothing ran them when they were written: `cargo test` does not,
    and neither did the hook, CI or this script. A suite nothing runs is a suite
    that rots, and this repo has the receipts for what that costs.
    """
    if not suites_on_disk():
        return []
    missing = [
        name
        for name, path in (
            ("pre-commit hook", HOOK),
            ("CI workflow", WORKFLOW),
            ("local-ci.py", LOCAL_CI),
        )
        if not runs_pytest(path.read_text(encoding="utf-8"))
    ]
    return [f"{name} does not run the python suites under tests/" for name in missing]


def differences(found: dict[str, set[str]], existing: set[str]) -> list[str]:
    problems = []
    for name, listed in sorted(found.items()):
        missing = existing - listed
        extra = listed - existing
        if missing:
            problems.append(f"{name} does not run: {', '.join(sorted(missing))}")
        if extra:
            problems.append(f"{name} runs a checker that does not exist: {', '.join(sorted(extra))}")
    return problems


def self_test() -> int:
    """Prove it still detects before trusting it to report clean."""
    clean = {"a": {"check_x.py", "check_y.py"}, "b": {"check_x.py", "check_y.py"}}
    if differences(clean, {"check_x.py", "check_y.py"}):
        print("gate-parity self-test: agreeing lists were reported as differing", file=sys.stderr)
        return 1

    drifted = {"a": {"check_x.py", "check_y.py"}, "b": {"check_x.py"}}
    if not differences(drifted, {"check_x.py", "check_y.py"}):
        print("gate-parity self-test: a missing gate was NOT reported", file=sys.stderr)
        return 1

    ghost = {"a": {"check_x.py", "check_gone.py"}}
    if not differences(ghost, {"check_x.py"}):
        print("gate-parity self-test: a checker that does not exist was NOT reported", file=sys.stderr)
        return 1

    # And that it reads a real hook-shaped file rather than only its own dicts.
    with tempfile.TemporaryDirectory() as tmp:
        f = Path(tmp) / "hook"
        # A name with a digit in it, deliberately: the first version of this
        # pattern could not match one.
        f.write_text(
            "python3 tools/check_no_emoji.py --self-test\n"
            "python3 tools/check_row0_owner.py\n"
        )
        if named_in(f.read_text(), CHECKER) != {"check_no_emoji.py", "check_row0_owner.py"}:
            print("gate-parity self-test: the hook parser stopped finding checkers", file=sys.stderr)
            return 1

    # And that the pytest requirement detects both ways.
    if not runs_pytest('python3 -m pytest tests/ -q'):
        print("gate-parity self-test: a real pytest invocation was NOT recognised", file=sys.stderr)
        return 1
    if not runs_pytest('run("e2e", ["python3", "-m", "pytest", "tests/", "-q"])'):
        print("gate-parity self-test: the local-ci form was NOT recognised", file=sys.stderr)
        return 1
    if runs_pytest("cargo test --workspace"):
        print("gate-parity self-test: a cargo run was mistaken for pytest", file=sys.stderr)
        return 1

    print("gate-parity self-test: passed")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()

    existing = on_disk()
    problems = differences(gates(), existing) + suite_problems() + always_run_problems()
    if not problems:
        print(
            f"gate-parity: clean ({len(existing)} checkers and "
            f"{len(suites_on_disk())} e2e suites, run everywhere)"
        )
        return 0

    print("gate-parity: the three lists of gates disagree\n")
    for p in problems:
        print(f"  {p}")
    print(
        "\nA checker in tools/ has to be named in all three:\n"
        "  .githooks/pre-commit      -- so it blocks the commit\n"
        "  .github/workflows/ci.yml  -- so it blocks the merge\n"
        "  scripts/local-ci.py       -- so it can be run before pushing\n"
        "\nA gate that only one of them runs is a gate that only sometimes runs.\n"
        "The python suites under tests/ are required the same way, as a\n"
        "directory rather than a list, so a new one cannot be forgotten."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
