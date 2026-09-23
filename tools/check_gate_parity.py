#!/usr/bin/env python3
"""Keep the three lists of gates identical.

The gates are named in three places -- the pre-commit hook, the CI workflow, and
`GOH_CI_STEPS` in `.gatesrc` (what the pre-push hook runs) -- and two of them
are lists somebody has to remember to add to. The third runs the checkers
through `tools/checkers.sh`, which globs `tools/check_*.py`, so it is the set
on disk by construction. They had already drifted once, in the direction that
matters (when the third place was `scripts/local-ci.py`, retired 2026-09-14):

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
GATESRC = REPO / ".gatesrc"

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


def gatesrc_steps() -> str:
    """The evaluated GOH_CI_STEPS: sourced the way local_ci.sh sources it, so
    quoting and `$GOH_DIR` read as the runner sees them, not as file text."""
    import subprocess
    proc = subprocess.run(
        ["bash", "-c", f"set -euo pipefail; source '{GATESRC}'; printf '%s' \"$GOH_CI_STEPS\""],
        capture_output=True, text=True,
    )
    if proc.returncode != 0:
        return ""
    return proc.stdout


def steps_run_every_checker(steps: str, existing: set[str]) -> set[str]:
    """The step list runs the checkers through tools/checkers.sh, which globs
    tools/check_*.py -- so naming that script means every checker on disk.
    A step list that names individual checkers instead is matched literally."""
    if "tools/checkers.sh" in steps:
        return set(existing)
    return named_in(steps, CHECKER)

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
    r"|tools/fuzz_check\.sh"                         # the gate script (both layers)
)


# The hook runs the steps GOH_CI_STEPS also runs through gates_of_heck's
# proven-step cache, so the pre-push run over the same tree can skip them. The
# cache keys on the step STRING: a hook step spelled one character differently
# from its GOH_CI_STEPS twin still runs and passes, and simply never hits --
# the ~10 min coverage suite quietly runs twice per commit again, and nothing
# goes red. So every `proven_step '<step>'` in the hook must be, to the letter,
# one of the colon-separated GOH_CI_STEPS tokens.
PROVEN_STEP = re.compile(r"^\s*(?:if\s+!\s+)?proven_step\s+'([^']*)'", re.MULTILINE)


def proven_mismatches(hook_text: str, steps: str) -> list[str]:
    tokens = {t for t in steps.split(":") if t}
    return [s for s in PROVEN_STEP.findall(hook_text) if s not in tokens]


def proven_problems() -> list[str]:
    hook = HOOK.read_text(encoding="utf-8")
    problems = [
        f"pre-commit hook proves a step GOH_CI_STEPS does not spell that way: {s!r}"
        for s in proven_mismatches(hook, gatesrc_steps())
    ]
    if "proven_step" in hook and not PROVEN_STEP.search(hook):
        problems.append("pre-commit hook defines proven_step but no call was recognised")
    return problems


def named_in(text: str, pattern: re.Pattern[str]) -> set[str]:
    return {m for m in pattern.findall(text)} - EXEMPT


def gates() -> dict[str, set[str]]:
    return {
        "pre-commit hook": named_in(HOOK.read_text(encoding="utf-8"), CHECKER),
        "CI workflow": named_in(WORKFLOW.read_text(encoding="utf-8"), CHECKER),
        ".gatesrc GOH_CI_STEPS": steps_run_every_checker(gatesrc_steps(), on_disk()),
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
        (".gatesrc GOH_CI_STEPS", None),
    )
    for label, pattern in (("the line-count ratchet", RATCHET_RUN),
                           ("the fuzz targets", FUZZ_RUN)):
        for name, path in places:
            text = gatesrc_steps() if path is None else path.read_text(encoding="utf-8")
            if not pattern.search(text):
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
            (".gatesrc GOH_CI_STEPS", None),
        )
        if not runs_pytest(gatesrc_steps() if path is None else path.read_text(encoding="utf-8"))
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
    if not runs_pytest('cargo build -p multitop && python3 -m pytest tests/ -q'):
        print("gate-parity self-test: the step-list form was NOT recognised", file=sys.stderr)
        return 1
    if runs_pytest("cargo test --workspace"):
        print("gate-parity self-test: a cargo run was mistaken for pytest", file=sys.stderr)
        return 1

    # And that a proven step respelled against GOH_CI_STEPS is caught, both ways.
    steps = 'tools/checkers.sh:bash tools/coverage_check.sh'
    if proven_mismatches("if ! proven_step 'bash tools/coverage_check.sh'; then\n", steps):
        print("gate-parity self-test: a matching proven step was reported", file=sys.stderr)
        return 1
    if not proven_mismatches("if ! proven_step 'bash  tools/coverage_check.sh'; then\n", steps):
        print("gate-parity self-test: a respelled proven step was NOT reported", file=sys.stderr)
        return 1

    print("gate-parity self-test: passed")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()

    existing = on_disk()
    problems = (differences(gates(), existing) + suite_problems() + always_run_problems()
                + proven_problems())
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
        "  .gatesrc GOH_CI_STEPS     -- so the pre-push hook runs it (tools/checkers.sh globs them)\n"
        "\nA gate that only one of them runs is a gate that only sometimes runs.\n"
        "The python suites under tests/ are required the same way, as a\n"
        "directory rather than a list, so a new one cannot be forgotten."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
