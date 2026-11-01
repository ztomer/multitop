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
from pathlib import Path


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


def own_rglob(root: Path, pattern: str):
    """`root.rglob(pattern)`, without the files of ANOTHER checkout nested in it.

    A git worktree under `.claude/worktrees/` (or any nested repository) is a
    second copy of the tree inside this one. A checker that walks the root sees
    every file twice: doc-paths then called `ops/triage.rs` ambiguous and failed
    every commit on main while a parallel session had a worktree open
    (2026-11-01). A directory below `root` holding a `.git` entry -- a
    worktree's `.git` file or a repository's `.git` directory -- is another
    checkout, and its files are not this repository's.
    """
    root = Path(root)
    foreign: dict[Path, bool] = {}

    def is_foreign(directory: Path) -> bool:
        if directory == root or root not in directory.parents:
            return False
        if directory not in foreign:
            foreign[directory] = (directory / ".git").exists() or is_foreign(directory.parent)
        return foreign[directory]

    for path in root.rglob(pattern):
        if not is_foreign(path.parent):
            yield path
