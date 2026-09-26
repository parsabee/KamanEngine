#!/usr/bin/env python3
"""Validate the code links in KamanEngine's Markdown docs.

The docs link into source with relative paths and `#L<n>` line anchors. Those
anchors are precise and therefore fragile: any edit above a cited line silently
slides the anchor onto unrelated code, and nothing complains. A doc that points
confidently at the wrong line is worse than one that points at the file, because
a reader trusts it.

This linter catches four failure modes:

1. **Broken path** — the linked file does not exist.
2. **Out-of-range anchor** — `#L<n>` is past the end of the file.
3. **Blank target** — the anchored line is empty, which always means drift; a doc
   never deliberately cites a blank line.
4. **Drifted anchor** — checked two ways:
   - a label like `game.rs:517` whose `#L` anchor says something else;
   - a label naming a Rust item, like ``player_position`` or ``FontAtlas``, where
     the anchored line does not mention that name. This is the check that finds
     real rot: the anchor is in range and non-blank, so nothing else notices.

Run from the repo root:

    python3 scripts/check-doc-anchors.py

Exits non-zero if anything fails, so it can gate a commit.
"""

from __future__ import annotations

import os
import re
import sys

# Docs that link into source. Extend as new ones are added.
DOCS = [
    "README.md",
    "docs/ARCHITECTURE.md",
    "docs/DESIGN.md",
    "docs/GETTING_STARTED.md",
    "docs/PLAYABLE_DEMO.md",
    "games/playable-demo/README.md",
    "crates/kaman-render/README.md",
]

LINK = re.compile(r"\[([^\]]*?)\]\(([^)\s]+)\)")
# A label that names a Rust item: `foo`, `Foo`, `Foo::bar`, `config::SEED`.
SYMBOL_LABEL = re.compile(r"^`([A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*)`")
# A label that names a file and line: `game.rs:517`, `kaman-scene:107`.
FILE_LINE_LABEL = re.compile(r":(\d+)(?:[–-]\d+)?$")


def check_file(doc: str) -> list[str]:
    """Return a list of human-readable problems found in `doc`."""
    problems: list[str] = []
    base = os.path.dirname(doc)
    with open(doc, encoding="utf-8") as fh:
        text = fh.read()

    for match in LINK.finditer(text):
        label, target = match.group(1), match.group(2)
        if target.startswith(("http://", "https://", "#", "mailto:")):
            continue

        path = target.split("#")[0]
        if not path:
            continue

        full = os.path.normpath(os.path.join(base, path))
        if not os.path.exists(full):
            problems.append(f"{doc}: broken path -> {target}")
            continue

        anchor = re.search(r"#L(\d+)$", target)
        if not anchor:
            continue
        line_no = int(anchor.group(1))

        with open(full, encoding="utf-8") as fh:
            lines = fh.read().split("\n")
        if line_no > len(lines):
            problems.append(
                f"{doc}: anchor past end of file -> {path}#L{line_no} "
                f"({len(lines)} lines)"
            )
            continue

        line = lines[line_no - 1].strip()
        if not line:
            problems.append(
                f"{doc}: anchor on a blank line -> {path}#L{line_no} (label {label!r})"
            )
            continue

        # A `file.rs:NN` label must agree with its own anchor.
        file_line = FILE_LINE_LABEL.search(label)
        if file_line and int(file_line.group(1)) != line_no:
            problems.append(
                f"{doc}: label says line {file_line.group(1)} but anchor is "
                f"#L{line_no} -> {path}"
            )
            continue

        # A label naming a Rust item should land on a line mentioning it.
        symbol = SYMBOL_LABEL.match(label)
        if symbol:
            name = symbol.group(1).split("::")[-1]
            # Look at the anchored line and the two after it: a doc often cites
            # the start of a doc comment or an attribute above the item.
            window = "\n".join(lines[line_no - 1 : line_no + 2])
            if name not in window:
                problems.append(
                    f"{doc}: anchor does not mention {name!r} -> "
                    f"{path}#L{line_no} (found {line[:60]!r})"
                )

    return problems


def main() -> int:
    if not os.path.exists("scripts/check-doc-anchors.py"):
        print("run this from the repository root", file=sys.stderr)
        return 2

    all_problems: list[str] = []
    for doc in DOCS:
        if not os.path.exists(doc):
            all_problems.append(f"{doc}: listed in DOCS but missing")
            continue
        all_problems.extend(check_file(doc))

    if all_problems:
        for problem in all_problems:
            print(f"  ✗ {problem}")
        print(f"\n{len(all_problems)} doc anchor problem(s).")
        return 1

    print(f"✓ all code links in {len(DOCS)} docs resolve, and every anchor is live.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
