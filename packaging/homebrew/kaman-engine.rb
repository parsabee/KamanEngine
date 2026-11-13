# frozen_string_literal: true

# Canonical copy of the Homebrew formula published in the parsabee/homebrew-kaman tap:
#
#   brew install parsabee/kaman/kaman-engine
#
# Installs the prebuilt `kaman-demo` (the playable demo), the C library (`libkaman`, its header,
# Swift module map and pkg-config file) and the engine source, from the Apple Silicon tarball that
# .github/workflows/release.yml attaches to each GitHub release.
# Kept current automatically: on every v* tag, release.yml runs scripts/update-homebrew-formula.sh
# to point `url`/`sha256` at the new tarball, here and in the tap.
class KamanEngine < Formula
  desc "Metal graphics engine for Apple Silicon: a C library, Rust crates and a demo"
  homepage "https://github.com/parsabee/KamanEngine"
  url "https://github.com/parsabee/KamanEngine/releases/download/v0.1.0-beta.1/kaman-engine-0.1.0-beta.1-aarch64-apple-darwin.tar.gz"
  sha256 "ecec5320082b79964c6d64bae9f108e5624215ba8917ec5d20453cebf1399694"
  license "Apache-2.0"

  # Prebuilt for Apple Silicon only. On an Intel Mac (or Linux) brew refuses to install with
  # "The arm64 architecture is required for this software." (or the macOS requirement).
  depends_on arch: :arm64
  depends_on :macos

  def install
    bin.install "bin/kaman-demo"
    lib.install "lib/libkaman.a", "lib/libkaman.dylib"
    (lib/"pkgconfig").install "lib/pkgconfig/kaman.pc"
    # In include/kaman/ so the generic module.modulemap never collides in the shared prefix.
    (include/"kaman").install "include/kaman/kaman.h", "include/kaman/module.modulemap"
    # `kaman-demo` looks for its assets in <bin>/../share/kaman-engine/assets, which is
    # exactly pkgshare/"assets" in the keg.
    pkgshare.install "share/kaman-engine/assets"
    pkgshare.install "share/kaman-engine/src"
  end

  def caveats
    <<~EOS
      Run the playable demo (a Metal window; arrow keys to change lanes, Space to start):
        kaman-demo

      The C library: #include "kaman.h" and link libkaman, e.g. with pkg-config:
        cc app.c $(pkg-config --cflags --libs kaman) -o app
      Swift: add #{opt_include}/kaman to the import paths and `import Kaman`.
      The API is described in #{opt_pkgshare}/src/docs/C_API.md.

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

    (testpath/"version.c").write <<~C
      #include <stdio.h>
      #include "kaman.h"
      int main(void) {
        printf("%s\n", kaman_version_string());
        return kaman_api_version() == KAMAN_API_VERSION ? 0 : 1;
      }
    C
    system ENV.cc, "-std=c11", "version.c", "-I#{include}/kaman", "-L#{lib}", "-lkaman",
           "-Wl,-rpath,#{lib}", "-o", "version"
    assert_match version.to_s, shell_output("./version")
  end
end
