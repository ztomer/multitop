"""monitor's ``cargo audit`` runs through goh's advisory-db wrapper (rust_gate.sh, since GOH.EA2).

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


def test_the_audit_is_the_house_rust_gates():
    """Since goh GOH.EA2 the house rust gate audits EVERY tracked Cargo.lock (the root's and
    fuzz/'s), through advisory_db.py, and ratchets each ignore list: the hand-written root and
    `cd fuzz` audit steps were retired in favour of it."""
    assert '"$GOH_DIR/gates/rust_gate.sh"' in _steps_line(), "the house rust gate left GOH_CI_STEPS"


def test_no_bare_audit_invocation_remains():
    """Any `cargo audit` the step list still carries sits behind the wrapper."""
    line = _steps_line()
    for m in re.finditer(r"cargo audit", line):
        before = line[max(0, m.start() - 40) : m.start()]
        assert "run --fetch -- " in before, f"bare cargo audit at offset {m.start()}: {before!r}"
