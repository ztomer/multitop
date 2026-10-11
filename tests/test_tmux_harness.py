"""Unit tests for the e2e harness itself: the staleness refusal.

`stale_reason` blocks a suite run against a binary older than the source,
because that tests code nobody wrote. But integration tests and benches are
never linked into the binary, so a commit touching only them tripped the
refusal after a build that (correctly) changed nothing -- the gate demanded
the impossible. These pin the boundary: the files cargo's dep-info lists count, nothing
else does (a #[path] unit-test module in src/ is not read by the build);
without dep-info, tests/ and benches/ do not count and src/ does.
"""

import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from tmux_harness import built_from, newest_source_time, stale_reason  # noqa: E402


def make_tree(root, newest_src=1000000000):
    """A fake crates/ tree: src old, tests and benches newer."""
    src = os.path.join(root, "pkg", "src")
    tests = os.path.join(root, "pkg", "tests")
    benches = os.path.join(root, "pkg", "benches")
    for d in (src, tests, benches):
        os.makedirs(d, exist_ok=True)
    paths = {}
    for d, name, mtime in [
        (src, "lib.rs", newest_src),
        (tests, "something_test.rs", newest_src + 100),
        (benches, "something_bench.rs", newest_src + 200),
    ]:
        p = os.path.join(d, name)
        with open(p, "w", encoding="utf-8") as f:
            f.write("// fake\n")
        os.utime(p, (mtime, mtime))
        paths[name] = p
    return paths


def test_tests_and_benches_do_not_count_as_source(tmp_path):
    make_tree(str(tmp_path))
    # The newest .rs files are tests/ and benches/, but the answer must be
    # the src/ mtime: neither target reaches the binary under test.
    assert newest_source_time(str(tmp_path)) == 1000000000


def test_src_counts(tmp_path):
    paths = make_tree(str(tmp_path))
    now = time.time()
    os.utime(paths["lib.rs"], (now, now))
    assert newest_source_time(str(tmp_path)) == now


def test_stale_reason_ignores_test_only_changes(tmp_path):
    make_tree(str(tmp_path))
    binary = os.path.join(str(tmp_path), "multitop")
    with open(binary, "w", encoding="utf-8") as f:
        f.write("fake\n")
    # Binary newer than src/ but older than tests/: current, not stale.
    os.utime(binary, (1000000000 + 50, 1000000000 + 50))
    assert stale_reason(binary, str(tmp_path)) is None


def test_stale_reason_still_fires_on_stale_binary(tmp_path):
    make_tree(str(tmp_path))
    binary = os.path.join(str(tmp_path), "multitop")
    with open(binary, "w", encoding="utf-8") as f:
        f.write("fake\n")
    os.utime(binary, (999999999, 999999999))
    reason = stale_reason(binary, str(tmp_path))
    assert reason is not None and "stale" in reason


def dep_info_tree(root):
    """A binary with cargo's dep-info beside it, a source it lists, and a
    unit-test module in src/ that a non-test build never reads."""
    src = os.path.join(root, "src dir")
    os.makedirs(src)
    lib = os.path.join(src, "lib.rs")
    unit = os.path.join(src, "lib_tests.rs")
    binary = os.path.join(root, "multitop")
    for p, t in [(lib, 1000), (unit, 1000), (binary, 2000)]:
        with open(p, "w", encoding="utf-8") as f:
            f.write("x\n")
        os.utime(p, (t, t))
    with open(binary + ".d", "w", encoding="utf-8") as f:
        f.write(f"{binary}: {lib.replace(' ', chr(92) + ' ')}\n")
    return binary, lib, unit


def test_dep_info_is_read_with_escaped_spaces(tmp_path):
    binary, lib, _ = dep_info_tree(str(tmp_path))
    assert built_from(binary) == [lib]


def test_a_unit_test_module_the_build_never_reads_is_not_staleness(tmp_path):
    # 2026-10-10: editing ops_tests.rs (a #[path] cfg(test) module) refused
    # the e2e suite after a build that correctly relinked nothing.
    binary, _, unit = dep_info_tree(str(tmp_path))
    os.utime(unit, (3000, 3000))
    assert stale_reason(binary) is None


def test_a_listed_source_newer_than_the_binary_is_staleness(tmp_path):
    binary, lib, _ = dep_info_tree(str(tmp_path))
    os.utime(lib, (3000, 3000))
    assert "stale" in (stale_reason(binary) or "")


def test_a_listed_source_that_is_gone_is_staleness(tmp_path):
    binary, lib, _ = dep_info_tree(str(tmp_path))
    os.remove(lib)
    assert "stale" in (stale_reason(binary) or "")
