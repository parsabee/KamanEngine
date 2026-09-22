#!/usr/bin/env bash
# KamanEngine toolchain preflight — verifies the project's build/runtime
# dependencies are installed. Run this before your first build; CI runs it too.
# Exits non-zero if a REQUIRED tool is missing.
#
#   ./scripts/preflight.sh          check what's needed to build the engine today
#   ./scripts/preflight.sh --ios    ALSO require the Phase 3 (iOS) + .metallib toolchain
#
# Two tiers:
#   * Required now  — build + run the engine on macOS (KamanEngine is Apple-only).
#   * iOS / KE-0107 — full Xcode (metal/metallib shader compiler) + rustup iOS
#                     targets. Only a warning by default; a hard requirement with
#                     --ios (use it once you start Phase 3 or precompiled shaders).
set -uo pipefail

MIN_RUST="1.91.0"
ios=0
[ "${1:-}" = "--ios" ] && ios=1
fail=0

green="\033[32m"; red="\033[31m"; yellow="\033[33m"; reset="\033[0m"
ok()   { printf "  ${green}ok${reset}      %-16s %s\n" "$1" "$2"; }
miss() { printf "  ${red}MISSING${reset} %-16s %s\n" "$1" "$2"; fail=1; }
warn() { printf "  ${yellow}warn${reset}    %-16s %s\n" "$1" "$2"; }
have() { command -v "$1" >/dev/null 2>&1; }

# Emit MISSING when in the given tier is required, otherwise a warn.
need() { # tier_is_required  name  hint
  if [ "$1" -eq 1 ]; then miss "$2" "$3"; else warn "$2" "$3"; fi
}

echo "Platform:"
if [ "$(uname -s)" = "Darwin" ]; then
  ok "macOS" "$(sw_vers -productVersion 2>/dev/null)"
else
  miss "macOS" "KamanEngine targets Apple platforms only (raw Metal)"
fi

echo
echo "Required (build + run the engine on macOS):"
if have cargo && have rustc; then
  rv=$(rustc --version 2>/dev/null | awk '{print $2}')
  if [ "$(printf '%s\n%s\n' "$MIN_RUST" "$rv" | sort -V | head -1)" = "$MIN_RUST" ]; then
    ok "rust" "$rv (>= $MIN_RUST)"
  else
    miss "rust" "$rv found; need >= $MIN_RUST (see rust-toolchain.toml)"
  fi
else
  miss "rust" "install from https://rustup.rs (or: brew install rust)"
fi
if have clang; then
  ok "clang" "$(clang --version 2>/dev/null | head -1 | sed 's/ (.*//')"
else
  miss "clang" "install Command Line Tools: xcode-select --install"
fi
if xcrun --sdk macosx --show-sdk-path >/dev/null 2>&1; then
  ok "macOS SDK" "$(xcrun --sdk macosx --show-sdk-version 2>/dev/null)"
else
  miss "macOS SDK" "xcode-select --install"
fi

echo
echo "Required for iOS bring-up (Phase 3) + precompiled .metallib (KE-0107):"
if xcode-select -p 2>/dev/null | grep -q "Xcode.app"; then
  ok "Xcode" "$(xcode-select -p)"
else
  need "$ios" "Xcode" "install Xcode.app, then: sudo xcode-select -s /Applications/Xcode.app"
fi
if xcrun -sdk macosx metal --version >/dev/null 2>&1; then
  ok "metal" "shader compiler present"
else
  need "$ios" "metal/metallib" "needs full Xcode (Xcode 16.3+: xcodebuild -downloadComponent MetalToolchain)"
fi
if have rustup; then
  ok "rustup" "$(rustup --version 2>/dev/null | awk '{print $2}')"
  for t in aarch64-apple-ios aarch64-apple-ios-sim; do
    if rustup target list --installed 2>/dev/null | grep -q "^$t$"; then
      ok "target" "$t"
    else
      need "$ios" "target" "rustup target add $t"
    fi
  done
else
  need "$ios" "rustup" "Homebrew rust is host-only; install rustup for iOS targets"
fi

echo
if [ "$fail" -ne 0 ]; then
  echo -e "${red}preflight: FAILED${reset} — install the MISSING required tools above."
  exit 1
fi
echo -e "${green}preflight: OK${reset}"
