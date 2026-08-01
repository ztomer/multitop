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

C = dict(info="→", ok="✓", warn="⚠", die="✗")


def emit(label, msg):
    print(f"  {C[label]} {msg}")


def info(msg):
    emit("info", msg)


def ok(msg):
    emit("ok", msg)


def warn(msg):
    emit("warn", msg)


def die(msg):
    emit("die", msg)
    sys.exit(1)


def check(*args, **kw):
    kw.setdefault("text", True)
    cap = kw.pop("capture_output", True)
    return subprocess.run(args, capture_output=cap, **kw)


def gh(*args, **kw):
    kw.setdefault("capture_output", True)
    kw.setdefault("text", True)
    return subprocess.run(["gh"] + list(args), **kw)


def github_token() -> str:
    """A token with push access to the tap.

    Prefer the environment, but fall back to the `gh` CLI's own credential.
    `gh auth login` commonly stores the token in the system keyring with no
    env var set at all, and without this fallback the tap push failed with an
    opaque auth error at the very last step of the release.
    """
    env = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if env:
        return env
    r = check("gh", "auth", "token")
    return r.stdout.strip() if r.returncode == 0 else ""


# ---------------------------------------------------------------------------


def step_cut(tag: str):
    """Bump the version, commit, tag, and push — the steps that used to be
    copy-pasted out of RELEASE.md by hand.

    Doing it by hand is how v0.21.0 and v0.22.0 ended up tagged but never
    released, and how Cargo.lock drifted from Cargo.toml.
    """
    version = tag.lstrip("v")

    r = check("git", "status", "--porcelain")
    if r.stdout.strip():
        die("working tree is dirty — commit or stash before cutting a release")

    branch = check("git", "rev-parse", "--abbrev-ref", "HEAD").stdout.strip()

    root = Path(__file__).resolve().parent.parent
    manifest = root / "Cargo.toml"
    content = manifest.read_text()

    new_content, n = re.subn(
        r'(?m)^(version = ")[^"]+(")', f"\\g<1>{version}\\g<2>", content, count=1
    )
    if n != 1:
        die("could not find the workspace version line in Cargo.toml")

    if new_content == content:
        ok(f"Cargo.toml already at {version}")
    else:
        manifest.write_text(new_content)
        ok(f"Cargo.toml bumped to {version}")

    # Refresh Cargo.lock so it does not drift from the manifest. A stale lock
    # breaks any --locked build (CI, Homebrew) with a confusing error.
    info("refreshing Cargo.lock")
    r = check("cargo", "metadata", "--format-version", "1", "--offline", cwd=str(root))
    if r.returncode != 0:
        r = check("cargo", "metadata", "--format-version", "1", cwd=str(root))
    if r.returncode != 0:
        die("cargo metadata failed — cannot refresh Cargo.lock")

    r = check("git", "status", "--porcelain")
    if r.stdout.strip():
        check("git", "add", "Cargo.toml", "Cargo.lock", check=True)
        r = check("git", "commit", "-m", f"chore: release {tag}")
        if r.returncode != 0:
            die(f"commit failed (pre-commit gates?): {r.stdout}{r.stderr}")
        ok(f"committed version bump {tag}")
    else:
        ok("nothing to commit — version already recorded")

    r = check("git", "tag", "-l", tag)
    if tag in r.stdout.split():
        ok(f"tag {tag} already exists")
    else:
        check("git", "tag", "-a", tag, "-m", f"Release {version}", check=True)
        ok(f"tagged {tag}")

    info(f"pushing {branch} and {tag}")
    if check("git", "push", "origin", branch).returncode != 0:
        die(f"failed to push {branch}")
    if check("git", "push", "origin", tag).returncode != 0:
        die(f"failed to push {tag}")
    ok(f"pushed {branch} and {tag}")


def step_verify_tag(tag: str):
    r = check("git", "tag", "-l", tag)
    if tag not in r.stdout.split():
        die(f"Tag {tag} not found. Run: git tag -a {tag} -m 'release: {tag}' && git push origin {tag}")

    r = check("git", "ls-remote", "--tags", "origin", tag)
    if tag not in r.stdout:
        die(f"Tag {tag} not on origin. Push first: git push origin {tag}")

    ok(f"tag {tag} verified")


def previous_ref(tag: str) -> str:
    """The point users are actually upgrading FROM.

    That is the last *published* release, not the last tag. Tags get created
    for versions that are then never released (it has happened twice in this
    repo), and anchoring to the newest tag silently drops every change between
    the last real release and that dead tag — exactly the changes a user
    upgrading via Homebrew is about to receive.
    """
    r = gh("api", f"repos/{REPO}/releases/latest", "--jq", ".tag_name")
    latest = r.stdout.strip()
    if r.returncode == 0 and latest and latest != tag:
        # Guard against a release whose tag is no longer in this clone.
        if check("git", "rev-parse", "--verify", f"{latest}^{{commit}}").returncode == 0:
            return latest

    r = check("git", "tag", "-l", "--sort=-version:refname")
    for t in r.stdout.strip().split():
        if t.startswith("v") and t != tag:
            return t

    return check("git", "rev-list", "--max-parents=1", "HEAD").stdout.strip()


def step_release_notes(tag: str) -> str:
    prev = previous_ref(tag)
    info(f"building release notes (since {prev}, the last published release)")

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
            token = github_token()
            clone_url = (
                f"https://oauth2:{token}@github.com/{TAP_REPO}.git" if token else f"https://github.com/{TAP_REPO}.git"
            )
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
            f"\\g<1>{tag}\\g<3>",
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
        token = github_token()
        if token:
            push_url = f"https://oauth2:{token}@github.com/{TAP_REPO}.git"
            check("git", "-C", str(clone), "remote", "set-url", "origin", push_url, check=True)
        check("git", "-C", str(clone), "push", "origin", TAP_BRANCH, check=True)

        ok(f"homebrew formula updated and pushed to {TAP_REPO}")

    finally:
        shutil.rmtree(tmp, ignore_errors=True)


# ---------------------------------------------------------------------------


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    flags = {a for a in sys.argv[1:] if a.startswith("--")}

    if not args or flags - {"--cut"}:
        print(f"Usage: {sys.argv[0]} <version> [--cut]", file=sys.stderr)
        print(f"  eg:  {sys.argv[0]} v0.23.0 --cut   bump, commit, tag, push, release", file=sys.stderr)
        print(f"       {sys.argv[0]} v0.23.0         release an already-pushed tag", file=sys.stderr)
        sys.exit(1)

    tag = args[0]
    if not tag.startswith("v"):
        tag = "v" + tag
    if not re.match(r"^v\d+\.\d+\.\d+$", tag):
        die(f"bad tag format: {tag} (expected vX.Y.Z)")

    # Prereqs
    for cmd in ("git", "gh"):
        if not shutil.which(cmd):
            die(f"required command not found: {cmd}")

    if not github_token():
        die("no GitHub token available (run: gh auth login)")

    print(f"\n  Release multitop {tag}\n{'─' * 48}\n")

    if "--cut" in flags:
        step_cut(tag)
        print()

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
