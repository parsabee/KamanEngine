#!/usr/bin/env bash
# Copyright (c) 2026 Parsa Bagheri
#
# This software is released under the Apache-2.0 License.
#
# Package a prebuilt Apple Silicon release of KamanEngine.
#
#   scripts/package-release.sh <version> <outdir>
#
# <version> is the workspace version without the leading `v` (e.g. 0.1.0-alpha.2).
# Expects an already-built release binary (`cargo build --release -p playable-demo
# --locked`) at $KAMAN_DEMO_BIN (default: target/release/playable-demo). Produces
#
#   <outdir>/kaman-engine-<version>-aarch64-apple-darwin.tar.gz
#   <outdir>/kaman-engine-<version>-aarch64-apple-darwin.tar.gz.sha256
#
# laid out for Homebrew (bin/ beside share/, which the demo's asset lookup expects):
#
#   kaman-engine-<version>-aarch64-apple-darwin/
#     bin/kaman-demo                     the playable demo
#     share/kaman-engine/assets/         the demo's runtime assets
#     share/kaman-engine/src/            the engine source (tracked files only)
#     LICENSE  NOTICE  README.md
#
# Before tarring it checks the binary is arm64, and runs `kaman-demo --smoke` from an
# unpacked copy in a temp dir, asserting the assets resolve to that copy's share/.
# Used by .github/workflows/release.yml; runnable locally.
set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "usage: $0 <version> <outdir>" >&2
    exit 2
fi
version=$1
outdir=$2
repo=$(cd "$(dirname "$0")/.." && pwd)
bin=${KAMAN_DEMO_BIN:-$repo/target/release/playable-demo}
name="kaman-engine-${version}-aarch64-apple-darwin"
tarball="${name}.tar.gz"

# The version must match the workspace's, or the tarball would lie about its contents.
ws_version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo/Cargo.toml" | head -n1)
if [[ "$version" != "$ws_version" ]]; then
    echo "error: version $version does not match the workspace version $ws_version" >&2
    exit 1
fi

[[ -x "$bin" ]] || { echo "error: no release binary at $bin (cargo build --release -p playable-demo --locked)" >&2; exit 1; }

archs=$(lipo -archs "$bin")
if [[ "$archs" != "arm64" ]]; then
    echo "error: $bin is '$archs', expected arm64 only" >&2
    exit 1
fi
file "$bin"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
stage="$work/$name"
mkdir -p "$stage/bin" "$stage/share/kaman-engine/assets" "$stage/share/kaman-engine/src"

# Binary, renamed to the installed command name.
cp "$bin" "$stage/bin/kaman-demo"
chmod 755 "$stage/bin/kaman-demo"

# Runtime assets: every file the demo loads, plus licence text. Generator-only inputs
# (the *_src.jpg photos, font.ttf) and the cube.gltf test fixture stay in the source
# copy below but are not needed at runtime.
assets_src="$repo/games/playable-demo/assets"
runtime_assets=(
    sports_car.glb car.glb car2.glb police_car.glb
    skyscraper_a.glb skyscraper_b.glb large_a.glb large_b.glb large_c.glb
    small_a.glb small_b.glb low_a.glb
    road.gltf skyline.gltf
    runner_loop.wav car_crash_impact_only.wav
    font.bin font-OFL.txt
)
for f in "${runtime_assets[@]}"; do
    cp "$assets_src/$f" "$stage/share/kaman-engine/assets/"
done

# Engine source for developers (Cargo path dependencies): tracked files only, so
# target/, .git and local clutter never ship.
src_paths=(Cargo.toml Cargo.lock rust-toolchain.toml crates games LICENSE NOTICE README.md CHANGELOG.md docs)
(cd "$repo" && git ls-files -z -- "${src_paths[@]}" | tar --null -T - -cf -) \
    | tar -xf - -C "$stage/share/kaman-engine/src"

cp "$repo/LICENSE" "$repo/NOTICE" "$repo/README.md" "$stage/"

mkdir -p "$outdir"
outdir=$(cd "$outdir" && pwd)
# Deterministic-ish archive: no macOS resource forks / xattrs.
COPYFILE_DISABLE=1 tar --no-xattrs -czf "$outdir/$tarball" -C "$work" "$name"
(cd "$outdir" && shasum -a 256 "$tarball" > "$tarball.sha256")

# Verify the packaged artefact, not the staging dir: unpack into a fresh temp dir and
# boot the smoke oracle from there. $KAMAN_DEMO_ASSETS is cleared so the lookup must
# find the tarball's own share/ (resolution rule b), which beats the build checkout.
check="$work/check"
mkdir -p "$check"
tar -xzf "$outdir/$tarball" -C "$check"
resolved=$(env -u KAMAN_DEMO_ASSETS "$check/$name/bin/kaman-demo" --print-assets-dir)
expected=$(cd "$check/$name/share/kaman-engine/assets" && pwd -P)
if [[ "$resolved" != "$expected" ]]; then
    echo "error: packaged kaman-demo resolved assets to $resolved, expected $expected" >&2
    exit 1
fi
env -u KAMAN_DEMO_ASSETS "$check/$name/bin/kaman-demo" --smoke | tee "$work/smoke.out"
grep -qx "smoke: 120 frames OK" "$work/smoke.out"

echo "packaged $outdir/$tarball ($(du -h "$outdir/$tarball" | cut -f1))"
cat "$outdir/$tarball.sha256"
