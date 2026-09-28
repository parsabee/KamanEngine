#!/usr/bin/env bash
# Point the Homebrew formula at a release: rewrite its `url` and `sha256` lines (and `version`,
# if the formula has one; normally it doesn't, since brew scans the version from the url).
#
#   scripts/update-homebrew-formula.sh <formula.rb> <version> <sha256>
#   scripts/update-homebrew-formula.sh packaging/homebrew/kaman-engine.rb 0.1.0-alpha.3 <64 hex>
#
# Used by .github/workflows/release.yml after the release tarball is uploaded, for both the tap's
# Formula/kaman-engine.rb and the canonical copy in packaging/homebrew/. Fails if `url` or `sha256`
# is missing, so a reshaped formula is noticed instead of silently left stale.
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: $0 <formula.rb> <version> <sha256>" >&2
  exit 2
fi
formula=$1 version=$2 sha=$3

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || { echo "bad version: $version" >&2; exit 1; }
[[ "$sha" =~ ^[0-9a-f]{64}$ ]] || { echo "bad sha256: $sha" >&2; exit 1; }
for key in url sha256; do
  grep -Eq "^  $key \"" "$formula" || { echo "$formula has no top-level '$key' line" >&2; exit 1; }
done

url="https://github.com/parsabee/KamanEngine/releases/download/v${version}/kaman-engine-${version}-aarch64-apple-darwin.tar.gz"
tmp=$(mktemp)
sed -E \
  -e "s|^  url \".*\"$|  url \"${url}\"|" \
  -e "s|^  version \".*\"$|  version \"${version}\"|" \
  -e "s|^  sha256 \".*\"$|  sha256 \"${sha}\"|" \
  "$formula" > "$tmp"
mv "$tmp" "$formula"
echo "updated $formula -> $version ($sha)"
