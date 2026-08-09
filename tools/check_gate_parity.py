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
WORKFLOW = REPO / ".github" / "workflows" / "ci.yml"
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

    print("gate-parity self-test: passed")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()

    existing = on_disk()
    problems = differences(gates(), existing)
    if not problems:
        print(f"gate-parity: clean ({len(existing)} checkers, run everywhere)")
        return 0

    print("gate-parity: the three lists of gates disagree\n")
    for p in problems:
        print(f"  {p}")
    print(
        "\nA checker in tools/ has to be named in all three:\n"
        "  .githooks/pre-commit      -- so it blocks the commit\n"
        "  .github/workflows/ci.yml  -- so it blocks the merge\n"
        "  scripts/local-ci.py       -- so it can be run before pushing\n"
        "\nA gate that only one of them runs is a gate that only sometimes runs."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
