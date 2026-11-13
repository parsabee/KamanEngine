#!/usr/bin/env bash
# Build libstickshift, then compile the C smoke test and the C sample against stickshift.h and
# run them. On macOS they render through Metal; elsewhere (or with --null) they use the
# null backend.
#
#   ./scripts/test-c-api.sh [--null]
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
null_flag=""
if [[ "${1:-}" == "--null" || "$(uname -s)" != "Darwin" ]]; then
    null_flag="--null"
fi

out="target/c-api"
mkdir -p "$out"

# Build the static library and ask rustc which native libraries it needs. Touch the
# crate root so rustc runs (and prints them) even when the library is up to date.
touch crates/stickshift-capi/src/lib.rs
native_libs="$(cargo rustc --quiet -p stickshift-capi --lib --crate-type staticlib -- \
    --print native-static-libs 2>&1 | sed -n 's/.*native-static-libs: //p' | tail -n 1)"
lib="target/debug/libstickshift.a"
[[ -f "$lib" ]] || { echo "$lib was not built" >&2; exit 1; }
echo "native libs: $native_libs"

cc="${CC:-cc}"
cflags=(-std=c11 -Wall -Wextra -pedantic -Werror -I crates/stickshift-capi/include)
# shellcheck disable=SC2086 # the native library list is meant to split into words
"$cc" "${cflags[@]}" crates/stickshift-capi/tests/c/smoke.c "$lib" $native_libs -o "$out/smoke"
# shellcheck disable=SC2086
"$cc" "${cflags[@]}" examples/c/offscreen.c "$lib" $native_libs -o "$out/offscreen"

# The dynamic library must carry an @rpath install name (crates/stickshift-capi/build.rs).
if [[ "$(uname -s)" == "Darwin" ]]; then
    cargo build --quiet -p stickshift-capi
    install_name="$(otool -D target/debug/libstickshift.dylib | tail -n 1)"
    if [[ "$install_name" != "@rpath/libstickshift.dylib" ]]; then
        echo "libstickshift.dylib's install name is '$install_name', expected @rpath/libstickshift.dylib" >&2
        exit 1
    fi
fi

fixture="crates/stickshift-assets/tests/fixtures/cube.gltf"
"$out/smoke" $null_flag "$fixture"
"$out/offscreen" "$fixture" "$out/offscreen.ppm" $null_flag
[[ -s "$out/offscreen.ppm" ]] || { echo "the sample wrote no image" >&2; exit 1; }
echo "C API: smoke test and sample passed"
