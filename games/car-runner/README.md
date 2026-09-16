# car-runner

KamanEngine's first title (an infinite car runner) and the host for the headless
**smoke oracle**.

```sh
cargo run -p car-runner -- --smoke
```

The `--smoke` oracle boots a fixed scene and simulates a 120-frame clear-color loop
offscreen, then exits 0. It requires **no GPU/Metal device**, so it runs on headless CI
runners. The real windowing/renderer path migrates in Phase 1.

Part of the [KamanEngine](../../README.md) workspace. Apache-2.0.
