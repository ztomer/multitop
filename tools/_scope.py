"""Empty-scope guard shared by this repo's checkers.

A gate that reports "clean" over zero files is indistinguishable from a
spotless tree, and it is what every checker here printed when the house
sweep (gates_of_heck/checks/check_empty_scope.py) ran them over an empty
repo on 2026-09-14. "clean (0 files checked)" is not a clean run; it is a
scope that moved out from under the gate.

    if scope_is_empty("magic-numbers", len(files), "Rust files"):
        return 1
"""
from __future__ import annotations

import sys


def scope_is_empty(gate: str, count: int, unit: str = "files") -> bool:
    """True (after printing why) when `count` is zero: the caller must fail."""
    if count > 0:
        return False
    print(
        f"{gate}: inspected 0 {unit} — the scope is gone, not clean.\n"
        "  Something this gate depends on moved: a directory rename, a changed\n"
        "  file convention, or a build that has not run yet. Fix the scope; do\n"
        "  not let a gate report compliance over an empty set.",
        file=sys.stderr,
    )
    return True
