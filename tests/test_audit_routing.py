"""monitor's own ``cargo audit`` step must run through goh's advisory-db wrapper.

A repo that runs ``cargo audit`` bare (default mode fetches) refreshes the
SHARED ``~/.cargo/advisory-db``, rewriting it file by file, while any other
``cargo audit`` on the machine may be reading it -- the 2026-10-12 tear
(``parse error: unexpected start of advisory: ""``) between two repos pushing
at once. goh's ``lib/advisory_db.py`` runs the audit against goh's own copy
(``~/.cache/goh/advisory-db``, overridable via ``GOH_ADVISORY_DB``) under a
flock: exclusive to refresh, shared to read.

Proven on this machine 2026-10-17: a bare ``cargo audit`` changes the shared
database's bytes; the same audit through the wrapper loads
``~/.cache/goh/advisory-db`` and leaves the shared database byte-identical.
"""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GATESRC = ROOT / ".gatesrc"


def _steps_line() -> str:
    for line in GATESRC.read_text().splitlines():
        if line.startswith("GOH_CI_STEPS="):
            return line
    raise AssertionError("no GOH_CI_STEPS line in .gatesrc")


def test_the_audit_step_runs_through_gohs_advisory_db():
    line = _steps_line()
    assert 'advisory_db.py" run --fetch -- cargo audit' in line, (
        "the cargo audit step must go through goh's advisory_db.py wrapper"
    )


def test_no_bare_audit_invocation_remains():
    """Every `cargo audit` in the step list sits behind the wrapper."""
    line = _steps_line()
    hits = list(re.finditer(r"cargo audit", line))
    assert hits, "the audit step went missing from GOH_CI_STEPS"
    for m in hits:
        before = line[max(0, m.start() - 40) : m.start()]
        assert "run --fetch -- " in before, (
            f"bare cargo audit at offset {m.start()}: {before!r}"
        )
