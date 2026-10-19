"""install.sh replaces the installed binary by RENAME, never by rewriting it.

macOS caches a binary's code signature per vnode. `cp -f` over a binary that
has already been executed rewrites the same inode, the cache goes on
describing the old bytes, and every later exec of that path is SIGKILLed
(exit 137). That is how the installed multitop was dead on 2026-10-19 while a
copy of the same bytes ran -- and replacing the inode alone revived it. The
property that rules the class out is checkable on any OS: the installed path
is a NEW inode after every install, and nothing staged is left behind.
"""

import shutil
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]


def _install(binary: Path, prefix: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", str(REPO / "install.sh"), "--bin", str(binary), "--prefix", str(prefix)],
        capture_output=True,
        text=True,
        stdin=subprocess.DEVNULL,
        check=False,
    )


def _runs(path: Path) -> int:
    return subprocess.run([str(path), "x"], check=False, stdin=subprocess.DEVNULL, capture_output=True).returncode


def test_a_reinstall_is_a_new_inode_that_runs(tmp_path):
    binary = tmp_path / "multitop-built"
    shutil.copyfile("/bin/echo", binary)
    binary.chmod(0o755)
    prefix = tmp_path / "bin"
    prefix.mkdir()
    first = _install(binary, prefix)
    assert first.returncode == 0, first.stderr
    installed = prefix / "multitop"
    before = installed.stat().st_ino
    assert _runs(installed) == 0
    second = _install(binary, prefix)
    assert second.returncode == 0, second.stderr
    assert installed.stat().st_ino != before, "the binary was rewritten in place, not replaced"
    assert _runs(installed) == 0, "the reinstalled binary does not run"
    assert sorted(p.name for p in prefix.iterdir()) == ["multitop"], "a staged copy was left behind"
