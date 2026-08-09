#!/usr/bin/env python3
"""Fail if an integration test can reach the real OS credential store.

An integration test binary is compiled WITHOUT `cfg(test)`, so
`password_store::is_mock_enabled()` is false unless the test says otherwise.
Anything that then calls into `password_store` -- directly, or indirectly via
`passwords::open`, which calls `Panel::ensure_sudo_password` -- queries the
developer's real keychain.

That is not a theoretical leak. Every rebuild changes the test binary's code
signature, so macOS raises a keychain-access dialog and the whole suite stops
dead waiting for a human to type their login password. It also means a test can
read, overwrite, or delete the credentials the user actually depends on.

The rule: a test file that can reach the credential store must divert it, by
calling `enable_mock_store` or by setting `MULTITOP_MOCK_KEYCHAIN`.

Run with --self-test to check the checker.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Anything that can reach the credential store, directly or through the app.
#
# The indirect entries are the important ones and the reason this list is broad.
# `app_test.rs` reached the real keychain with no mention of `password_store` in
# it at all: it drove `App::apply`, which loads credentials several calls down.
# A gate that only matched direct calls reported that file clean, and the suite
# went on raising keychain dialogs. Text matching cannot follow a call graph, so
# the rule is deliberately coarse -- anything holding an `App` or pressing a key
# must divert the store, whether or not it turns out to need it.
REACHES_STORE = re.compile(
    r"password_store::(load|save|delete|account|load_sso|save_sso|delete_sso)"
    r"|passwords::open"
    r"|ensure_sudo_password"
    r"|App::new\("
    r"|handle_key\("
    r"|confirm_upgrade"
)

# Anything that proves the store was diverted first.
#
# Checked per test body, not per file. A file-level check was fooled by
# `server_settings_test.rs`: it defines a `setup_mock_store()` helper, which was
# enough to look clean, while three of its tests never called it and went
# straight to the real keychain.
DIVERTS = re.compile(
    r"enable_mock_store"
    r"|MULTITOP_MOCK_KEYCHAIN"
    r"|lock_for_test"
    r"|isolate_keychain"
    r"|setup_mock_store"
    r"|isolate\(\)"
)

# Any fn, so a test that calls a local `setup()` helper which diverts counts as
# diverting. Without this the check reports every test whose diversion is one
# call away, which is most of them, and a gate that cries wolf gets switched off.
ANY_FN = re.compile(
    r"(?m)^(?:pub )?(?:async )?fn (?P<name>[A-Za-z_0-9]+)\s*\("
)

TEST_FN = re.compile(
    r"(?m)^#\[(?:tokio::)?test\][^\n]*\n(?:#\[[^\n]*\]\n)*"
    r"(?:async )?fn (?P<name>[A-Za-z_0-9]+)\s*\("
)

TEST_DIRS = ["crates/multitop/tests", "crates/vault/tests", "crates/agent/tests"]

# Unit tests live in `src/` too, and they were not covered.
#
# multitop's own unit tests are safe by construction -- `mock_enabled_from`
# takes `cfg!(test)`, which is true for anything compiled into the lib's test
# binary. The vault's keychain use is NOT: `lockout.rs` and `rollback.rs` gate
# on a `use_keychain`/`use_os_keychain` flag carried in `VaultConfig`, so a
# vault unit test that builds a config with the flag on reaches the real OS
# keychain and the suite stops on a dialog nobody is there to dismiss.
#
# So the src sweep is scoped to the crate whose gate is a runtime flag rather
# than a compile-time one. Scoping it to where the hazard actually is keeps the
# check from crying wolf, which is how gates get switched off.
SRC_DIRS = ["crates/vault/src"]

# What reaches the vault's keychain, directly or by building a config that
# permits it. `Vault::new` is included for the same reason `App::new` is above:
# the keychain use is several calls down, and text matching cannot follow a
# call graph.
REACHES_VAULT_KEYCHAIN = re.compile(
    r"Vault::new\("
    r"|LockoutState::"
    r"|RollbackAnchor"
    r"|load_or_default\("
)

# What proves a vault test cannot reach it: the flag is off.
#
# Both spellings of "off" count. `LockoutState` grew a `new(use_keychain)` and a
# `load(path, use_keychain)` after this list was written, and every test that
# used them was passing only because the body parser below used to run past the
# end of a test and pick up a struct literal from a later one. Splitting the
# file made the bodies exact and the whole set lit up at once -- the tests were
# safe, the list was incomplete.
VAULT_DIVERTS = re.compile(
    r"use_os_keychain:\s*false"
    r"|use_keychain:\s*false"
    r"|LockoutState::new\(\s*false\s*\)"
    r"|LockoutState::load\([^)]*,\s*false\s*\)"
    r"|fast_vault_config"
    r"|isolated"
    r"|without_keychain"
    # An explicit, explained exception. Same shape as the `// reachability:`
    # note in check_test_only_code.py: for a test that must construct with the
    # policy ON and cannot reach the keychain anyway, put the note *inside* the
    # test body -- that is the text this check reads.
    r"|//\s*keychain-safe:"
)


def test_bodies(text: str):
    """Yield (name, body) for every test fn, matching braces to find its end.

    Brace matching rather than "the next line-start `}`". That shortcut was
    right only for a test at file scope: a test nested inside a `mod` closes at
    an *indented* brace, so the scan ran past it to the end of the module and
    the body picked up every later test's text -- including their diversion
    markers. Whole modules of unguarded tests read as guarded, which is how a
    process-global mock store ended up being stomped by a test the check had
    already cleared.
    """
    for m in TEST_FN.finditer(text):
        open_brace = text.find("{", m.end())
        if open_brace == -1:
            continue
        depth = 0
        i = open_brace
        while i < len(text):
            c = text[i]
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    break
            elif c == '"':
                # Skip a string literal whole; braces inside it are not code.
                i += 1
                while i < len(text) and text[i] != '"':
                    i += 2 if text[i] == "\\" else 1
            i += 1
        yield m.group("name"), text[open_brace : min(i + 1, len(text))]


def helpers_matching(text: str, pattern) -> set[str]:
    """Names of functions in this file whose own body matches `pattern`.

    Used for both halves of the question: which helpers divert the store, and
    which helpers reach it. Text matching cannot follow a call graph, so one
    hop through a locally-defined helper is as far as this goes.
    """
    names = set()
    for m in ANY_FN.finditer(text):
        start = m.end()
        end = text.find("\n}\n", start)
        body = text[start: end if end != -1 else len(text)]
        if pattern.search(body):
            names.add(m.group("name"))
    return names


def calls_any(names: set[str]):
    """A pattern matching a call to any of `names`, or None if there are none."""
    if not names:
        return None
    return re.compile(r"\b(?:" + "|".join(re.escape(n) for n in names) + r")\s*\(")


def sweep(root: Path, directory: str, reaches, diverts) -> list[tuple[Path, str]]:
    """Every test in `directory` that reaches the store without diverting it."""
    found = []
    base = root / directory
    if not base.is_dir():
        return found
    for path in sorted(base.rglob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        if not reaches.search(text):
            continue
        calls_diverter = calls_any(helpers_matching(text, diverts))
        calls_reacher = calls_any(helpers_matching(text, reaches))
        for name, body in test_bodies(text):
            # Only a test that can actually reach the store owes a diversion.
            # This used to be decided per FILE -- one integration test made
            # every pure unit test beside it an offender, which is exactly the
            # crying-wolf a gate does not survive. `repaint_test.rs` lit up
            # twelve times over `Painter` tests that touch nothing at all.
            reaches_here = reaches.search(body) or (
                calls_reacher and calls_reacher.search(body)
            )
            if not reaches_here:
                continue
            if diverts.search(body):
                continue
            if calls_diverter and calls_diverter.search(body):
                continue
            found.append((path.relative_to(root), name))
    return found


def offenders(root: Path) -> list[tuple[Path, str]]:
    found = []
    for directory in TEST_DIRS:
        found += sweep(root, directory, REACHES_STORE, DIVERTS)
    # The src sweep closes the hole this check shipped with: it scanned
    # `crates/*/tests` only, so every unit test inside `src/` was unchecked.
    for directory in SRC_DIRS:
        found += sweep(root, directory, REACHES_VAULT_KEYCHAIN, VAULT_DIVERTS)
    return found


def self_test() -> int:
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        d = root / "crates/multitop/tests"
        d.mkdir(parents=True)

        (d / "clean.rs").write_text(
            "#[test]\nfn a() {\n    password_store::enable_mock_store();\n"
            "    passwords::open(&mut x, 0, false);\n}\n"
        )
        (d / "unrelated.rs").write_text("#[test]\nfn b() {\n    assert_eq!(1, 1);\n}\n")
        (d / "leaky.rs").write_text(
            "#[test]\nfn c() {\n    passwords::open(&mut x, 0, false);\n}\n"
        )
        # Reaches the store only through App, naming nothing itself.
        (d / "indirect.rs").write_text(
            "#[test]\nfn dd() {\n    let a = App::new(vec![]);\n}\n"
        )
        # The regression this check exists for: a helper the file defines and
        # one of its tests forgets to call.
        (d / "partial.rs").write_text(
            "fn setup_mock_store() {\n    password_store::enable_mock_store();\n}\n"
            "#[test]\nfn guarded() {\n    let _g = setup_mock_store();\n"
            "    let a = App::new(vec![]);\n}\n"
            "#[test]\nfn forgot() {\n    let a = App::new(vec![]);\n}\n"
        )
        # A pure test sitting beside a reaching one is NOT an offender. Both
        # lack a diversion; only the one that can reach the store needs it.
        (d / "mixed.rs").write_text(
            "#[test]\nfn pure() {\n    assert_eq!(1, 1);\n}\n"
            "#[test]\nfn reaches() {\n    let a = App::new(vec![]);\n}\n"
        )
        # ...and reaching one hop through a local helper still counts.
        (d / "reaching_helper.rs").write_text(
            "fn build() -> App {\n    App::new(vec![])\n}\n"
            "#[test]\nfn pure_beside_it() {\n    assert_eq!(1, 1);\n}\n"
            "#[test]\nfn via_builder() {\n    let a = build();\n}\n"
        )
        # A locally-named helper must count, or the check reports tests that
        # do divert -- one call away.
        (d / "via_helper.rs").write_text(
            "fn setup() {\n    password_store::enable_mock_store();\n}\n"
            "#[test]\nfn ok_via_helper() {\n    let _g = setup();\n"
            "    let a = App::new(vec![]);\n}\n"
        )

        # The src sweep, which this check shipped without: a unit test inside
        # `crates/vault/src` that builds a vault with the keychain flag left on
        # reaches the real OS keychain, and nothing was looking there.
        v = root / "crates/vault/src"
        v.mkdir(parents=True)
        (v / "api.rs").write_text(
            "#[cfg(test)]\nmod tests {\n"
            "fn fast_vault_config(p: PathBuf) -> VaultConfig {\n"
            "    VaultConfig { use_os_keychain: false }\n}\n"
            "#[test]\nfn diverted() {\n"
            "    let v = Vault::new(fast_vault_config(p));\n}\n"
            "#[test]\nfn reaches_the_real_keychain() {\n"
            "    let v = Vault::new(VaultConfig { use_os_keychain: true });\n}\n"
            "}\n"
        )

        got = {(p.name, n) for p, n in offenders(root)}
        want = {
            ("leaky.rs", "c"),
            ("indirect.rs", "dd"),
            ("partial.rs", "forgot"),
            ("api.rs", "reaches_the_real_keychain"),
            ("mixed.rs", "reaches"),
            ("reaching_helper.rs", "via_builder"),
        }
        if got != want:
            print(f"self-test FAILED:\n  expected {sorted(want)}\n  got      {sorted(got)}")
            return 1
    print("keychain-isolation: self-test passed")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()

    found = offenders(ROOT)
    if not found:
        print("keychain-isolation: clean")
        return 0

    print("keychain-isolation: these tests can reach the real OS keychain\n")
    for path, name in found:
        print(f"  {path}::{name}")
    print(
        "\nDivert the store before touching it:\n"
        "    let _guard = password_store::lock_for_test();   // or lock_for_test_async().await\n"
        "    password_store::enable_mock_store();\n"
        "    password_store::clear_mock_store();\n"
        "\nHold the guard for the whole test body -- the mock store is process-global."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
