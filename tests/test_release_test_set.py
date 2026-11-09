"""A release runs CI's test set, by CI's own command strings.

`release.sh` said its gates mirrored CI and ran `cargo test --workspace`, a
command neither CI nor the pre-push hook ever runs: they run the suite once,
instrumented, under `bash tools/coverage_check.sh`, plus the doc tests. So the
release was the one place a plain macOS run happened, and the one place a test
failed that nothing else saw (v0.69.2, 2026-11-08; M.RELFAIL, M.CIPARITY).

House rule 17 is "local gate identical to CI". This holds it for the release:
every test step `release.sh` runs is, to the letter, a `GOH_CI_STEPS` token,
the CI workflow runs the same string, every test token in `GOH_CI_STEPS` is in
the release list, and no other line of `release.sh` runs a test.
"""

from __future__ import annotations

import re
import shutil
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
RELEASE = REPO / "release.sh"
GATESRC = REPO / ".gatesrc"
WORKFLOW = REPO / ".github" / "workflows" / "ci.yml"

# What makes a step a test step: it runs cargo's test harness, directly or
# through the coverage gate (which runs `cargo test` instrumented).
TEST_STEP = re.compile(r"\bcargo\s+(?:\+\S+\s+)?(?:test|nextest|llvm-cov)\b|coverage_check\.sh")
ARRAY = re.compile(r"^RELEASE_TEST_STEPS=\(\n(.*?)^\)", re.MULTILINE | re.DOTALL)


def _live(text: str) -> list[str]:
    return [line for line in text.splitlines() if not line.lstrip().startswith("#")]


def release_steps(text: str) -> list[str]:
    m = ARRAY.search(text)
    return re.findall(r"'([^']*)'", m.group(1)) if m else []


def gatesrc_tokens() -> list[str]:
    bash = shutil.which("bash") or "/bin/bash"
    proc = subprocess.run(  # noqa: S603 -- a fixed argv: bash sourcing this repo's .gatesrc
        [bash, "-c", f"set -euo pipefail; source '{GATESRC}'; printf '%s' \"$GOH_CI_STEPS\""],
        capture_output=True,
        text=True,
        stdin=subprocess.DEVNULL,
        check=True,
    )
    return [t for t in proc.stdout.split(":") if t]


def ci_run_lines() -> list[str]:
    """Every shell line the workflow runs: `run: x` and the body of `run: |`."""
    lines, in_block, indent = [], False, 0
    for raw in WORKFLOW.read_text(encoding="utf-8").splitlines():
        stripped = raw.strip()
        if in_block:
            if stripped and len(raw) - len(raw.lstrip()) <= indent:
                in_block = False
            elif stripped and not stripped.startswith("#"):
                lines.append(stripped)
                continue
            else:
                continue
        m = re.match(r"^(\s*)(?:-\s+)?run:\s*(.*)$", raw)
        if m:
            if m.group(2) in ("|", ">"):
                in_block, indent = True, len(m.group(1))
            elif m.group(2):
                lines.append(m.group(2).strip())
    return lines


def problems(release_text: str, tokens: list[str], ci_lines: list[str]) -> list[str]:
    out = []
    steps = release_steps(release_text)
    if not steps:
        out.append("release.sh has no RELEASE_TEST_STEPS list")
    test_tokens = [t for t in tokens if TEST_STEP.search(t)]
    for s in steps:
        if s not in tokens:
            out.append(f"release step {s!r} is not a GOH_CI_STEPS token")
        if s not in ci_lines:
            out.append(f"release step {s!r} is not a command the CI workflow runs")
    for t in test_tokens:
        if t not in steps:
            out.append(f"GOH_CI_STEPS test step {t!r} is not run by release.sh")
    # Any other line that runs a test is a test set CI does not have.
    in_array = False
    for line in _live(release_text):
        if line.startswith("RELEASE_TEST_STEPS=("):
            in_array = True
        if in_array:
            in_array = line.strip() != ")"
            continue
        if TEST_STEP.search(line):
            out.append(f"release.sh runs a test outside RELEASE_TEST_STEPS: {line.strip()}")
    return out


def _check(ok: object, msg: object) -> None:
    if not ok:
        raise AssertionError(msg)


def test_release_runs_cis_test_set() -> None:
    found = problems(RELEASE.read_text(encoding="utf-8"), gatesrc_tokens(), ci_run_lines())
    _check(not found, "\n".join(found))


def test_the_check_refuses_the_old_plain_run() -> None:
    """Calibration: the release.sh this replaced must be refused."""
    old = (
        'info "running the gates (mirrors CI) ..."\n'
        'step "cargo test --workspace" cargo test --workspace || die "cargo test failed"\n'
    )
    tokens = ["bash tools/coverage_check.sh", "cargo test --workspace --all-features --doc"]
    found = problems(old, tokens, tokens)
    _check(any("no RELEASE_TEST_STEPS" in p for p in found), found)
    _check(any("outside RELEASE_TEST_STEPS" in p for p in found), found)
    _check(any("is not run by release.sh" in p for p in found), found)


def test_the_check_refuses_a_respelled_or_missing_step() -> None:
    tokens = ["bash tools/coverage_check.sh", "cargo test --workspace --all-features --doc"]
    respelled = "RELEASE_TEST_STEPS=(\n  'bash tools/coverage_check.sh'\n  'cargo test --workspace --doc'\n)\n"
    found = problems(respelled, tokens, tokens)
    _check(any("not a GOH_CI_STEPS token" in p for p in found), found)
    _check(any("not run by release.sh" in p for p in found), found)
    agreeing = "RELEASE_TEST_STEPS=(\n" + "".join(f"  '{t}'\n" for t in tokens) + ")\n"
    _check(problems(agreeing, tokens, tokens) == [], "agreeing lists were refused")
    _check(any("CI workflow" in p for p in problems(agreeing, tokens, tokens[:1])), "a step CI lacks passed")


def test_ci_lines_are_read_from_block_and_inline_runs() -> None:
    lines = ci_run_lines()
    for want in ("bash tools/coverage_check.sh", "cargo test --workspace --all-features --doc"):
        _check(want in lines, f"{want!r} not read from the workflow: {lines}")
