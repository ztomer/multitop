#!/usr/bin/env python3
"""
Deterministic release script for multitop.

Usage (from repo root):
    python3 scripts/release.py v0.10.0

Idempotent — safe to re-run. Skips already-completed steps.

Prerequisites:
  - gh (GitHub CLI), git, sha256sum on PATH and authenticated
  - Tag already exists locally and on origin
  - SSH push access to ztomer/homebrew-tap
"""

import os
import re
import sys
import json
import shutil
import hashlib
import subprocess
import tempfile
import urllib.request
from pathlib import Path

TAP_REPO = "ztomer/homebrew-tap"
FORMULA_PATH = "Formula/multitop.rb"
TAP_BRANCH = "main"
REPO = "ztomer/multitop"

C = dict(
    info="→", ok="✓", warn="⚠", die="✗"
)


def emit(label, msg):
    print(f"  {C[label]} {msg}")


def info(msg): emit("info", msg)
def ok(msg): emit("ok", msg)
def warn(msg): emit("warn", msg)
def die(msg): emit("die", msg); sys.exit(1)


def check(*args, **kw):
    kw.setdefault("text", True)
    cap = kw.pop("capture_output", True)
    return subprocess.run(args, capture_output=cap, **kw)


def gh(*args, **kw):
    kw.setdefault("capture_output", True)
    kw.setdefault("text", True)
    return subprocess.run(["gh"] + list(args), **kw)


# ---------------------------------------------------------------------------

def step_verify_tag(tag: str):
    r = check("git", "tag", "-l", tag)
    if tag not in r.stdout.split():
        die(f"Tag {tag} not found. Run: git tag -a {tag} -m 'release: {tag}' && git push origin {tag}")

    r = check("git", "ls-remote", "--tags", "origin", tag)
    if tag not in r.stdout:
        die(f"Tag {tag} not on origin. Push first: git push origin {tag}")

    ok(f"tag {tag} verified")


def step_release_notes(tag: str) -> str:
    # Find previous tag
    r = check("git", "tag", "-l", "--sort=-version:refname")
    tags = [t for t in r.stdout.strip().split() if t.startswith("v")]
    prev = None
    for t in tags:
        if t != tag:
            prev = t
            break

    if not prev:
        prev = check("git", "rev-list", "--max-parents=1", "HEAD").stdout.strip()

    info(f"building release notes (since {prev})")

    # Main commit summaries
    r = check("git", "log", "--oneline", f"{prev}..HEAD", "--format=- %s")
    commits = r.stdout.strip()

    # Detail body from the release commit
    r = check("git", "log", f"{prev}..HEAD", "--format=%b")
    detail = r.stdout.strip()

    notes = f"## What's new\n\n{commits}\n"
    if detail:
        notes += f"\n### Details\n\n{detail}\n"

    return notes


def step_github_release(tag: str, notes: str):
    r = gh("release", "view", tag, "--json", "tagName")
    if r.returncode == 0:
        ok(f"GitHub release {tag} already exists")
        return

    info(f"creating GitHub release {tag}")
    with tempfile.NamedTemporaryFile(mode="w", suffix=".md", delete=False) as f:
        f.write(notes)
        p = f.name

    try:
        r = gh("release", "create", tag, "--title", tag, "--notes-file", p, "--latest")
        if r.returncode != 0:
            die(f"gh release create failed: {r.stderr}")
        ok(f"GitHub release {tag} created")
    finally:
        os.unlink(p)


def step_tarball_sha256(tag: str) -> str:
    url = f"https://github.com/{REPO}/archive/refs/tags/{tag}.tar.gz"
    info(f"downloading tarball: {url}")
    try:
        with urllib.request.urlopen(url, timeout=120) as resp:
            data = resp.read()
    except Exception as e:
        die(f"tarball download failed: {e}")
    sha = hashlib.sha256(data).hexdigest()
    ok(f"sha256 = {sha}")
    return sha


def step_homebrew(tag: str, sha256: str):
    tmp = Path(tempfile.mkdtemp(prefix="homebrew-release-"))
    try:
        clone = tmp / "tap"
        if not clone.is_dir():
            token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN") or ""
            clone_url = f"https://oauth2:{token}@github.com/{TAP_REPO}.git" if token else f"https://github.com/{TAP_REPO}.git"
            info(f"cloning {TAP_REPO}")
            check("git", "clone", clone_url, str(clone), check=True)

        formula = clone / FORMULA_PATH
        if not formula.is_file():
            die(f"formula not found: {formula}")

        content = formula.read_text()

        # Extract current version for comparison
        m = re.search(r'url ".*/archive/refs/tags/([^"/]+)\.tar\.gz"', content)
        current_ver = m.group(1) if m else None
        m = re.search(r'sha256 "([^"]+)"', content)
        current_sha = m.group(1) if m else None

        if current_ver == tag and current_sha == sha256:
            ok("Homebrew formula already up to date")
            return

        if current_ver == tag and current_sha != sha256:
            warn(f"version matches but hash differs — updating hash only")

        new = re.sub(
            r'(url ".*/archive/refs/tags/)([^"/]+)(\.tar\.gz")',
            f'\\g<1>{tag}\\g<3>',
            content,
        )
        new = re.sub(r'sha256 "[^"]*"', f'sha256 "{sha256}"', new)

        if new == content:
            die("formula unchanged after update — regex may be wrong")

        formula.write_text(new)

        info("committing and pushing formula update")
        check("git", "-C", str(clone), "add", FORMULA_PATH, check=True)
        check("git", "-C", str(clone), "commit", "-m", f"multitop: update to {tag}", check=True)
        # Re-set origin URL to use token for push
        token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN") or ""
        if token:
            push_url = f"https://oauth2:{token}@github.com/{TAP_REPO}.git"
            check("git", "-C", str(clone), "remote", "set-url", "origin", push_url, check=True)
        check("git", "-C", str(clone), "push", "origin", TAP_BRANCH, check=True)

        ok(f"homebrew formula updated and pushed to {TAP_REPO}")

    finally:
        shutil.rmtree(tmp, ignore_errors=True)


# ---------------------------------------------------------------------------

def main():
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <tag>", file=sys.stderr)
        print(f"  eg:  {sys.argv[0]} v0.10.0", file=sys.stderr)
        sys.exit(1)

    tag = sys.argv[1]
    if not tag.startswith("v"):
        tag = "v" + tag
    if not re.match(r'^v\d+\.\d+\.\d+$', tag):
        die(f"bad tag format: {tag} (expected vX.Y.Z)")

    # Prereqs
    for cmd in ("git", "gh"):
        if not shutil.which(cmd):
            die(f"required command not found: {cmd}")

    if not os.environ.get("GITHUB_TOKEN") and not os.environ.get("GH_TOKEN"):
        r = check("gh", "auth", "status")
        if r.returncode != 0:
            die("not authenticated with gh (run: gh auth login)")

    print(f"\n  Release multitop {tag}\n{'─' * 48}\n")

    step_verify_tag(tag)

    print()
    notes = step_release_notes(tag)

    print()
    step_github_release(tag, notes)

    print()
    sha256 = step_tarball_sha256(tag)

    print()
    step_homebrew(tag, sha256)

    print(f"\n{'─' * 48}")
    ok(f"multitop {tag} released")
    print(f"   brew:  brew upgrade ztomer/tap/multitop")
    print(f"   url:   https://github.com/{REPO}/releases/tag/{tag}\n")


if __name__ == "__main__":
    main()
