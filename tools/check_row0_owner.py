#!/usr/bin/env python3
"""Fail if a panel's `view` is assigned outside the two helpers that own row 0.

`ui::draw` replaces `lines[0]` with the host banner on every frame. A body
written into a pane starting at row 0 is therefore eaten, and a *one-line* body
is eaten whole -- built, stored, and never once drawn.

The project has found this defect four times, in four different places:

    Panel::new              "connecting..."             the box was blank
    Panel::show_last_frame  the last agent frame        the header vanished
    Msg::Status             "installing agent...",      an ssh error was
                            error_line(e)               destroyed on arrival
    App::toggle_fetch       "-> Fetching system info"   pressing `f` showed an
    App::toggle_docker      "-> Docker loading..."      empty pane

Each time it was fixed at the site. The fifth site was always going to be
written, because nothing about `p.view = vec![...]` says row 0 belongs to
somebody else -- so the rule is now structural rather than remembered.

The rule: inside `crates/multitop/src`, only `panel.rs` may assign `view`.
Everything else goes through one of two helpers, and the choice between them is
the whole point:

    Panel::show_body(lines)   app-authored text -- reserves row 0 for the banner
    Panel::show_frame(lines)  a rendered agent frame -- carries its own row 0

Run with --self-test to check the checker.
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

SRC = Path("crates/multitop/src")

# The file that owns the field may assign it; that is where the helpers live.
OWNER = "panel.rs"

# `foo.view = ...` or `foo.view: ...` inside a struct literal. `==` is excluded
# so a comparison is not mistaken for a write.
ASSIGN = re.compile(r"\.view\s*=(?!=)|(?<![.\w])view\s*:\s*vec!")


def offenders(root: Path) -> list[tuple[Path, int, str]]:
    hits: list[tuple[Path, int, str]] = []
    for path in sorted(root.rglob("*.rs")):
        if path.name == OWNER:
            continue
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            code = line.split("//", 1)[0]
            if ASSIGN.search(code):
                hits.append((path, lineno, line.strip()))
    return hits


def self_test() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / OWNER).write_text("self.view = lines;", encoding="utf-8")

        (root / "app.rs").write_text("p.view = vec![text];", encoding="utf-8")
        if not offenders(root):
            print("self-test FAILED: a raw view assignment was not detected")
            return 1

        (root / "app.rs").write_text("p.show_body(std::iter::once(text));", encoding="utf-8")
        if offenders(root):
            print("self-test FAILED: a helper call was reported as an assignment")
            return 1

        (root / "app.rs").write_text("if p.view == other { act() }", encoding="utf-8")
        if offenders(root):
            print("self-test FAILED: a comparison was reported as an assignment")
            return 1

        (root / "app.rs").write_text("// p.view = vec![text];", encoding="utf-8")
        if offenders(root):
            print("self-test FAILED: a commented-out assignment was reported")
            return 1
    print("check_row0_owner self-test: ok")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not SRC.is_dir():
        print(f"row0-owner: {SRC} not found -- run from the repository root")
        return 1
    hits = offenders(SRC)
    if not hits:
        print("row0-owner: clean")
        return 0
    print("row0-owner: a pane's `view` is assigned outside panel.rs\n")
    for path, lineno, line in hits:
        print(f"  {path}:{lineno}")
        print(f"    {line[:100]}")
    print(
        "\nRow 0 belongs to the host banner, which `ui::draw` composes over\n"
        "whatever is there. Use `Panel::show_body` for app-authored text (it\n"
        "reserves that row) or `Panel::show_frame` for a rendered agent frame\n"
        "(it carries its own row 0). A body assigned raw loses its first line,\n"
        "and a one-line body is lost whole."
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
