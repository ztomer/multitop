"""A release pushes its bump commit and its tag in ONE atomic push.

Before 2026-10-19 `release.sh` required `main` pushed first and then pushed the
tag alone: two pushes, and between them a window where origin had the bump
commit with no tag, or (had the tag push been first) a tag naming a commit the
branch did not have. `git push --atomic` moves both refs or neither, and the
pre-push gate runs once over the commit both name. Afterwards the script reads
both refs back from origin and refuses to go on unless they agree with HEAD.
"""

import re
from pathlib import Path

RELEASE = Path(__file__).resolve().parents[1] / "release.sh"


def test_branch_and_tag_go_in_one_atomic_push():
    text = RELEASE.read_text()
    assert re.search(r'git push --atomic origin "?HEAD:main"? "\$\{TAG\}"', text), (
        "release.sh must push HEAD:main and the tag together with --atomic"
    )
    assert 'git push origin "${TAG}"' not in text, "a lone tag push is the two-push window"


def test_an_unpushed_bump_commit_is_allowed_but_a_diverged_main_is_not():
    text = RELEASE.read_text()
    assert "main has unpushed commits" not in text, (
        "the bump commit is pushed by the atomic push, so it may be unpushed"
    )
    assert "git merge-base --is-ancestor origin/main HEAD" in text, (
        "HEAD must descend from origin/main, or the atomic push is a rewrite"
    )


def test_both_refs_are_proven_on_origin():
    text = RELEASE.read_text()
    assert "git ls-remote origin" in text and "refs/heads/main" in text, (
        "release.sh must read the branch and the tag back from origin"
    )
