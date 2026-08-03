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

# Calls that reach the credential store, directly or one level down.
REACHES_STORE = re.compile(
    r"password_store::(load|save|delete|account|load_sso|save_sso|delete_sso)"
    r"|passwords::open"
    r"|ensure_sudo_password"
)

# Anything that proves the store was diverted first.
DIVERTS = re.compile(r"enable_mock_store|MULTITOP_MOCK_KEYCHAIN")

TEST_DIRS = ["crates/multitop/tests", "crates/vault/tests", "crates/agent/tests"]


def offenders(root: Path) -> list[tuple[Path, str]]:
    found = []
    for directory in TEST_DIRS:
        base = root / directory
        if not base.is_dir():
            continue
        for path in sorted(base.rglob("*.rs")):
            text = path.read_text(encoding="utf-8", errors="replace")
            hit = REACHES_STORE.search(text)
            if hit and not DIVERTS.search(text):
                found.append((path.relative_to(root), hit.group(0)))
    return found


def self_test() -> int:
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        d = root / "crates/multitop/tests"
        d.mkdir(parents=True)

        (d / "clean.rs").write_text(
            "fn s() { password_store::enable_mock_store(); passwords::open(&mut a, 0, false); }"
        )
        (d / "env_opt_in.rs").write_text(
            'fn s() { std::env::set_var("MULTITOP_MOCK_KEYCHAIN", "1"); '
            "password_store::load(&srv); }"
        )
        (d / "unrelated.rs").write_text("fn s() { assert_eq!(1, 1); }")
        (d / "leaky.rs").write_text("fn s() { passwords::open(&mut a, 0, false); }")

        got = {p.name for p, _ in offenders(root)}
        if got != {"leaky.rs"}:
            print(f"self-test FAILED: expected {{'leaky.rs'}}, got {got}")
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
    for path, call in found:
        print(f"  {path}  ->  {call}")
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
