#!/usr/bin/env python3
"""The line-count ratchet, in one place so three callers cannot disagree.

`tools/loc_baseline.txt` holds recorded CEILINGS for production files already
over the 500-line cap. Shrink-only:

  * a file newly over the cap        -> a new key   -> fail
  * a listed file above its ceiling  -> it grew     -> fail
  * a listed file at or below the cap-> vanished    -> pass, and the number here
                                                       can come down

# Why this file exists

The invocation used to live inside `scripts/local-ci.py` and nowhere else, under
a comment saying the ratchet is "not in CI and not in the hook, because it is a
shape rule rather than a correctness one". That was a coherent decision and it
was outlived by an event: `diag.rs` grew past its ceiling in a commit the
pre-commit hook passed, and nothing said so until someone ran the pre-push
script by hand before a release. A shape rule enforced only by whoever remembers
to run something is not enforced.

So it moved here, and the hook and CI call it too. Extracting it rather than
copying the command three times is the point -- three copies of a gate is how
one of them ends up weaker than the others, which is the exact defect
`check_gate_parity.py` was written for.

The ratchet logic itself stays in gates_of_heck. This does not reimplement it:
the house rule is that a repo never vendors a copy of a shared checker, because
a fix there has to reach every repo without a re-install.

Usage:
    python3 tools/ratchet_check.py [--self-test]
"""

from __future__ import annotations

import os
import pathlib
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent

#: Files this size or smaller need no ceiling at all.
LOC_LIMIT = 500

#: Production source only. Test files are held to the same cap by the
#: gates_of_heck structural gate (`.gatesrc`: `GOH_MAX_LINES`), which runs
#: whole-tree; this sweep stays src-scoped so its ratchet and CI's stay
#: comparable.
LOC_SCOPE = "crates/*/src/**/*.rs"

BASELINE = REPO / "tools" / "loc_baseline.txt"

#: Split so `check_gate_parity.py` -- which greps for quoted `tools/check_*.py`
#: names -- does not mistake the HOUSE checker for a local one that ought to
#: exist in this repo's `tools/`.
RATCHET = "check_" + "baseline_ratchet.py"


def goh() -> pathlib.Path:
    return pathlib.Path(
        os.environ.get("GOH_DIR") or (pathlib.Path.home() / "Projects" / "gates_of_heck")
    )


def current_command() -> str:
    """Prints `<lines> <path>` for every production file over the cap.

    The sentinel keeps this NON-EMPTY when nothing is over the cap. An empty
    current set is a precondition failure to the ratchet, and "cannot run" must
    never read as "clean" -- its ceiling in the baseline is 0, which a 0 can
    never exceed.
    """
    return f"""python3 -c 'import pathlib
root = pathlib.Path(".")
print(0, "__under_cap_sentinel__")
for p in sorted(root.glob("{LOC_SCOPE}")):
    n = len(p.read_text(encoding="utf-8").splitlines())
    if n > {LOC_LIMIT}:
        print(n, p.as_posix())
'"""


def self_test() -> int:
    """Prove the pieces this file owns still work before trusting a clean run.

    The ratchet's own logic has its own self-test in gates_of_heck; what is
    testable here is that the sweep can still find files and that the sentinel
    survives, because a sweep that silently matches nothing reports a clean tree
    exactly as a clean tree does.
    """
    out = subprocess.run(
        ["sh", "-c", current_command()], cwd=REPO, capture_output=True, text=True
    )
    if out.returncode != 0:
        print(f"ratchet self-test: the sweep failed: {out.stderr}", file=sys.stderr)
        return 1
    lines = [ln for ln in out.stdout.splitlines() if ln.strip()]
    if not lines or "__under_cap_sentinel__" not in lines[0]:
        print("ratchet self-test: the sentinel is missing from the sweep", file=sys.stderr)
        return 1
    # And that the glob reaches real source: an empty match set is the failure
    # this cannot otherwise distinguish from a tidy repo.
    if not list(REPO.glob(LOC_SCOPE)):
        print(f"ratchet self-test: {LOC_SCOPE} matched no files", file=sys.stderr)
        return 1
    if not BASELINE.is_file():
        print(f"ratchet self-test: {BASELINE} is missing", file=sys.stderr)
        return 1
    print("ratchet self-test: passed")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()

    checker = goh() / "checks" / RATCHET
    if not checker.is_file():
        # Named, not skipped. A gate that cannot run must not report clean --
        # that is how the coverage floor sat dead in CI for weeks, exiting 127
        # against a path that does not exist on a runner.
        print(
            f"ratchet: cannot find {checker}\n"
            "  Set GOH_DIR, or clone https://github.com/ztomer/gates_of_heck.\n"
            "  A gate that cannot run is not a gate that passed.",
            file=sys.stderr,
        )
        return 1

    return subprocess.run(
        [
            sys.executable,
            str(checker),
            "--baseline",
            str(BASELINE),
            "--current-from-command",
            current_command(),
        ],
        cwd=REPO,
    ).returncode


if __name__ == "__main__":
    sys.exit(main())
