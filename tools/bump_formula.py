#!/usr/bin/env python3
"""Bump a Homebrew formula's version, source tarball and prebuilt agent
resources -- in lockstep, and refusing anything it cannot prove.

# Why this is a file and not a heredoc inside release.sh

It was a quoted heredoc inside a `$( )` inside a double-quoted assignment, and
the first attempt at fixing the bug it is here to fix reintroduced an
unbalanced quote in a *comment* inside that nesting, which `bash -n` rejected
with a line number 70 lines from the cause. A transform this shape cannot be
read, and a bug in a release path is the worst place to be unable to read it.
It is also directly testable from here, which the heredoc was not.

# What it fixes

The version in release.sh was a line-order state machine: it armed `pending`
when it saw an agent `url` line and wrote that agent's sha onto the next sha
line it saw. That pairs url with digest correctly only while every url in the
file names the same tag.

It did not. Two agent urls had been left at v0.47.3 by a release four versions
earlier, so on v0.49.0 the state machine matched nothing, changed nothing,
and every assertion still passed -- `pending` had simply never been armed. The
formula shipped 0.49.0 digests beside 0.47.3 urls. `brew fetch` refused it
with a checksum mismatch, and `brew upgrade ztomer/tap/multitop` would have
failed for anyone who ran it.

The final assertion is the half that should have caught it, and it only checked
that the *tarball's* old tag was gone. A url left at a *different* old tag is
not that string, so it walked past.

# The two rules now

1. **One resource block at a time, addressed by name, with every count
   asserted.** A block that has lost its url, or gained a second sha, is
   refused rather than half-rewritten.
2. **Every tag the formula points anything at must be the tag being
   released.** Not "the old tag is gone" -- the new tag, everywhere.

Usage:
    CUR_FORMULA=<formula> NEW_TAG=vX.Y.Z TARBALL_SHA=<64hex> \\
    SHA_X64=<64hex> SHA_ARM=<64hex> python3 tools/bump_formula.py

The formula travels by environment rather than argv because it contains
quotes and newlines, and because `python3 -c "..."` inside a bash
double-quoted string is what lost those quotes the first time (v0.47.3:
"agent sha line not found").
"""

from __future__ import annotations

import os
import re
import sys

#: The agent resources this formula carries, and the digest variable each one
#: reads. The filenames carry hyphens -- `multitop-agent-x86_64-unknown-linux-musl`
#: -- so a character class over them has to as well; omitting it is what made
#: the first attempt at this fix refuse a correct formula with
#: "expected exactly one agent url, found 0", which is the assertion working.
AGENT_RESOURCES: tuple[tuple[str, str], ...] = (
    ("multitop-agent-x86_64", "SHA_X64"),
    ("multitop-agent-aarch64", "SHA_ARM"),
)


def _digest(name: str) -> str:
    """One 64-hex digest from the environment, or a refusal naming the variable."""
    var = dict(AGENT_RESOURCES)[name]
    value = os.environ.get(var, "")
    if not re.fullmatch(r"[0-9a-f]{64}", value):
        raise SystemExit(f"{var} is not a sha256: {value!r}")
    return value


def fix_resource(formula: str, name: str) -> str:
    """Rewrite one resource block: its url to the new tag, its sha to the new
    digest. Block-scoped, and every count asserted."""
    tag = os.environ["NEW_TAG"]
    pattern = re.compile(
        r'(resource\s+"%s"\s+do\n)(.*?)(\n\s*end)' % re.escape(name), re.S
    )
    match = pattern.search(formula)
    if match is None:
        raise SystemExit(f"resource block not found: {name}")
    body = match.group(2)
    body, urls = re.subn(
        r"(releases/download/)v[0-9.]+(/multitop-agent-[A-Za-z0-9_.-]+\")",
        r"\g<1>%s\g<2>" % tag,
        body,
    )
    if urls != 1:
        raise SystemExit(f"{name}: expected exactly one agent url, found {urls}")
    body, shas = re.subn(
        r'sha256 "[0-9a-f]+"', 'sha256 "%s"' % _digest(name), body, count=1
    )
    if shas != 1:
        raise SystemExit(f"{name}: expected exactly one sha, found {shas}")
    return formula[: match.start(2)] + body + formula[match.end(2) :]


def bump(formula: str) -> str:
    """The whole transform, or a refusal. Never a partial rewrite."""
    tag = os.environ["NEW_TAG"]
    tarball = re.search(r"multitop/archive/refs/tags/(v[\d.]+)\.tar\.gz", formula)
    if tarball is None:
        raise SystemExit("source tarball url not found")
    out = formula.replace(
        "multitop/archive/refs/tags/%s.tar.gz" % tarball.group(1),
        "multitop/archive/refs/tags/%s.tar.gz" % tag,
    )
    out, n = re.subn(
        r'sha256 "[0-9a-f]+"', 'sha256 "%s"' % os.environ["TARBALL_SHA"], out, count=1
    )
    if n != 1:
        raise SystemExit(f"source tarball: expected one sha, found {n}")

    for name, _ in AGENT_RESOURCES:
        out = fix_resource(out, name)

    # Rule 2. Every tag the formula points anything at is the tag being
    # released -- not merely that the previous one is gone.
    for match in re.finditer(r"releases/download/(v[0-9.]+)/multitop-agent-", out):
        if match.group(1) != tag:
            raise SystemExit(
                f"agent resource still points at {match.group(1)}, not {tag}"
            )
    for match in re.finditer(r"archive/refs/tags/(v[0-9.]+)\.tar\.gz", out):
        if match.group(1) != tag:
            raise SystemExit(
                f"source tarball still points at {match.group(1)}, not {tag}"
            )

    # Command substitution strips trailing newlines; a formula must end with one.
    if not out.endswith("\n"):
        out += "\n"
    return out


def main() -> int:
    for var in ("NEW_TAG", "TARBALL_SHA", "SHA_X64", "SHA_ARM"):
        if not os.environ.get(var):
            raise SystemExit(f"{var} is unset")
    sys.stdout.write(bump(os.environ["CUR_FORMULA"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
