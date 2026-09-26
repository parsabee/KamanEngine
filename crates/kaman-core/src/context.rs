// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! [`EngineCtx`] — the narrow handle the engine hands a [`Game`](crate::Game).
//!
//! `EngineCtx` is the *only* channel through which game code reaches engine
//! services. It deliberately exposes a small, curated surface — the ECS world,
//! the render seam, an input snapshot, and frame timing — and nothing else. No
//! Metal type and no game type ever crosses it (ARCHITECTURE §3).
//!
//! # Why accessors, not fields
//!
//! `EngineCtx` is the borrow-checker chokepoint called out in the ticket. Rather
//! than hand the game long-lived `&mut` handles to individual subsystems (which
//! would let it alias engine state across a frame), the engine builds a fresh
//! `EngineCtx` borrowing its state and passes `&mut EngineCtx` into each
//! lifecycle hook. The game reaches each service through a short-lived borrow
//! from an accessor ([`world_mut`](EngineCtx::world_mut),
//! [`renderer`](EngineCtx::renderer), …); those borrows end at the end of the
//! statement, so the game cannot hold two conflicting mutable views at once.

use kaman_audio::Audio;
use kaman_camera::Camera;
use kaman_ecs::hecs::World;
use kaman_perf::PerfSnapshot;
use kaman_render_api::{FrameRecorder, RenderDevice};
use kaman_scene::Scene;

use crate::input::InputState;

/// The combined render seam: a type that is both a [`RenderDevice`] (load-time
/// resource ownership) and a [`FrameRecorder`] (per-frame command recording).
///
/// The engine owns one renderer implementing both halves of the
/// `kaman-render-api` seam and exposes it to the game as a single
/// `&mut dyn Renderer`. In Phase 1 the concrete backend is
/// [`NullRenderer`](kaman_render_api::NullRenderer); KE-0102 swaps in the real
/// Metal backend behind the same trait object, so game code is unaffected.
///
/// This is a pure marker: it adds no methods, only bundles the two seam traits
/// so a single trait object can be handed across the boundary. A blanket impl
/// covers any type that implements both, so backends never implement it
/// explicitly.
pub trait Renderer: RenderDevice + FrameRecorder {}

impl<T: RenderDevice + FrameRecorder> Renderer for T {}

/// The engine-services handle passed into every [`Game`](crate::Game) hook.
///
/// `EngineCtx` borrows the engine's live state for the duration of a single
/// hook call. Its lifetime `'a` ties it to that borrow: the engine constructs it
/// immediately before calling [`init`](crate::Game::init),
/// [`update`](crate::Game::update), or [`render`](crate::Game::render), and drops
/// it immediately after, so no game can stash it across frames.
///
/// # What a game may touch
///
/// - The [`Scene`] — [`scene`](Self::scene) / [`scene_mut`](Self::scene_mut),
///   which owns the ECS [`World`] and the physics world and drives streaming and
///   the floating-origin rebase.
/// - The ECS [`World`] — [`world`](Self::world) / [`world_mut`](Self::world_mut),
///   convenience accessors that delegate to the scene's world.
/// - The render seam — [`renderer`](Self::renderer), a `&mut dyn Renderer`
///   ([`RenderDevice`] + [`FrameRecorder`]). The game records draws here in
///   [`render`](crate::Game::render); it never sees a Metal type.
/// - The [`Camera`] — [`camera`](Self::camera) / [`camera_mut`](Self::camera_mut).
///   The game positions it (e.g. a chase camera following the player); the engine
///   pushes its view-projection across the render seam each frame before
///   [`render`](crate::Game::render), so the backend stays camera-free.
/// - The [`InputState`] snapshot — [`input`](Self::input) (read-only; the engine
///   owns input).
/// - The [`Audio`] layer — [`audio`](Self::audio). Load sounds in
///   [`init`](crate::Game::init), then play them from game events. It needs no
///   audio device and never fails, so it is called unconditionally.
/// - Frame timing — [`perf`](Self::perf), a [`PerfSnapshot`] for the previous
///   frame.
///
/// There is deliberately no accessor for anything else: no window, no platform
/// handle, no Metal device, and no game state.
pub struct EngineCtx<'a> {
    scene: &'a mut Scene,
    renderer: &'a mut dyn Renderer,
    camera: &'a mut Camera,
    input: &'a InputState,
    audio: &'a mut Audio,
    perf: PerfSnapshot,
    alpha: f32,
}

impl<'a> EngineCtx<'a> {
    /// Assemble a context borrowing the engine's state for one hook call.
    ///
    /// This is `pub(crate)`: only the engine's loop/driver constructs an
    /// `EngineCtx`. Each of `scene`, `renderer`, `input`, and `audio` is a distinct
    /// field of engine-owned state, so the borrows do not alias.
    ///
    /// `alpha` is the fixed-timestep interpolation factor (see
    /// [`alpha`](Self::alpha)); it is meaningful on the render path and `0.0` for
    /// `update`/`init` contexts, where interpolation does not apply.
    pub(crate) fn new(
        scene: &'a mut Scene,
        renderer: &'a mut dyn Renderer,
        camera: &'a mut Camera,
        input: &'a InputState,
        audio: &'a mut Audio,
        perf: PerfSnapshot,
        alpha: f32,
    ) -> Self {
        Self {
            scene,
            renderer,
            camera,
            input,
            audio,
            perf,
            alpha,
        }
    }

    /// Shared access to the [`Scene`] — the ECS world, the physics world, and the
    /// streaming / rebase bookkeeping.
    ///
    /// Use this for read-only physics queries or to inspect streaming state; use
    /// [`scene_mut`](Self::scene_mut) to spawn bodies, step physics, or drive
    /// [`Scene::stream`] / [`Scene::maybe_rebase`].
    #[must_use]
    pub fn scene(&self) -> &Scene {
        self.scene
    }

    /// Mutable access to the [`Scene`].
    ///
    /// This is the handle a streaming game drives: spawn entities with physics
    /// bodies, call [`Scene::stream`] with the current focus and a spawn callback,
    /// [`Scene::step_physics`], and [`Scene::maybe_rebase`]. The returned borrow
    /// lasts only as long as you hold it, so it cannot alias another accessor.
    #[must_use]
    pub fn scene_mut(&mut self) -> &mut Scene {
        self.scene
    }

    /// Shared access to the ECS [`World`] (delegates to the scene's world).
    ///
    /// Use for read-only queries; call [`world_mut`](Self::world_mut) to spawn,
    /// despawn, or mutate components.
    #[must_use]
    pub fn world(&self) -> &World {
        self.scene.world()
    }

    /// Mutable access to the ECS [`World`] (delegates to the scene's world) —
    /// spawn, despawn, and mutate components.
    ///
    /// The returned borrow lasts only as long as you hold it; it cannot be kept
    /// past the current statement while also reaching another accessor, which is
    /// what prevents aliasing engine state.
    #[must_use]
    pub fn world_mut(&mut self) -> &mut World {
        self.scene.world_mut()
    }

    /// The render seam: a `&mut dyn Renderer` that is both a [`RenderDevice`] and
    /// a [`FrameRecorder`].
    ///
    /// A game creates resources (at load time, typically in
    /// [`init`](crate::Game::init)) and records the frame (in
    /// [`render`](crate::Game::render)) through this one handle. The concrete
    /// backend is [`NullRenderer`](kaman_render_api::NullRenderer) in Phase 1 and
    /// the Metal backend from KE-0102 later — the game code is identical either
    /// way because it only ever sees the trait object.
    #[must_use]
    pub fn renderer(&mut self) -> &mut dyn Renderer {
        self.renderer
    }

    /// Shared access to the engine [`Camera`].
    ///
    /// Read the current view/projection or basis vectors; use
    /// [`camera_mut`](Self::camera_mut) to move it.
    #[must_use]
    pub fn camera(&self) -> &Camera {
        self.camera
    }

    /// Mutable access to the engine [`Camera`] (KE-0205).
    ///
    /// Position it here — typically by driving a
    /// [`kaman_camera::ChaseController`] toward the player each frame in
    /// [`render`](crate::Game::render) (or [`update`](crate::Game::update)). The
    /// engine reads `camera().view_projection_matrix()` and pushes it across the
    /// render seam **before** the game's `render` runs, so whatever pose the game
    /// last set is the pose the frame is drawn from. The game never touches the
    /// render backend's camera (there isn't one) — the view crosses the seam as a
    /// plain matrix, accompanied by `camera().position()` for view-dependent
    /// shading (KE-0406).
    #[must_use]
    pub fn camera_mut(&mut self) -> &mut Camera {
        self.camera
    }

    /// The read-only [`InputState`] snapshot for the current frame.
    ///
    /// Read keyboard/mouse state here during [`update`](crate::Game::update). The
    /// engine owns input, so this is shared (not mutable) access.
    #[must_use]
    pub fn input(&self) -> &InputState {
        self.input
    }

    /// The engine's [`Audio`] layer (KE-0405) — load sounds, play one-shots, drive
    /// the looping bed, set the master level.
    ///
    /// Mutable because every audio operation mutates the mixer (a load registers a
    /// sound, a play starts one). Load in [`init`](crate::Game::init) — decoding is
    /// load-time work, and a repeat load of the same path is free — then trigger
    /// playback from game *events*, not every step: the loop never polls audio for
    /// you and nothing here belongs on the fixed-update hot path.
    ///
    /// **It works with no audio device.** The engine hands a game either a live
    /// mixer (the windowed entry) or a silent no-op that accepts everything and
    /// produces nothing (the headless driver, and any machine with no output
    /// device). Both behave identically from here, so game code has no `cfg`, no
    /// availability check, and nothing to unwrap.
    #[must_use]
    pub fn audio(&mut self) -> &mut Audio {
        self.audio
    }

    /// The [`PerfSnapshot`] describing recent frame timing.
    ///
    /// A `PerfSnapshot` is a `Copy` value captured by the engine before the hook
    /// runs; it reflects the rolling window up to (not including) the current
    /// frame.
    #[must_use]
    pub fn perf(&self) -> PerfSnapshot {
        self.perf
    }

    /// The fixed-timestep **interpolation factor** in `0.0..=1.0`, for the
    /// render path.
    ///
    /// Because the engine simulates in fixed [`FIXED_DT`](crate::FIXED_DT) steps
    /// but renders once per display frame, the render usually falls *between* two
    /// simulated states. `alpha` is the fraction of a fixed step that has elapsed
    /// since the last [`update`](crate::Game::update): `0.0` means "exactly at the
    /// last fixed state", approaching `1.0` means "almost at the next one". A game
    /// may lerp between the previous and current fixed states by this amount in
    /// [`render`](crate::Game::render) for motion that stays smooth even when the
    /// display rate doesn't divide evenly into the fixed rate.
    ///
    /// It is `0.0` in [`init`](crate::Game::init) and
    /// [`update`](crate::Game::update) contexts, where interpolation has no
    /// meaning. Phase 2 games may ignore it; it is wired through for later use.
    #[must_use]
    pub fn alpha(&self) -> f32 {
        self.alpha
    }
}
