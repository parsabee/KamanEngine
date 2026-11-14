#!/usr/bin/env bash
# Fail if the project's pre-rename name comes back anywhere in the repository.
#
# The project was renamed to Stick Shift Engine in v0.1.0-beta.2. The old name may stay only in:
#   - CHANGELOG.md, whose past entries keep the names they shipped with;
#   - this script, which has to name what it looks for;
#   - the README's one "formerly ..." note;
#   - the line in .github/workflows/release.yml that maps the old Homebrew formula to
#     stickshift-engine in the tap's formula_renames.json.
#
# Run from anywhere: ./scripts/check-renamed.sh (CI runs it on every push and PR).
set -euo pipefail

cd "$(dirname "$0")/.."
old="kaman"

hits="$(git grep -n -i -I "$old" -- . ':!CHANGELOG.md' ':!scripts/check-renamed.sh' \
    | grep -v -E '^README\.md:[0-9]+:.*formerly KamanEngine' \
    | grep -v -E '^\.github/workflows/release\.yml:[0-9]+:.*formula_renames' \
    || true)"

if [[ -n "$hits" ]]; then
    echo "The pre-rename name is still used here; rename it to Stick Shift Engine / stickshift:" >&2
    echo "$hits" >&2
    exit 1
fi
echo "no pre-rename names left"
