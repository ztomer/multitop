"""G3: a release bumps the tap through the tap's own gate, and the redundant
transform is gone.

`release.sh` used to rewrite Formula/multitop.rb itself (tools/bump_formula.py)
and push it with `gh api -X PUT`, so the tap's own gate never ran on the bump
-- the exact hole goh 6.8 closed for five products (BACKLOG 6.8). The bump is
now one call to goh's `tap_bump.sh`, which clones the tap, moves the version,
every url and every sha256 together (tap_rewrite.py: a resource gets ITS
artifact's digest via --resource NAME=PATH|URL, which is how the formula's
two agent resources move in lockstep), commits, runs the tap's
`tools/gate.sh --full`, and pushes pinned. A red gate pushes nothing.
`--dry-run` is plumbed through and tap_bump's dry-run line names the tap gate
step it would run.

The local transform and its test are deleted in the same shape: two mechanisms
for one bump is how the 0.47.3 stale-url bug happened, and a dead transform
passes forever.
"""

from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
RELEASE = REPO / "release.sh"


def test_release_delegates_the_tap_bump_to_tap_bump_sh():
    text = RELEASE.read_text()
    assert "tap_bump.sh" in text, "release.sh must call the house tap_bump.sh"
    assert 'gh api -X PUT "repos/${TAP}/contents/${FORMULA_PATH}"' not in text, (
        "the formula must not be pushed around the tap's own gate any more"
    )


def test_release_supports_dry_run_and_forwards_it():
    text = RELEASE.read_text()
    assert "--dry-run" in text, "release.sh must accept --dry-run"
    assert 'TAP_BUMP_ARGS' in text, (
        "the tap bump arguments must be built up before the call so --dry-run is forwarded"
    )


def test_the_redundant_formula_transform_gone():
    assert not (REPO / "tools" / "bump_formula.py").is_file(), (
        "tools/bump_formula.py duplicated the house tap_rewrite.py -- one "
        "mechanism for the bump, or we ship 0.47.3 again"
    )
    assert not (REPO / "tests" / "test_bump_formula.py").is_file(), (
        "the transform's test cannot outlive the transform"
    )


def test_the_two_agent_resources_move_by_their_own_artifacts():
    text = RELEASE.read_text()
    for name in (
        "multitop-agent-x86_64=",
        "multitop-agent-aarch64=",
    ):
        assert name in text, (
            f"the --resource flag must name {name} so its digest follows ITS artifact"
        )
