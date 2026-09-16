//! `car-runner` — KamanEngine's first title, and the host for the headless smoke oracle.
//!
//! Phase 0 has no renderer yet (that migrates in Phase 1), so this binary's real job today is
//! the `--smoke` oracle: it boots a fixed scene and drives a fixed number of clear-color
//! frames offscreen, then exits 0. The oracle deliberately requires **no GPU / Metal device**
//! so it runs on headless GitHub macOS CI runners. See `docs/INTEGRATION.md` §1.

use clap::Parser;

/// Number of frames the smoke oracle simulates before exiting.
const SMOKE_FRAMES: u32 = 120;

/// Command-line arguments for `car-runner`.
#[derive(Parser, Debug)]
#[command(name = "car-runner", about = "KamanEngine car-runner + headless smoke oracle")]
struct Cli {
    /// Run the headless smoke oracle: simulate a fixed clear-color frame loop and exit 0.
    ///
    /// Requires no GPU/Metal device — safe on headless CI runners.
    #[arg(long)]
    smoke: bool,
}

fn main() {
    let cli = Cli::parse();

    if cli.smoke {
        run_smoke(SMOKE_FRAMES);
        return;
    }

    // No interactive game loop yet — the windowing/renderer path migrates in Phase 1.
    println!("car-runner: no interactive mode yet (Phase 1). Try `--smoke`.");
}

/// Boot a fixed scene and simulate `frames` clear-color frames offscreen, then report success.
///
/// This is a CPU-only stand-in for the real render loop: it performs no GPU work, so it is
/// valid on headless CI. It exists to prove the workspace builds and the app entry point runs
/// end-to-end on every commit (the "continuous oracle" from `docs/INTEGRATION.md`).
fn run_smoke(frames: u32) {
    // A fixed "scene": a single clear color. Phase 1 replaces this with a real offscreen
    // render + golden pixel hash.
    let clear_color = [0.1_f32, 0.2, 0.3, 1.0];
    let mut acc = 0.0_f32;

    for frame in 0..frames {
        // Simulate per-frame work so the loop can't be optimized away and any panic surfaces.
        acc += clear_color[(frame % 4) as usize];
    }

    // Sanity: the loop actually ran over the clear color `frames` times.
    debug_assert!(acc > 0.0);
    let _ = acc;

    println!("smoke: {frames} frames OK");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_runs_without_panicking() {
        // The oracle must complete for the fixed frame count without requiring a GPU.
        run_smoke(SMOKE_FRAMES);
    }
}
