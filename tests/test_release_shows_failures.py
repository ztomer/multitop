"""A release step that fails shows its output.

`./release.sh v0.69.2` stopped at "cargo test failed — nothing tagged" with
the test's output sent to /dev/null, so nobody could see which test failed
(2026-11-08, M.RELFAIL). Every gated step (`... || die`) now runs through
`step`, which is quiet on success and prints the output's tail on failure.
This test refuses a gated line that discards its output instead; the only
exemptions are presence probes whose failure message already says it all.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
from pathlib import Path

RELEASE = Path(__file__).resolve().parents[1] / "release.sh"
PROBES = ("command -v ",)
GUARD_EXIT = 7


def _code_lines() -> list[tuple[int, str]]:
    return [
        (n, line.strip())
        for n, line in enumerate(RELEASE.read_text().splitlines(), 1)
        if not line.strip().startswith("#")
    ]


def _fail(msg: str) -> None:
    raise AssertionError(msg)


def test_no_gated_step_discards_its_output() -> None:
    bad = [
        f"release.sh:{n}: {line}"
        for n, line in _code_lines()
        if "|| die" in line and "/dev/null" in line and not line.startswith(PROBES)
    ]
    if bad:
        _fail("a gated step hides its output; run it through `step`:\n" + "\n".join(bad))


def _step_function() -> str:
    m = re.search(r"^step\(\) \{\n.*?^\}\n", RELEASE.read_text(), re.MULTILINE | re.DOTALL)
    if m is None:
        _fail("release.sh has no step() helper")
        return ""
    return m.group(0)


def test_step_prints_a_failing_commands_output(tmp_path: Path) -> None:
    script = tmp_path / "s.sh"
    script.write_text(
        "set -euo pipefail\n"
        + _step_function()
        + 'step "demo" sh -c "echo quiet-pass" || exit 9\n'
        + f'step "demo" sh -c "echo test it_names_the_test ... FAILED; exit 3" || exit {GUARD_EXIT}\n'
    )
    bash = shutil.which("bash") or "/bin/bash"
    r = subprocess.run(  # noqa: S603 -- a fixed argv: bash and a script this test wrote
        [bash, str(script)],
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        check=False,
        env={**os.environ, "TMPDIR": str(tmp_path)},
    )
    if r.returncode != GUARD_EXIT:
        _fail(f"step() did not report the failure: exit {r.returncode}")
    if "quiet-pass" in r.stdout + r.stderr:
        _fail("step() printed a passing command's output")
    if "it_names_the_test ... FAILED" not in r.stderr:
        _fail(f"step() hid the failing command's output: {r.stderr!r}")
