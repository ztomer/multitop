"""tools/gate_lock.py takes its lock in the git DIR, so it works in a linked worktree.

2026-09-23: the lock was `<toplevel>/.git/gate.lock`. In a linked worktree `.git` is a file,
so every hook run there died with NotADirectoryError before a single gate ran.
"""

import os
import subprocess
from pathlib import Path

LOCK = Path(__file__).resolve().parent.parent / "tools" / "gate_lock.py"
GIT_VARS = ("GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE", "GIT_COMMON_DIR", "GIT_PREFIX")


def _env() -> dict:
    return {k: v for k, v in os.environ.items() if k not in GIT_VARS and k != "GATE_LOCK"}


def _git(cwd: Path, *args: str) -> str:
    return subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *args], cwd=cwd,
                          env=_env(), check=True, capture_output=True, text=True).stdout.strip()


def _lock(cwd: Path, action: str) -> subprocess.CompletedProcess:
    return subprocess.run(["python3", str(LOCK), action, str(os.getpid())], cwd=cwd, env=_env(),
                          capture_output=True, text=True, timeout=30)


def test_acquire_and_release_in_a_linked_worktree(tmp_path):
    main = tmp_path / "main"
    main.mkdir()
    _git(main, "init", "-q", "-b", "main")
    (main / "f").write_text("x\n")
    _git(main, "add", "f")
    _git(main, "commit", "-q", "-m", "one")
    wt = tmp_path / "wt"
    _git(main, "worktree", "add", "-q", "-b", "side", str(wt))
    assert (wt / ".git").is_file()

    got = _lock(wt, "acquire")
    assert got.returncode == 0, got.stderr
    lock = Path(_git(wt, "rev-parse", "--absolute-git-dir")) / "gate.lock"
    assert (lock / "pid").read_text().strip() == str(os.getpid())
    # Per worktree: the main checkout's lock is untouched, so it does not wait on this one.
    assert not (main / ".git" / "gate.lock").exists()

    freed = _lock(wt, "release")
    assert freed.returncode == 0, freed.stderr
    assert not lock.exists()
