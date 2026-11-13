# stickshift-capi

The Stick Shift Engine **C API**: `libstickshift.a` / `libstickshift.dylib` and the generated
[`include/stickshift.h`](include/stickshift.h), with a [`module.modulemap`](include/module.modulemap)
so Swift can `import StickShift`.

The contract (concepts, threading, what may wait in the beta, the reference) is
[docs/C_API.md](../../docs/C_API.md). In short, the API is **asynchronous in shape**:
calls return at once with a handle or a result code, resources go `PENDING` →
`READY` / `FAILED`, completions arrive as events through `stickshift_poll_events` or per-call
callbacks on the host's thread, and `stickshift_frame_acquire` never waits.

## Layout

| File | |
|---|---|
| `src/types.rs` | Every C-visible type and constant |
| `src/core.rs` | The boundary: errors, the per-thread last error, panic containment, version |
| `src/context.rs` | `StickShiftContext`: the busy guard, events and callbacks, housekeeping |
| `src/resources.rs`, `src/models.rs`, `src/fonts.rs` | Meshes and textures, glTF models (worker pool), SDF fonts |
| `src/frame.rs` | Recording and submitting frames, read-back |
| `src/backend.rs`, `src/metal.rs` | The backend trait, the null backend, and the Metal backend (macOS) |
| `src/handles.rs` | Generational handle tables (32-bit generations, never reissued) |

The crate is graphics-only: it depends on `stickshift-render` (macOS), `stickshift-render-api`,
`stickshift-assets` (without its `ecs` feature) and `stickshift-math`, and CI fails if a windowing,
audio, ECS or physics crate reaches it.

## Building and testing

```sh
cargo build --release -p stickshift-capi     # target/release/libstickshift.{a,dylib}
cargo test -p stickshift-capi                # Rust tests through the exported functions
./scripts/test-c-api.sh                 # compile + run the C smoke test and sample
./scripts/gen-header.sh                 # regenerate include/stickshift.h (cbindgen 0.29.0)
./scripts/gen-header.sh --check         # CI: fail if the committed header is stale
```

The tests run anywhere: off macOS (or with `STICKSHIFT_BACKEND_NULL`) the null backend stands in
for Metal. On macOS one test also checks the C API renders the same pixels as the Rust API.
