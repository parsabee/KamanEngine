# frozen_string_literal: true

# Canonical copy of the Homebrew formula published in the parsabee/homebrew-kaman tap:
#
#   brew install parsabee/kaman/kaman-engine
#
# Installs the prebuilt `kaman-demo` (the playable demo) and the engine source, from the
# Apple Silicon tarball that .github/workflows/release.yml attaches to each GitHub release.
# Kept current automatically: on every v* tag, release.yml runs scripts/update-homebrew-formula.sh
# to point `url`/`sha256` at the new tarball, here and in the tap.
class KamanEngine < Formula
  desc "Rust game engine for Apple platforms with raw Metal, plus its playable demo"
  homepage "https://github.com/parsabee/KamanEngine"
  url "https://github.com/parsabee/KamanEngine/releases/download/v0.1.0-alpha.2/kaman-engine-0.1.0-alpha.2-aarch64-apple-darwin.tar.gz"
  sha256 "4ce9564ad61bbd1e8ad8711d3cc49600de0a6e85b1a3e49a6e4eeb27b3597a42"
  license "Apache-2.0"

  # Prebuilt for Apple Silicon only. On an Intel Mac (or Linux) brew refuses to install with
  # "The arm64 architecture is required for this software." (or the macOS requirement).
  depends_on arch: :arm64
  depends_on :macos

  def install
    bin.install "bin/kaman-demo"
    # `kaman-demo` looks for its assets in <bin>/../share/kaman-engine/assets, which is
    # exactly pkgshare/"assets" in the keg.
    pkgshare.install "share/kaman-engine/assets"
    pkgshare.install "share/kaman-engine/src"
  end

  def caveats
    <<~EOS
      Run the playable demo (a Metal window; arrow keys to change lanes, Space to start):
        kaman-demo

      The engine source is installed for use as Cargo path dependencies, e.g.:
        [dependencies]
        kaman-core = { path = "#{opt_pkgshare}/src/crates/kaman-core" }
        kaman-math = { path = "#{opt_pkgshare}/src/crates/kaman-math" }
      Building against it needs Rust 1.91 or newer (see #{opt_pkgshare}/src/rust-toolchain.toml).

      Documentation:
        #{opt_pkgshare}/src/README.md
        #{opt_pkgshare}/src/docs/GETTING_STARTED.md
        https://parsabee.github.io/KamanEngine/
    EOS
  end

  test do
    assert_match "share/kaman-engine/assets", shell_output("#{bin}/kaman-demo --print-assets-dir")
    assert_match "smoke: 120 frames OK", shell_output("#{bin}/kaman-demo --smoke")
  end
end
