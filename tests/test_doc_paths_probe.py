"""tools/_doc_paths_probe.py leaves no temp directory behind (MON.FZ2, 2026-11-01).

The self-test used `root` after its TemporaryDirectory closed, so `mkdir(parents=True)`
re-created the deleted directory and nothing ever removed it: one leaked tmp dir per run.
"""

import sys
import tempfile
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools"))
import _doc_paths_probe as probe


def test_self_test_leaks_no_temp_dir(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    scratch = tmp_path / "tmp"
    scratch.mkdir()
    monkeypatch.setattr(tempfile, "tempdir", str(scratch))
    if probe.self_test() != 0:
        pytest.fail("the self-test failed")
    if leaked := list(scratch.iterdir()):
        pytest.fail(f"self-test leaked: {leaked}")
