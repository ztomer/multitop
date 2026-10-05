"""The self-proof for `tools/check_doc_paths.py`, split out on 2026-10-04.

The checker grew past the 500-line cap (house `check_file_length.py`) the day
the shallow-clone defect was fixed, and the fix was ~100 lines of assertions --
which is the whole point of this split: a self-proof is the part that grows, and
the part that grows is the part that has to be somewhere else.

It is a separate MODULE rather than a separate file for the gates' benefit
because it needs the functions it tests. So the import runs the other way --
`check_doc_paths` imports this lazily, inside `main()`, and this imports
`check_doc_paths` at module scope -- which is the only ordering that terminates.

`gates_of_heck/checks/check_probes_pass.py` finds the self-proof by the
dispatch in `check_doc_paths.py`, which is where the `--self-test` handling
stays, so the probe is still discovered and still run from where it always was.
"""

import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_doc_paths as target  # noqa: E402


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
            return {n for _, _, n in target.offenders(root, set(deleted))}

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
        if target.deleted_paths(root) is not None:
            print("self-test FAILED: a non-repo reported a readable history")
            return 1

    # The deletion exemption, against a repository this function BUILDS.
    #
    # It used to assert that `scripts/release.py` and
    # `scripts/clean_slskd_history.py` appear in *this* repository's
    # `git log`. That asserts a property of the clone, not of the code: CI
    # checks out with `fetch-depth: 1`, so the commits that deleted them are
    # outside the history the runner has, and the self-test failed there on
    # every run -- `main` red from 2026-10-01 with eleven local gates green.
    #
    # The two claims actually worth keeping are (a) a deletion that LANDED is
    # found in history, and (b) a STAGED deletion counts too, because the
    # commit that removes a file is gated by the very checker that only
    # believes it is gone once that commit lands -- so without (b) that commit
    # can never be made. Both are proved below on a throwaway repository, so
    # they hold at any clone depth, and neither depends on a filename that a
    # future commit is free to delete.
    with tempfile.TemporaryDirectory() as tmp:
        history = Path(tmp) / "repo"
        (history / "scripts").mkdir(parents=True)

        def git(*args: str) -> None:
            subprocess.run(
                ["git", "-C", str(history), *args],
                check=True, capture_output=True, text=True,
            )

        git("-c", "init.defaultBranch=main", "init", "-q")
        git("config", "user.email", "self-test@example.invalid")
        git("config", "user.name", "self-test")
        (history / "scripts" / "landed.py").write_text("#\n", encoding="utf-8")
        (history / "scripts" / "staged.py").write_text("#\n", encoding="utf-8")
        git("add", "-A")
        git("commit", "-q", "-m", "add")
        (history / "scripts" / "landed.py").unlink()
        git("add", "-A")
        git("commit", "-q", "-m", "delete")

        if target.is_shallow(history):
            print("self-test FAILED: a freshly built repository reads as shallow")
            return 1

        found = target.deleted_paths(history)
        if found is None or "scripts/landed.py" not in found:
            print("self-test FAILED: a deletion in git history was not found -- "
                  "the exemption that lets a doc SAY a file was deleted is a hole")
            return 1

        (history / "scripts" / "staged.py").unlink()
        git("add", "-A")
        found = target.deleted_paths(history)
        if found is None or "scripts/staged.py" not in found:
            print("self-test FAILED: a STAGED deletion was not found -- the "
                  "commit that removes a file is gated by the checker that only "
                  "believes it is gone once that commit lands, so it can never "
                  "be made")
            return 1

        # And the case the shallow clone exposed, asserted where it can be
        # produced: a SHALLOW repository must be recognised as one, because the
        # alternative is six phantom findings that look like real dead paths.
        shallow = Path(tmp) / "shallow"
        subprocess.run(
            ["git", "clone", "-q", "--depth", "1", "--no-local", str(history), str(shallow)],
            check=True, capture_output=True, text=True,
        )
        if target.is_shallow(shallow) is not True:
            print("self-test FAILED: a --depth 1 clone did not read as shallow, "
                  "so main() would report phantoms on CI and nobody would be told")
            return 1
        if target.deleted_paths(shallow) is None:
            print("self-test FAILED: a shallow clone reported unreadable history "
                  "rather than empty history -- the two must be told apart")
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
    if target.offenders(root, never):
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
    if not target.offenders(root, never):
        print("self-test FAILED: an ambiguous tail was accepted -- two files "
              "end the same way, so the reference identifies neither")
        return 1
    (root / "crates" / "app" / "views.rs").unlink()
    (root / "crates" / "app").rmdir()
    (root / "crates").rmdir()

    print("check_doc_paths self-test: ok")
    return 0
