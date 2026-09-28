#!/usr/bin/env python3
"""Fail if a doc tells a reader to run a path that does not exist.

`RELEASE.md` opened with "Run the gates first -- `python3 scripts/local-ci.py`".
That script was deleted on 2026-09-14 when `tools/gate.sh` replaced it, and
nothing noticed: the file was correct when written, correct for the eleven days
the script lived, and wrong from the day after with no signal at all. The first
person to follow the release process would have hit `No such file or directory`
as step one.

The same class of rot had already been found in this repo's docs, which is why
the fix is a gate and not a correction. A file that is named once drifts; a file
named once per release does not get noticed for years.

SCOPE, and the oracle that makes it survivable. Every backticked path in an
instructing doc is checked, in prose as well as in fenced blocks -- the first
version of this checker looked only inside fences, and calibrating it against
the real defect proved that wrong: `RELEASE.md`'s dead instruction was an inline
mention in a sentence, and the checker that was written for it did not see it.

That leaves the hard case, which is history. `RELEASE.md` correctly records that
`scripts/local-ci.py` and `scripts/release.py` were deleted, and a checker that
failed on the sentences explaining a deletion would push the docs toward lying
about the past to satisfy themselves. The exemption is therefore not a
hand-kept allowlist -- those rot, and a new deletion would need a human to
remember -- but the repository's own history:

    a path that git records as DELETED is a path this doc is talking about

Self-maintaining, and it cannot rot: a typo was never in git, and a file that is
deleted becomes allowed without anyone editing this file. `CHANGELOG.md` is
excluded at file scope for a related reason -- a file that has since moved must
not break the build over a stanza written when it was where the changelog says.

The remaining allowlist is for paths that were NEVER in this repository: the
reader's own `config.toml`, a hostname, a licence file that does not exist.

Run with --self-test to check the checker.
"""

from __future__ import annotations

import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

#: The docs that instruct. CHANGELOG.md is absent on purpose -- see the module
#: docstring; a moved file must not fail the build over a historical stanza.
DOCS = ("README.md", "DEVELOPMENT.md", "RELEASE.md", "docs/roadmap.md")

#: Extensions this repo owns, so a path to something in another repository is not
#: mistaken for a missing file. `Formula/multitop.rb` lives in the tap and is
#: named by RELEASE.md's manual fallback on purpose.
SUFFIXES = (
    "rs", "py", "sh", "bash", "zsh", "md", "toml", "lock", "zst",
    "json", "yml", "yaml", "txt", "cfg",
)

#: Paths that are real but were never in this repository, each with the reason.
#: Deleted paths do NOT belong here -- see `deleted_paths()`, which asks git.
#: A new entry must say why; a stale entry is rot in a different file.
ALLOW = {
    "config.toml": "the reader's own file, not the repo's -- the repo ships "
                   "config.example.toml, which IS checked",
    "LICENSE": "not present in this tree",
    "sh.rustup.rs": "a hostname -- rustup's installer. Undecidable from the "
                    "shape, since `sh` and `rs` are both real extensions",
    "gates_of_heck/gates/local_ci.sh": "lives in the gates_of_heck repo, "
                                       "reached via $GOH_DIR -- never in this "
                                       "one, so no deletion history can help",
    "gates_of_heck/tui/lib.sh": "ditto -- the shared TUI library",
}

#: `crates/agent/src/proto/mod.rs:31` -- the line number is not part of the path.
_LINE_SUFFIX = re.compile(r":\d+(?::\d+)?$")
#: A conservative path shape: no shell metacharacters, no spaces. Deliberately
#: rejects `$(brew --prefix)/bin` and friends rather than trying to parse them.
#: `~` and `.` may LEAD a token so the decision about what it means happens in
#: `candidate()`, where the context is visible. Excluding them here is what made
#: `~/.config/multitop/config.toml` arrive as `config/multitop/config.toml` --
#: indistinguishable from a repo path -- and would equally have made
#: `./release.sh` uncheckable, which is the single most important path in
#: RELEASE.md.
_TOKEN = re.compile(r"[~.A-Za-z0-9_][~.A-Za-z0-9_./\-*]*")
#: Removed before tokenizing. A url tokenizes at the `:` and leaves a bare
#: hostname that ends in a file extension -- `https://example.com/a/b.py` yields
#: `example.com/a/b.py`, which then reads as a missing file.
_URL = re.compile(r"(?:https?://|www\.)\S+")
#: A character immediately before a token that means "not a path in this repo".
#: `$` is interpolation (`$(brew --prefix)/bin`); the rest are quoting, which
#: means the token was prose inside a string rather than a path in a command.
_PRECEDED_BY = ("$", '"', "'", "<", ">")
_FENCE = re.compile(r"^\s*(?:```|~~~)")
_KEY_FILES_HEADER = re.compile(r"^\s*\|\s*(?:file|path)\s*\|", re.IGNORECASE)
#: A mention of a DELETED path is history, and history is allowed -- but only
#: when the sentence says so. Deliberately narrow, for the reason
#: `check_key_hints.py` gives: these are the shapes that have actually appeared,
#: and a scanner that guesses more widely produces false alarms nobody keeps
#: fixing. The cost is a convention -- a doc that names a removed file marks it
#: -- which is cheap, because the alternative is a checker that cannot tell
#: "run this" from "this was removed", and the original defect here was exactly
#: that sentence.
_HISTORY = re.compile(
    r"\b(deleted|removed|retired|replaced|until|formerly|used to|was|were)\b",
    re.IGNORECASE,
)


def resolves(root: Path, relative: Path) -> str | None:
    """Whether `relative` names a file under `root`, uniquely, by its tail.

    A doc may name a file relative to a base it has just stated -- "four in
    `app/views.rs`, all under `crates/multitop/src`" -- and a reader resolves
    that without trouble. Resolving from the repository root alone flagged every
    one of them, which is how this checker produced six false positives on the
    first real multi-file sentence it saw in a doc.

    So a token that is not at the root is also looked for as a SUFFIX, and
    accepted only when exactly one file in the tree ends that way. Ambiguity is
    not accepted: if two files match, the reference does not identify one, which
    is the same defect this checker exists to catch, in a different shape.
    """
    if (root / relative).exists():
        return str(relative)
    tail = relative.parts
    matches = [
        path
        for path in root.rglob(relative.name)
        if path.is_file() and path.parts[-len(tail):] == tail
    ]
    if len(matches) == 1:
        return str(matches[0].relative_to(root))
    return None


def candidate(token: str) -> str | None:
    """Return the path this token names, or None if it is not one."""
    token = _LINE_SUFFIX.sub("", token)
    if token in ALLOW:
        return None
    # Home-absolute, absolute, or walking out of the repo: the reader's machine,
    # not this tree. `./x` IS this tree, and is the form RELEASE.md uses.
    if token.startswith(("~", "/", "../")) or token == "..":
        return None
    if token.startswith("./"):
        token = token[2:]
    if "." not in token:
        return None
    if token.rsplit(".", 1)[-1] not in SUFFIXES:
        return None
    # A hostname, not a path in this repo. Only decidable for a MULTI-segment
    # path, where the first segment would be a top-level directory and none of
    # those has a dot in its name. For a single segment there is no such rule:
    # `gone.sh` and `sh.rustup.rs` are the same shape, and a file loses.
    if "/" in token and "." in token.split("/", 1)[0]:
        return None
    # A glob or a brace expansion. README draws the tree with
    # `src/{color,consts,render}.rs`, which tokenizes to a bare `.rs` -- and
    # `.rs` is not a path, it is an extension with no stem in front of it.
    if "*" in token:
        return None
    if token.rsplit("/", 1)[-1].startswith("."):
        return None
    return token


def deleted_paths(root: Path) -> set[str] | None:
    """Every path git records as deleted, or None if history is unreadable.

    This is the exemption that lets a doc SAY a file was deleted. Asking git
    rather than keeping a list is the whole point: a list needs a human to add
    to it every time something is removed, which is the same human who has to
    remember the doc still mentions it.

    STAGED deletions count, and without that this checker deadlocks: the commit
    that removes a script is gated by a checker that only believes the script is
    gone once that commit lands, so the commit can never be made. Found by
    trying to use the thing, which is the only way several of today's fixes
    were found.

    None means the query failed, and the caller must fail rather than pass --
    a checker that cannot tell history from a typo must not report clean.
    """
    names: set[str] = set()
    queries = (
        # In history: the commit already landed.
        ["git", "-C", str(root), "log", "--diff-filter=D",
         "--name-only", "--pretty=format:"],
        # Staged: the commit is what is being made right now, and the pre-commit
        # hook is running as part of making it.
        ["git", "-C", str(root), "diff", "--cached", "--diff-filter=D",
         "--name-only"],
    )
    for cmd in queries:
        try:
            out = subprocess.run(
                cmd, capture_output=True, text=True, timeout=60, check=True,
            ).stdout
        except (OSError, subprocess.SubprocessError):
            return None
        names |= {line.strip() for line in out.splitlines() if line.strip()}
    return names


def instructions(text: str):
    """Yield (lineno, token) for every path an instructing doc hands a reader.

    Inline code spans, fenced blocks, and the label column of a table whose
    header calls its first column File or Path.
    """
    in_fence = False
    in_table = False
    for lineno, line in enumerate(text.splitlines(), 1):
        if _FENCE.match(line):
            in_fence = not in_fence
            continue
        if _KEY_FILES_HEADER.match(line):
            in_table = True
            continue
        if in_table and not line.lstrip().startswith("|"):
            in_table = False
        # Strip markdown link syntax so a label is not read as a path twice.
        bare = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", line)
        # A table row's FIRST cell is the path; the rest is prose describing
        # it, and prose in a table is still prose. Reading the whole row
        # reported `logos.bin.zst` out of a row whose label was correct.
        table_row = in_table and not in_fence
        if table_row:
            cells = bare.split("|")
            bare = cells[1] if len(cells) > 1 else ""
        bare = _URL.sub(" ", bare)
        # Inline code spans are included: see the module docstring for the
        # calibration that proved the fences-only scope missed the real defect.
        for match in _TOKEN.finditer(bare):
            if match.start() and bare[match.start() - 1] in _PRECEDED_BY:
                continue
            yield lineno, match.group(0), in_fence or table_row, line


def _paragraphs(text: str) -> dict[int, str]:
    """Map each line number to the text of the paragraph containing it.

    Blank-line delimited, which is how markdown delimits a paragraph and so how
    a reader's eye delimits a sentence.
    """
    out: dict[int, str] = {}
    current: list[str] = []
    start = 1
    for lineno, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            for i in range(start, lineno):
                out[i] = "\n".join(current)
            current = []
            start = lineno + 1
            continue
        if not current:
            start = lineno
        current.append(line)
    for i in range(start, start + len(current)):
        out[i] = "\n".join(current)
    return out


def offenders(root: Path, deleted: set[str]):
    """Documented paths that neither exist nor appear in the deletion history.

    `deleted` is passed in rather than queried so the caller owns the
    unreadable-history guard: a caller that cannot ask git must fail, and a
    function that quietly treats "no answer" as "nothing was deleted" is how
    that guard gets skipped.
    """
    hits = []
    for name in DOCS:
        path = root / name
        if not path.is_file():
            continue
        text = path.read_text(encoding="utf-8")
        # A marker is looked for in the whole PARAGRAPH, not the line. Markdown
        # wraps sentences, so "the same duplication that killed" is three lines
        # above the `tools/repo_gates.sh` it killed -- and a line-local test
        # reads that correct sentence as an unmarked instruction.
        paragraphs = _paragraphs(text)
        for lineno, token, is_command, _line in instructions(text):
            named = candidate(token)
            if named is None:
                continue
            # Resolved, not merely probed: a token may be named relative to a
            # base the sentence has just stated.
            if resolves(root, Path(named)) is not None:
                continue
            # In PROSE a bare filename is shorthand -- "`build.rs` panics for
            # release" names the one build.rs in the tree and nobody reads it as
            # a path to run. In a command or a Key Files label a bare name IS
            # the thing to execute, and that is where `gone.sh` must be caught.
            if not is_command and "/" not in named:
                continue
            # The exemption is for PROSE, never for a command. "Run the gates
            # with `scripts/local-ci.py`" names a file that no longer exists and
            # is wrong however true it is that the file was deleted; the
            # sentence "`scripts/local-ci.py` was deleted on 2026-09-14" is
            # exactly right. Calibrating against the real defect is what found
            # this -- the oracle cannot tell them apart on its own, because
            # both name a path git says is gone.
            if named in deleted and not is_command:
                # Prose naming a deleted path is history. It is only history if
                # the sentence says so; otherwise it reads as an instruction,
                # which is the defect this checker exists for.
                if _HISTORY.search(paragraphs.get(lineno, "")):
                    continue
            hits.append((name, lineno, named))
    return hits


def self_test() -> int:
    """Every case below is a shape the checker must DISTINGUISH, not a shape it
    must catch. The first version of this function asserted only that dead paths
    are caught, which is the half that already worked -- and it passed while the
    checker missed the real defect three times."""
    never: set[str] = set()   # what git returns for a path never in the tree
    gone = {"scripts/local-ci.py", "tools/gone.sh"}   # what it returns for deleted
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "tools").mkdir()
        (root / "tools" / "gate.sh").write_text("#!/bin/sh\n", encoding="utf-8")
        (root / "build.sh").write_text("#!/bin/sh\n", encoding="utf-8")
        (root / "crates/agent/src/proto").mkdir(parents=True)
        (root / "crates/agent/src/proto/mod.rs").write_text("//\n", encoding="utf-8")

        def probe(doc: str, text: str, deleted=frozenset()) -> set[str]:
            """Write `text` into `doc` and report the paths it would flag."""
            (root / doc).write_text(text, encoding="utf-8")
            return {n for _, _, n in offenders(root, set(deleted))}

        def expect(label: str, flagged: set[str], got: set[str]) -> int:
            """`flagged` names the paths this case should report. Anything extra
            in `got` is a false positive, so both directions are asserted --
            a checker that flags everything passes a one-sided test."""
            if flagged <= got and not (got - flagged):
                return 0
            print(f"self-test FAILED: {label} -- wanted {sorted(flagged)}, "
                  f"got {sorted(got)}")
            return 1

        # -- the defect the checker was written for, and its three disguises --
        for label, text, flagged in (
            ("a dead script in a fenced command",
             "```bash\npython3 scripts/local-ci.py\n```\n",
             {"scripts/local-ci.py"}),
            ("a dead script in an INLINE mention, which is where the real "
             "RELEASE.md defect was",
             "Run the gates first -- `python3 scripts/local-ci.py` is "
             "everything CI runs.\n",
             {"scripts/local-ci.py"}),
            ("a dead script in a Key Files label",
             "| File | Purpose |\n|---|---|\n| `scripts/local-ci.py` | gates |\n",
             {"scripts/local-ci.py"}),
            ("a dead ./script in a command",
             "```bash\n./gone.sh v1\n```\n",
             {"gone.sh"}),
            ("a name git never recorded, in prose -- no history can save it",
             "Start with `scripts/never_existed.py`.\n",
             {"scripts/never_existed.py"}),
        ):
            rc = expect(label, flagged, probe("RELEASE.md", text, gone))
            if rc:
                return rc

        # -- the shapes that must NOT be flagged, each with its reason --
        for label, doc, text, deleted in (
            ("history in prose, marked", "RELEASE.md",
             "`scripts/local-ci.py` was deleted on 2026-09-14.\n", gone),
            ("history in prose, marker three wrapped lines away", "RELEASE.md",
             "`scripts/local-ci.py` was this file's instruction\n"
             "until 2026-09-14, when it\ndeleted.\n", gone),
            ("a bare filename in prose is shorthand, not an instruction",
             "RELEASE.md", "`build.rs` panics for release.\n", never),
            ("a brace expansion in a tree diagram", "RELEASE.md",
             "```\n  src/{color,consts,render}.rs\n  tests/*.rs\n```\n", never),
            ("a hostname ending in a file extension", "RELEASE.md",
             "```\nsh.rustup.rs | sh\n```\n", never),
            ("a home-absolute path is the reader's, not the repo's", "RELEASE.md",
             "```\ncp x ~/.config/multitop/config.toml\n```\n", never),
            ("another repository's file", "RELEASE.md",
             "```\ngit -C tap add Formula/multitop.rb\n```\n", never),
            ("a url, and the tail of one", "RELEASE.md",
             "```\nhttps://example.com/a/b.py\n```\n", never),
            ("a path in a table's purpose column, which is still prose",
             "DEVELOPMENT.md",
             "| File | Purpose |\n|---|---|\n"
             "| `tools/gate.sh` | replaces `scripts/old.sh` |\n", never),
            ("a line-numbered path resolves to the file", "RELEASE.md",
             "```\nsee crates/agent/src/proto/mod.rs:31\n```\n", never),
            ("an existing ./script in a command", "RELEASE.md",
             "```bash\n./build.sh\n```\n", never),
        ):
            rc = expect(label, set(), probe(doc, text, deleted))
            if rc:
                return rc

        # A directory that is not a repository must report NO history, so that
        # main() can refuse rather than treat "no answer" as "nothing deleted".
        if deleted_paths(root) is not None:
            print("self-test FAILED: a non-repo reported a readable history")
            return 1

    # A staged deletion counts as a deletion, which is what lets the commit
    # that removes a file pass the gate that runs inside that commit. Asserted
    # here rather than in the temp-dir cases, which are not repositories.
    staged = deleted_paths(REPO)
    if staged is None:
        print("self-test FAILED: could not query this repository")
        return 1
    for name in ("scripts/release.py", "scripts/clean_slskd_history.py"):
        if not (REPO / name).exists() and name not in staged:
            print(f"self-test FAILED: {name} is absent from the tree and from "
                  f"git's record of deletions and of staged deletions -- the "
                  f"deletion exemption has a hole and the commit that made it "
                  f"was gated by this checker")
            return 1

    # A path named relative to a base the sentence states, and one that is
    # genuinely missing. Both were false-positives in the first draft of this
    # rule, on text that was perfectly clear to a reader.
    (root / "src" / "app").mkdir(parents=True)
    (root / "src" / "app" / "views.rs").write_text("//\n", encoding="utf-8")
    (root / "RELEASE.md").write_text(
        "# Release\n\nFour in `app/views.rs`, all under `src`.\n",
        encoding="utf-8",
    )
    if offenders(root, never):
        print("self-test FAILED: a path named relative to a stated base was "
              "reported -- a reader resolves that without trouble")
        return 1

    # Ambiguity is NOT resolution. The second file has to share the whole TAIL,
    # not just the basename: `other/views.rs` does not compete with
    # `app/views.rs`, and testing against it asserted a stricter rule than the
    # one the code implements -- a test that passes for the wrong reason is the
    # same failure as one that fails.
    (root / "crates").mkdir()
    (root / "crates" / "app").mkdir()
    (root / "crates" / "app" / "views.rs").write_text("//\n", encoding="utf-8")
    if not offenders(root, never):
        print("self-test FAILED: an ambiguous tail was accepted -- two files "
              "end the same way, so the reference identifies neither")
        return 1
    (root / "crates" / "app" / "views.rs").unlink()
    (root / "crates" / "app").rmdir()
    (root / "crates").rmdir()

    print("check_doc_paths self-test: ok")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()

    deleted = deleted_paths(REPO)
    if deleted is None:
        print("doc-paths: cannot read this repository's deletion history "
              "(`git log --diff-filter=D`). Without it the checker cannot tell "
              "a doc recording a removal from a doc naming a file that never "
              "existed, and reporting clean there would be a guess.")
        return 1

    present = [name for name in DOCS if (REPO / name).is_file()]
    if not present:
        print("doc-paths: none of the instructing docs were found -- run from "
              "the repository root")
        return 1
    if len(present) < len(DOCS):
        missing = sorted(set(DOCS) - set(present))
        print(f"doc-paths: expected doc(s) missing: {', '.join(missing)}")
        return 1

    hits = offenders(REPO, deleted)
    for name, lineno, named in hits:
        print(f"  {name}:{lineno}: `{named}` does not exist")
    if hits:
        print(f"doc-paths: {len(hits)} documented path(s) do not exist. A reader "
              f"following these docs runs a command that cannot work. Fix the "
              f"path, or delete the line -- a doc that no longer applies is "
              f"worse than no doc, because it is trusted.")
        return 1
    print(f"check_doc_paths: ok ({len(present)} docs, "
          f"{len(ALLOW)} allowlisted path(s))")
    return 0


if __name__ == "__main__":
    sys.exit(main())
