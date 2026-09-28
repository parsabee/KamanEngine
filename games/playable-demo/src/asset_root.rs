// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Where the demo's `assets/` directory lives at runtime.
//!
//! A `cargo run` finds the assets next to this crate's source, but an installed
//! binary (the Homebrew `kaman-demo`, or an unpacked release tarball) has no source
//! tree to point at. So the asset root is resolved **once**, at startup, by trying
//! these candidates in order and taking the first directory that exists:
//!
//! 1. `$KAMAN_DEMO_ASSETS`, if set and non-empty (an explicit override);
//! 2. `<exe dir>/../share/kaman-engine/assets` — the Homebrew / release-tarball
//!    layout (`bin/` beside `share/`);
//! 3. `<exe dir>/assets` — a flat layout with the assets beside the binary;
//! 4. this crate's own `assets/` at build time (`CARGO_MANIFEST_DIR`), so
//!    `cargo run` and `cargo test` work unchanged from a checkout.
//!
//! The exe dir is tried both after resolving symlinks (Homebrew links
//! `bin/kaman-demo` into its prefix; the real file lives in the keg, whose
//! `share/kaman-engine` is right beside it) and as invoked.
//!
//! Config constants in [`crate::config`] name files *relative* to this root;
//! [`asset_path`] joins them. Nothing is resolved per frame.

use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The environment variable that overrides the asset root.
pub(crate) const ENV_VAR: &str = "KAMAN_DEMO_ASSETS";

/// The asset root, resolved on first use.
static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// No candidate directory existed. Lists what was tried, in order.
#[derive(Debug)]
pub(crate) struct NotFound {
    tried: Vec<PathBuf>,
    env_set: bool,
}

impl fmt::Display for NotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "cannot find the demo's assets directory. Tried, in order:"
        )?;
        if !self.env_set {
            writeln!(f, "  - ${ENV_VAR} (not set)")?;
        }
        for path in &self.tried {
            writeln!(f, "  - {}", path.display())?;
        }
        write!(
            f,
            "Set {ENV_VAR} to the directory holding the demo's assets (e.g. road.gltf)."
        )
    }
}

/// The candidate asset roots, in priority order (see the module docs).
///
/// `env` is the value of [`ENV_VAR`] (an empty value counts as unset), `exe` the
/// running binary's path, `manifest_dir` this crate's build-time source dir.
fn candidates(env: Option<&OsStr>, exe: Option<&Path>, manifest_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = env.filter(|v| !v.is_empty()) {
        out.push(PathBuf::from(dir));
    }
    if let Some(exe) = exe {
        // Resolved first: that finds the keg's own share/ behind a Homebrew symlink.
        let mut exe_dirs: Vec<PathBuf> = Vec::new();
        for path in [std::fs::canonicalize(exe).ok(), Some(exe.to_path_buf())] {
            if let Some(dir) = path.as_deref().and_then(Path::parent) {
                if !exe_dirs.iter().any(|d| d == dir) {
                    exe_dirs.push(dir.to_path_buf());
                }
            }
        }
        for dir in &exe_dirs {
            out.push(dir.join("../share/kaman-engine/assets"));
        }
        for dir in &exe_dirs {
            out.push(dir.join("assets"));
        }
    }
    out.push(manifest_dir.join("assets"));
    out
}

/// The first candidate that is an existing directory.
fn resolve_from(
    env: Option<&OsStr>,
    exe: Option<&Path>,
    manifest_dir: &Path,
) -> Result<PathBuf, NotFound> {
    let tried = candidates(env, exe, manifest_dir);
    match tried.iter().find(|p| p.is_dir()) {
        // Canonical, so `..` and symlinks are gone from what we print and join.
        Some(found) => Ok(std::fs::canonicalize(found).unwrap_or_else(|_| found.clone())),
        None => Err(NotFound {
            tried,
            env_set: env.is_some_and(|v| !v.is_empty()),
        }),
    }
}

/// Resolve the asset root from the real environment, once. Later calls return
/// the cached root.
///
/// `main` calls this before booting the game so a missing install prints a
/// readable error instead of a panic.
pub(crate) fn init() -> Result<&'static Path, NotFound> {
    if let Some(root) = ROOT.get() {
        return Ok(root);
    }
    let env = std::env::var_os(ENV_VAR);
    let exe = std::env::current_exe().ok();
    let root = resolve_from(
        env.as_deref(),
        exe.as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )?;
    Ok(ROOT.get_or_init(|| root))
}

/// The resolved asset root. Resolves on first use (tests boot the game without
/// `main`); panics only if nothing exists, which `main` has already ruled out.
pub(crate) fn root() -> &'static Path {
    init().unwrap_or_else(|e| panic!("{e}"))
}

/// The full path of the asset file `name` (relative to the asset root).
pub(crate) fn asset_path(name: &str) -> String {
    root().join(name).to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A fresh, empty temp directory, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("kaman-asset-root-{}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            // Canonical, so comparisons survive macOS's /var -> /private/var link.
            TempDir(fs::canonicalize(&dir).unwrap())
        }

        fn mkdir(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            fs::create_dir_all(&p).unwrap();
            p
        }

        fn touch(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"").unwrap();
            p
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn same(a: &Path, b: &Path) -> bool {
        fs::canonicalize(a).unwrap() == fs::canonicalize(b).unwrap()
    }

    #[test]
    fn env_var_wins_over_everything() {
        let t = TempDir::new();
        let env = t.mkdir("custom");
        let exe = t.touch("prefix/bin/kaman-demo");
        t.mkdir("prefix/share/kaman-engine/assets");
        t.mkdir("prefix/bin/assets");
        let manifest = t.mkdir("src");
        t.mkdir("src/assets");

        let root = resolve_from(Some(env.as_os_str()), Some(&exe), &manifest).unwrap();
        assert!(same(&root, &env));
    }

    #[test]
    fn missing_or_empty_env_var_falls_through_to_share_layout() {
        let t = TempDir::new();
        let exe = t.touch("prefix/bin/kaman-demo");
        let share = t.mkdir("prefix/share/kaman-engine/assets");
        t.mkdir("prefix/bin/assets");
        let manifest = t.mkdir("src");
        t.mkdir("src/assets");

        let missing = t.0.join("nope");
        for env in [Some(missing.as_os_str()), Some(OsStr::new("")), None] {
            let root = resolve_from(env, Some(&exe), &manifest).unwrap();
            assert!(
                same(&root, &share),
                "env {env:?} resolved to {}",
                root.display()
            );
        }
    }

    #[test]
    fn flat_layout_beats_the_build_dir() {
        let t = TempDir::new();
        let exe = t.touch("flat/kaman-demo");
        let flat = t.mkdir("flat/assets");
        let manifest = t.mkdir("src");
        t.mkdir("src/assets");

        let root = resolve_from(None, Some(&exe), &manifest).unwrap();
        assert!(same(&root, &flat));
    }

    #[test]
    fn build_dir_is_the_last_resort() {
        let t = TempDir::new();
        let exe = t.touch("bin/kaman-demo");
        let manifest = t.mkdir("src");
        let src_assets = t.mkdir("src/assets");

        let root = resolve_from(None, Some(&exe), &manifest).unwrap();
        assert!(same(&root, &src_assets));
        let root = resolve_from(None, None, &manifest).unwrap();
        assert!(same(&root, &src_assets));
    }

    #[test]
    fn symlinked_binary_finds_the_keg_share_dir() {
        // Homebrew: <prefix>/bin/kaman-demo -> ../Cellar/kaman-engine/<v>/bin/kaman-demo,
        // and the keg holds share/kaman-engine/assets. Nothing is linked into the
        // prefix's share/, so only the resolved path can find it.
        let t = TempDir::new();
        let real = t.touch("Cellar/kaman-engine/1.0/bin/kaman-demo");
        let keg_assets = t.mkdir("Cellar/kaman-engine/1.0/share/kaman-engine/assets");
        t.mkdir("prefix/bin");
        let link = t.0.join("prefix/bin/kaman-demo");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let manifest = t.mkdir("src");

        let root = resolve_from(None, Some(&link), &manifest).unwrap();
        assert!(same(&root, &keg_assets));
    }

    #[test]
    fn nothing_found_lists_every_candidate() {
        let t = TempDir::new();
        let exe = t.touch("bin/kaman-demo");
        let manifest = t.0.join("gone");

        let err = resolve_from(None, Some(&exe), &manifest).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("$KAMAN_DEMO_ASSETS (not set)"), "{msg}");
        assert!(msg.contains("share/kaman-engine/assets"), "{msg}");
        assert!(
            msg.contains(&manifest.join("assets").display().to_string()),
            "{msg}"
        );
        assert_eq!(err.tried.len(), 3, "share, flat, build dir: {msg}");

        let env = t.0.join("custom");
        let err = resolve_from(Some(env.as_os_str()), Some(&exe), &manifest).unwrap_err();
        let msg = err.to_string();
        assert!(!msg.contains("not set"), "{msg}");
        assert!(msg.contains(&env.display().to_string()), "{msg}");
    }

    #[test]
    fn the_real_root_holds_the_demo_assets() {
        // Under `cargo test` the root resolves (to the checkout's assets/) and
        // every path joins onto it.
        assert!(root().join("road.gltf").is_file());
        assert!(asset_path("font.bin").ends_with("font.bin"));
    }
}
