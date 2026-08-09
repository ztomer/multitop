#!/usr/bin/env python3
"""Flag public functions whose only callers are tests.

This review found the same defect five times: a function that is written,
tested, and reachable from nothing. `Vault::initialize`, `migrate_if_needed`,
`PasswordAction::Delete`, `rollback::parse_stored_counter`, and
`LockoutState::on_failure` all had tests calling them directly while no
production path did.

The last two were the reason this exists. In both, a *duplicate* of the dead
function's logic was doing the real work inline, and in both the surviving
duplicate was subtly weaker -- `unwrap_or(0)` where the dead one validated, a
bare assignment where the live path took `.max()`. So the tests were guarding
the wrong copy: the code under test could not drift, and the code in production
could drift freely without a single test noticing. A passing suite meant nothing
for the lines that actually ran.

Grepping for that by hand does not scale, so this makes it structural.

Usage:
    python3 tools/check_test_only_code.py [--self-test]

Exit status is 1 when something is flagged, so it can gate a commit.

To silence a legitimate case, put a `reachability:` note on the line above the
function saying who reaches it and why the tool cannot see it -- a trait method
called through dynamic dispatch, an API consumed by another binary, and so on.
The note is required to be non-empty: the point is to record the reason, not to
have a magic word that switches the check off.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BASELINE = REPO / "tools" / "test_only_baseline.txt"
ALLOW_RE = re.compile(r"//\s*reachability:\s*(\S.*)$")
PUB_FN_RE = re.compile(r"^\s*pub(?:\s*\([^)]*\))?\s+(?:const\s+|async\s+|unsafe\s+)*fn\s+(\w+)")

# Names that are called by the language or a derive rather than by our code, so
# "no call site" says nothing about whether they are reachable.
IMPLICIT = {
    "new", "default", "drop", "fmt", "clone", "eq", "ne", "hash", "cmp",
    "partial_cmp", "from", "into", "try_from", "try_into", "as_ref", "as_mut",
    "deref", "deref_mut", "next", "poll", "serialize", "deserialize", "main",
}


def strip_test_regions(text: str) -> str:
    """Blank out `#[cfg(test)]` items, preserving line numbering.

    Brace matching rather than a regex, because a test module contains nested
    braces and string literals with braces in them.
    """
    out = list(text)
    for m in re.finditer(r"#\[cfg\(test\)\]", text):
        i = text.find("{", m.end())
        if i == -1:
            continue
        # A `#[cfg(test)]` on a `use` or a single fn may have no block before
        # the next item; bail out if the next brace is implausibly far away.
        if text.count("\n", m.end(), i) > 3:
            continue
        depth, j = 0, i
        while j < len(text):
            if text[j] == "{":
                depth += 1
            elif text[j] == "}":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        for k in range(m.start(), min(j + 1, len(text))):
            if out[k] != "\n":
                out[k] = " "
    return "".join(out)


def rust_files(*globs: str) -> list[Path]:
    seen: set[Path] = set()
    for g in globs:
        seen.update(p for p in REPO.glob(g) if p.is_file())
    return sorted(seen)


def find_note(lines: list[str], idx: int) -> str:
    """Look for a `reachability:` marker in the comment block above `idx`.

    Walks up over the contiguous run of comments and attributes immediately
    preceding the declaration, rather than a fixed one or two lines: a real
    explanation runs to several lines, and the marker naturally opens it. The
    first version checked only two lines back, so a four-line note silently
    failed to register and the function stayed flagged.
    """
    back = idx - 1
    while back >= 0:
        stripped = lines[back].strip()
        if not (stripped.startswith("//") or stripped.startswith("#[") or stripped.startswith("#!")):
            break
        found = ALLOW_RE.search(lines[back])
        if found:
            return found.group(1).strip()
        back -= 1
    return ""


def collect_declarations(files: list[Path]) -> dict[str, list[tuple[Path, int, str]]]:
    """Public fns declared outside test regions, with any reachability note."""
    decls: dict[str, list[tuple[Path, int, str]]] = {}
    for path in files:
        raw = path.read_text(encoding="utf-8", errors="replace")
        lines = raw.splitlines()
        production = strip_test_regions(raw).splitlines()
        for idx, line in enumerate(production):
            m = PUB_FN_RE.match(line)
            if not m:
                continue
            name = m.group(1)
            if name in IMPLICIT:
                continue
            note = find_note(lines, idx)
            decls.setdefault(name, []).append((path, idx + 1, note))
    return decls


def count_production_uses(name: str, sources: list[tuple[Path, str]]) -> int:
    """Uses of `name` in production code, excluding its own declaration."""
    call = re.compile(rf"(?<!\w){re.escape(name)}\s*(?:\(|::|,|\)|\]|\}}|;|$)")
    total = 0
    for path, text in sources:
        for line in text.splitlines():
            if PUB_FN_RE.match(line) and PUB_FN_RE.match(line).group(1) == name:
                continue
            total += len(call.findall(line))
    return total


def run_check() -> int:
    all_src = rust_files("crates/*/src/**/*.rs", "crates/*/src/*.rs")
    # A src file named `*_tests.rs` is a test module in its entirety: the repo
    # includes them with `#[cfg(test)] #[path = "x_tests.rs"] mod x_tests;`, so
    # the `#[cfg(test)]` lives in the parent and `strip_test_regions` finds
    # nothing to blank inside the file itself. Without this, every call from one
    # of them read as a production call and the check quietly stopped finding
    # anything -- which is how splitting the vault crate made three known
    # baseline entries look resolved in the same commit.
    src = [p for p in all_src if not p.name.endswith("_tests.rs")]
    in_src_tests = [p for p in all_src if p.name.endswith("_tests.rs")]
    tests = rust_files("crates/*/tests/**/*.rs", "crates/*/tests/*.rs")

    decls = collect_declarations(src)
    # Production = every src file with test regions blanked. Integration tests
    # under tests/ are deliberately excluded: being called only from there is
    # exactly the condition being hunted.
    production = [(p, strip_test_regions(p.read_text(encoding="utf-8", errors="replace"))) for p in src]
    test_sources = [
        (p, p.read_text(encoding="utf-8", errors="replace")) for p in tests + in_src_tests
    ]
    test_sources += [
        (p, "\n".join(
            orig_line if orig_line != prod_line else ""
            for orig_line, prod_line in zip(
                p.read_text(encoding="utf-8", errors="replace").splitlines(),
                strip_test_regions(p.read_text(encoding="utf-8", errors="replace")).splitlines(),
            )
        ))
        for p in src
    ]

    flagged = []
    for name, sites in sorted(decls.items()):
        if count_production_uses(name, production) > 0:
            continue
        if count_production_uses(name, test_sources) == 0:
            continue  # Called by nothing at all: dead, but not *falsely assured*.
        for path, line, note in sites:
            if note:
                continue
            flagged.append((path.relative_to(REPO), line, name))

    # A ratchet, not a wall. This check arrived with 19 pre-existing cases, and a
    # gate that fails on day one is a gate somebody disables on day two. The
    # baseline records what was already there: anything new fails immediately,
    # and an entry that gets fixed must be removed from the baseline, so the list
    # can only shrink.
    baseline = set()
    if BASELINE.exists():
        for raw in BASELINE.read_text(encoding="utf-8").splitlines():
            entry = raw.split("#", 1)[0].strip()
            if entry:
                baseline.add(entry)

    current = {f"{path}:{name}" for path, _, name in flagged}
    new = sorted(current - baseline)
    fixed = sorted(baseline - current)

    if not new and not fixed:
        if baseline:
            print(
                f"test-only-code: clean ({len(decls)} public fns checked, "
                f"{len(baseline)} known cases outstanding)"
            )
        else:
            print(f"test-only-code: clean ({len(decls)} public fns checked)")
        return 0

    if new:
        print("test-only-code: NEW functions reachable only from tests\n")
        for path, line, name in flagged:
            if f"{path}:{name}" in set(new):
                print(f"  {path}:{line}: {name}")
        print(
            "\nEach is exercised by tests and by no production path, so its tests"
            "\nattest to code that never runs -- and if a duplicate of its logic is"
            "\nwhat production actually calls, that duplicate is untested and free to"
            "\ndrift. Either wire it up, delete it and point its tests at whatever"
            "\nreplaced it, or record why the tool cannot see the caller:"
            "\n    // reachability: <who calls this, and why this tool cannot see it>"
        )

    if fixed:
        print("\ntest-only-code: baseline entries no longer flagged -- remove them")
        print("from tools/test_only_baseline.txt so the list keeps shrinking:\n")
        for entry in fixed:
            print(f"  {entry}")

    return 1


def self_test() -> int:
    """Prove the checker detects what it claims to, and stays quiet otherwise."""
    cases = [
        ("pub fn live(x: u8) {}\nfn caller() { live(1); }\n", 0, "called in production"),
        ("pub fn ghost(x: u8) {}\n#[cfg(test)]\nmod t { fn a() { ghost(1); } }\n", 1, "test-only"),
        ("pub fn noted(x: u8) {}\n#[cfg(test)]\nmod t { fn a() { noted(1); } }\n", 0, "noted"),
        ("pub fn orphan(x: u8) {}\n", 0, "called by nothing at all"),
    ]
    failures = 0
    for source, want, label in cases:
        if label == "noted":
            source = (
                "// reachability: called by the C FFI shim\n"
                "// through a symbol the tool cannot resolve,\n"
                "// so it looks unreferenced from Rust.\n"
                "#[must_use]\n"
            ) + source
        text = strip_test_regions(source)
        decls = {}
        lines = source.splitlines()
        for idx, line in enumerate(text.splitlines()):
            m = PUB_FN_RE.match(line)
            if m and m.group(1) not in IMPLICIT:
                decls[m.group(1)] = find_note(lines, idx)
        got = 0
        prod = [(Path("x.rs"), text)]
        only_tests = "\n".join(
            o if o != p else "" for o, p in zip(source.splitlines(), text.splitlines())
        )
        for name, note in decls.items():
            if count_production_uses(name, prod) > 0 or note:
                continue
            if count_production_uses(name, [(Path("x.rs"), only_tests)]) == 0:
                continue
            got += 1
        if got != want:
            print(f"  self-test FAILED [{label}]: expected {want} flagged, got {got}")
            failures += 1
    if failures:
        return 1
    print("test-only-code self-test: clean")
    return 0


if __name__ == "__main__":
    sys.exit(self_test() if "--self-test" in sys.argv else run_check())
