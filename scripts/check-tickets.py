#!/usr/bin/env python3
"""KamanEngine ticket linter + progress reporter.

Validates that every `tickets/KE-*.md` follows the header schema in
`tickets/README.md`, then reports per-phase completion (% done + remaining work).

Usage:
    scripts/check-tickets.py            # lint + report
    scripts/check-tickets.py --quiet    # only print errors + the summary table

Exit code is non-zero if any ticket has a format ERROR, so it can gate CI and be
run after creating/editing tickets. Run this every time you add or change tickets.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

TICKETS_DIR = Path(__file__).resolve().parent.parent / "tickets"

# Required header fields, each on its own line (a `Blocks:` may share the
# `Depends on:` line, matching the established format).
REQUIRED_FIELDS = ["Phase", "Priority", "Status", "Integration", "Size", "Depends on", "Serves"]
OPTIONAL_FIELDS = ["Time", "Risk", "Blocks"]

VALID_PRIORITY = {"P0", "P1", "P2", "P3"}
VALID_STATUS = {"Todo", "In-Progress", "In-Review", "Blocked", "Done"}
VALID_INTEGRATION = {"Reuse-as-is", "Refactor", "New"}
SIZE_RE = re.compile(r"^(XS|S|M|L|XL)\s*·\s*A[0-3]$")
ID_RE = re.compile(r"KE-(\d{4})")

PHASE_NAMES = {
    0: "Foundation & Migration Harness",
    1: "Renderer Foundation",
    2: "Gameplay Core",
    3: "iOS Bring-up",
    4: "Look & Feel + Content",
    5: "KamanScript",
    6: "Release",
}


def field(text: str, name: str) -> str | None:
    """Return the value of a `Name:` field that starts its own line, else None."""
    m = re.search(rf"^{re.escape(name)}:\s*(\S.*?)\s*$", text, re.M)
    return m.group(1) if m else None


def lint(path: Path) -> tuple[dict, list[str]]:
    """Return (parsed-fields, errors) for one ticket file."""
    text = path.read_text(encoding="utf-8")
    errors: list[str] = []
    data: dict = {"file": path.name}

    # H1: "# KE-0PNN — Title"
    h1 = re.search(r"^#\s+(KE-\d{4})\s+—\s+(.+?)\s*$", text, re.M)
    if not h1:
        errors.append("missing or malformed H1 (`# KE-0PNN — Title`)")
        tid = ID_RE.search(path.name)
        data["id"] = tid.group(0) if tid else path.stem
    else:
        data["id"] = h1.group(1)
        data["title"] = h1.group(2)
        if not path.name.startswith(h1.group(1)):
            errors.append(f"filename does not start with its id {h1.group(1)}")

    # Required fields present.
    for name in REQUIRED_FIELDS:
        val = field(text, name)
        if val is None:
            errors.append(f"missing required field `{name}:` (each on its own line)")
        else:
            data[name] = val

    # Value validation.
    num = ID_RE.search(data.get("id", ""))
    id_phase = int(num.group(1)[1]) if num else None
    if "Phase" in data:
        if not re.fullmatch(r"[0-6]", data["Phase"]):
            errors.append(f"Phase must be 0..6, got {data['Phase']!r}")
        elif id_phase is not None and int(data["Phase"]) != id_phase:
            errors.append(f"Phase {data['Phase']} != id phase digit {id_phase} (from {data['id']})")
    if "Priority" in data and data["Priority"] not in VALID_PRIORITY:
        errors.append(f"Priority must be one of {sorted(VALID_PRIORITY)}, got {data['Priority']!r}")
    if "Status" in data and data["Status"] not in VALID_STATUS:
        errors.append(f"Status must be one of {sorted(VALID_STATUS)}, got {data['Status']!r}")
    if "Integration" in data and data["Integration"] not in VALID_INTEGRATION:
        errors.append(f"Integration must be one of {sorted(VALID_INTEGRATION)}, got {data['Integration']!r}")
    if "Size" in data and not SIZE_RE.match(data["Size"]):
        errors.append(f"Size must match `<XS|S|M|L|XL> · A<0-3>`, got {data['Size']!r}")

    # Required sections / gates.
    if not re.search(r"^##\s+Scope", text, re.M):
        errors.append("missing a `## Scope` section (Scope & Acceptance)")
    if "Test gate" not in text:
        errors.append("missing a `Test gate`")
    if "Doc gate" not in text:
        errors.append("missing a `Doc gate`")

    # Acceptance checkbox accounting.
    data["checked"] = len(re.findall(r"^- \[x\]", text, re.M))
    data["unchecked"] = len(re.findall(r"^- \[ \]", text, re.M))
    return data, errors


def main() -> int:
    quiet = "--quiet" in sys.argv
    files = sorted(TICKETS_DIR.glob("KE-*.md"))
    if not files:
        print("no ticket files found", file=sys.stderr)
        return 1

    tickets: list[dict] = []
    total_errors = 0
    for path in files:
        data, errors = lint(path)
        tickets.append(data)
        if errors:
            total_errors += len(errors)
            print(f"✗ {path.name}")
            for e in errors:
                print(f"    - {e}")
        elif not quiet:
            print(f"✓ {path.name}")

    # Per-phase progress report.
    print("\n" + "=" * 68)
    print(f"{'Phase':<32}{'Done':>6}{'Total':>7}{'%':>6}  Remaining")
    print("-" * 68)
    for phase in range(7):
        group = [t for t in tickets if t.get("Phase") == str(phase)]
        if not group:
            continue
        done = [t for t in group if t.get("Status") == "Done"]
        remaining = [t for t in group if t.get("Status") != "Done"]
        pct = round(100 * len(done) / len(group))
        label = f"{phase} {PHASE_NAMES.get(phase, '')}"[:31]
        rem = ", ".join(f"{t['id']}({t.get('Status','?')})" for t in remaining) or "—"
        print(f"{label:<32}{len(done):>6}{len(group):>7}{pct:>5}%  {rem}")

    dt = sum(t["checked"] + t["unchecked"] for t in tickets)
    dc = sum(t["checked"] for t in tickets)
    apct = round(100 * dc / dt) if dt else 0
    print("-" * 68)
    done_all = sum(1 for t in tickets if t.get("Status") == "Done")
    print(f"Overall: {done_all}/{len(tickets)} tickets Done; "
          f"acceptance items {dc}/{dt} checked ({apct}%).")
    print("=" * 68)

    if total_errors:
        print(f"\nFORMAT ERRORS: {total_errors} across the tickets above. Fix them.", file=sys.stderr)
        return 1
    print("\nAll tickets conform to the format.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
