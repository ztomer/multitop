"""The Homebrew formula bump, tested against the input that broke the release.

`tools/bump_formula.py` runs on the path between a tag and a working
`brew install`, which is the worst place for a transform to be wrong in a way
nobody notices until a user hits it. On v0.49.0 it was wrong in exactly that
way: it shipped 0.49.0 digests beside v0.47.3 agent urls, and every assertion
in the version it replaced still passed. These are those assertions' test.

Collected by `pytest tests/`, which is the step .gatesrc names. The release
script is the one caller and it is emphatically not a test environment.
"""

from __future__ import annotations

import os
import sys
import unittest

# The transform lives in tools/; this lives in tests/ because that is the
# directory the pytest step in .gatesrc runs, so a new suite cannot be added
# where nothing collects it.
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "tools"))
import bump_formula  # noqa: E402

A = "a" * 64
B = "b" * 64
C = "c" * 64


def formula(tarball_tag: str, agent_tag: str, *, drop_sha: bool = False) -> str:
    """A formula shaped like the real one, with the tags as parameters.

    Shaped like the real one deliberately: `url` before `sha256` inside each
    `resource` block, which is the shape the line-order transform depended on.
    """
    arm = "" if not drop_sha else ""
    return f"""class Multitop < Formula
  url "https://github.com/ztomer/multitop/archive/refs/tags/{tarball_tag}.tar.gz"
  sha256 "{A}"
  resource "multitop-agent-x86_64" do
    url "https://github.com/ztomer/multitop/releases/download/{agent_tag}/multitop-agent-x86_64-unknown-linux-musl"
    sha256 "{B}"
  end
  resource "multitop-agent-aarch64" do
    url "https://github.com/ztomer/multitop/releases/download/{agent_tag}/multitop-agent-aarch64-unknown-linux-musl"
    {arm}sha256 "{C}"
  end
end
"""


def env(**over: str) -> dict[str, str]:
    base = {
        "CUR_FORMULA": "",
        "NEW_TAG": "v9.9.9",
        "TARBALL_SHA": "d" * 64,
        "SHA_X64": "e" * 64,
        "SHA_ARM": "f" * 64,
    }
    base.update(over)
    return base


def run(text: str, **over: str) -> str:
    previous = dict(os.environ)
    os.environ.clear()
    os.environ.update(env(CUR_FORMULA=text, **over))
    try:
        return bump_formula.bump(text)
    finally:
        os.environ.clear()
        os.environ.update(previous)


class Bump(unittest.TestCase):
    """What must happen, and what must be refused."""

    def test_an_ordinary_bump_rewrites_every_url_and_digest(self):
        out = run(formula("v0.48.1", "v0.48.1"))
        self.assertNotIn("v0.48.1", out)
        self.assertEqual(out.count("v9.9.9"), 3, out)
        # One digest per thing, each the right one -- a transform that puts the
        # same digest everywhere is as broken as one that puts none.
        for digest in ("d" * 64, "e" * 64, "f" * 64):
            self.assertEqual(out.count(digest), 1, out)

    def test_the_exact_input_that_broke_v0490_lands_correctly(self):
        """Two agent urls left at an UNRELATED old tag.

        This is v0.49.0, verbatim in shape. The tarball said v0.48.1 and the
        agents said v0.47.3, so the transform it replaced armed `pending` on
        nothing, rewrote nothing, and passed every assertion it had -- the
        formula it shipped could not be installed.

        A bump's job is to make the formula point at the new tag, so this input
        is REPAIRED rather than refused: every url is rewritten whatever old
        tag it named, and the postcondition (rule 2) is what holds it there.
        Refusing here would mean a release cannot proceed over a formula the
        tap has been carrying, unrepaired, for four versions.
        """
        out = run(formula("v0.48.1", "v0.47.3"))
        self.assertNotIn("v0.47.3", out)
        self.assertNotIn("v0.48.1", out)
        self.assertEqual(out.count("v9.9.9"), 3, out)

    def test_a_url_the_transform_cannot_reach_is_refused(self):
        """The assertion that replaces `pending is None`.

        A resource whose url does not carry the filename the pattern anchors on
        is the shape the old transform matched nothing on -- silently, because
        `pending` was never armed so it was never checked. Here the count is
        asserted, so it is a refusal.
        """
        text = formula("v0.48.1", "v0.48.1").replace(
            "releases/download/v0.48.1/multitop-agent-aarch64-unknown-linux-musl",
            "releases/download/v0.48.1/agent-aarch64.tgz",
        )
        with self.assertRaises(SystemExit) as caught:
            run(text)
        self.assertIn("url", str(caught.exception))

    def test_an_agent_resource_the_tool_does_not_know_about_is_refused(self):
        """Rule 2, and the check the old one got wrong by looking only at the
        tarball's own old tag.

        `AGENT_RESOURCES` names the two agents this formula carries. A third
        block -- someone adds a riscv64 agent -- is not in that list, so the
        block transform leaves it alone, and rule 2 is the only thing standing
        between that and a formula with a stale url in it. The old assertion
        looked for the tarball's old tag only, so a different old tag anywhere
        in the file walked straight past.
        """
        text = formula("v0.48.1", "v0.48.1") + (
            "  resource \"multitop-agent-riscv64\" do\n"
            "    url \"https://github.com/ztomer/multitop/releases/download/v0.47.3/"
            "multitop-agent-riscv64-unknown-linux-musl\"\n"
            f"    sha256 \"{'e' * 64}\"\n"
            "  end\n"
        )
        with self.assertRaises(SystemExit) as caught:
            run(text)
        self.assertIn("0.47.3", str(caught.exception))
        self.assertIn("v9.9.9", str(caught.exception))

    def test_a_resource_block_that_lost_its_digest_is_refused(self):
        with self.assertRaises(SystemExit) as caught:
            run(formula("v0.48.1", "v0.48.1", drop_sha=True).replace(f'    sha256 "{C}"', ""))
        self.assertIn("sha", str(caught.exception))

    def test_a_missing_resource_block_is_refused(self):
        text = formula("v0.48.1", "v0.48.1").replace(
            'resource "multitop-agent-aarch64" do', 'resource "multitop-agent-riscv64" do'
        )
        with self.assertRaises(SystemExit) as caught:
            run(text)
        self.assertIn("not found", str(caught.exception))

    def test_a_formula_with_no_tarball_is_refused(self):
        with self.assertRaises(SystemExit) as caught:
            run("class Multitop < Formula\nend\n")
        self.assertIn("tarball", str(caught.exception))

    def test_a_digest_that_is_not_a_digest_is_refused(self):
        """A 64-hex check, so a 404 page's digest can never be shipped.

        The release script downloads each asset and hashes it. An HTML error
        page is non-empty and its hash is 64 hex characters, so the size and
        shape checks both pass it -- and the formula ships a digest for a file
        that is not the agent.
        """
        with self.assertRaises(SystemExit) as caught:
            run(formula("v0.48.1", "v0.48.1"), SHA_X64="not-a-digest")
        self.assertIn("SHA_X64", str(caught.exception))

    def test_the_output_ends_with_a_newline(self):
        """Command substitution strips it; a formula without one is a diff
        nobody can read."""
        self.assertTrue(run(formula("v0.48.1", "v0.48.1")).endswith("\n"))

    def test_a_bump_to_the_tag_already_present_is_a_no_op(self):
        """release.sh refuses an unchanged transform, so idempotence is what
        makes re-running the release safe."""
        once = run(formula("v0.48.1", "v0.48.1"))
        twice = run(once)
        self.assertEqual(once, twice)


if __name__ == "__main__":
    unittest.main(verbosity=2)
