// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The windowed entry: run a [`Game`] inside a `winit` window on
//! macOS.
//!
//! This is the platform half of the engine loop. It opens a window, translates
//! `winit` events into the engine's backend-agnostic
//! [`InputState`](crate::InputState), and drives the *same* shared
//! [`drive_frame`] as the headless driver — the only
//! difference is the clock. Here the per-frame elapsed time comes from a real
//! monotonic [`Instant`], so the fixed-timestep
//! [`Accumulator`](crate::timestep::Accumulator) may run 0..N `update`s before the
//! single `render`; per-frame drawing goes through the backend the game binary
//! supplies via a **backend factory**.
//!
//! # The backend seam (metal stays out of `kaman-core`)
//!
//! `kaman-core` must never depend on `metal` or on the concrete `kaman-render`
//! crate (ARCHITECTURE §2, enforced by CI). So the windowed entry does not
//! construct a backend itself: [`run_with_backend`] takes a **factory** (a
//! `FnOnce(&Window, u32, u32) -> Box<dyn Renderer>`) that the *game binary*
//! (which does depend on `kaman-render`) provides. The factory is invoked once
//! in `resumed`, right after the window is created, and
//! the returned `Box<dyn Renderer>` drives every frame through the seam. The
//! headless driver and the plain [`run`] entry keep the GPU-free
//! [`NullRenderer`].
//!
//! # Audio (KE-0405)
//!
//! This is also the only entry that opens an **audio output device**: it replaces
//! the loop's silent [`Audio`](kaman_audio::Audio) layer with
//! [`Audio::with_output_device`](kaman_audio::Audio::with_output_device) at
//! startup. The headless driver keeps the silent layer, so tests and `--smoke`
//! never touch audio hardware; and since the device-bound constructor falls back
//! to silence when there is no output, even this path cannot fail on that account.
//!
//! # macOS isolation
//!
//! The window-creation and event-translation helpers are kept as small free
//! functions so the KE-0301 `#[cfg(target_os = ...)]` split can wrap them without
//! disturbing the loop structure. The engine loop itself
//! (`drive_frame`) is platform-neutral.

use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton as WinitMouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use kaman_render_api::NullRenderer;

use crate::context::Renderer;
use crate::driver::{drive_frame, Loop};
use crate::game::Game;
use crate::input::{Key, MouseButton};

/// A factory that constructs the render backend for a freshly-created window.
///
/// The engine calls this once, in `resumed`, after the
/// window exists, passing the window plus its pixel size. The game binary (the
/// only place that depends on `kaman-render`) returns a boxed
/// [`Renderer`] — a type that is both a
/// [`RenderDevice`](kaman_render_api::RenderDevice) and a
/// [`FrameRecorder`](kaman_render_api::FrameRecorder). This is the single seam
/// that keeps `metal` out of `kaman-core`'s dependency tree while still letting
/// the windowed loop drive a real GPU backend.
pub type BackendFactory<'f> = Box<dyn FnOnce(&Window, u32, u32) -> Box<dyn Renderer> + 'f>;

/// Open a window and run `game` against a headless [`NullRenderer`] until the
/// window is closed (macOS).
///
/// This convenience entry wires **no GPU backend**; the window is present but
/// nothing is drawn to it (the game still records into the null seam). Use
/// [`run_with_backend`] to supply a real backend. Kept for tests and callers
/// that only need the input/loop bring-up.
///
/// # Panics
///
/// Panics if the platform event loop cannot be created (no display available).
///
/// # Example
///
/// ```no_run
/// use kaman_core::{Game, EngineCtx, run};
///
/// struct MyGame;
/// impl Game for MyGame {
///     fn init(&mut self, _: &mut EngineCtx) {}
///     fn update(&mut self, _: &mut EngineCtx, _dt: f32) {}
///     fn render(&mut self, _: &mut EngineCtx) {}
/// }
///
/// run(&mut MyGame);
/// ```
pub fn run<G: Game>(game: &mut G) {
    let event_loop = EventLoop::new().expect("failed to create winit event loop");
    let mut app: EngineApp<'_, '_, G> = EngineApp::new(game, None);
    event_loop
        .run_app(&mut app)
        .expect("winit event loop failed");
}

/// Open a window and run `game`, constructing the render backend from `factory`.
///
/// Identical to [`run`] except that the engine calls `factory` once after the
/// window is created and drives every frame through the returned
/// `Box<dyn Renderer>` — the real GPU present path. The factory is the seam that
/// lets the game binary inject a `metal` backend without `kaman-core` depending
/// on `metal`.
///
/// # Panics
///
/// Panics if the platform event loop cannot be created (no display available).
///
/// # Example
///
/// ```no_run
/// use kaman_core::{Game, EngineCtx, run_with_backend, Renderer};
/// use kaman_render_api::NullRenderer;
///
/// struct MyGame;
/// impl Game for MyGame {
///     fn init(&mut self, _: &mut EngineCtx) {}
///     fn update(&mut self, _: &mut EngineCtx, _dt: f32) {}
///     fn render(&mut self, _: &mut EngineCtx) {}
/// }
///
/// // A real game passes a Metal backend here; this doc uses NullRenderer.
/// run_with_backend(&mut MyGame, Box::new(|_win, _w, _h| {
///     Box::new(NullRenderer::new()) as Box<dyn Renderer>
/// }));
/// ```
pub fn run_with_backend<G: Game>(game: &mut G, factory: BackendFactory<'_>) {
    let event_loop = EventLoop::new().expect("failed to create winit event loop");
    let mut app: EngineApp<'_, '_, G> = EngineApp::new(game, Some(factory));
    event_loop
        .run_app(&mut app)
        .expect("winit event loop failed");
}

/// Window title used by the windowed entry.
const WINDOW_TITLE: &str = "KamanEngine";
/// Initial window width in logical pixels.
const WINDOW_WIDTH: u32 = 800;
/// Initial window height in logical pixels.
const WINDOW_HEIGHT: u32 = 600;

/// The `winit` [`ApplicationHandler`] that owns engine state and drives the game.
///
/// Holds the shared [`Loop`] (ECS [`World`](kaman_ecs::hecs::World), input, perf,
/// and the fixed-timestep [`Accumulator`](crate::timestep::Accumulator)) exactly
/// like [`Headless`](crate::headless::Headless) does, plus the window, the render
/// seam, and the game reference. The render seam is a `Box<dyn Renderer>`: a
/// [`NullRenderer`] until (and unless) the backend factory replaces it with a
/// real backend on first resume.
struct EngineApp<'g, 'f, G: Game> {
    game: &'g mut G,
    window: Option<Window>,
    lp: Loop,
    renderer: Box<dyn Renderer>,
    factory: Option<BackendFactory<'f>>,
    last_frame: Option<Instant>,
}

impl<'g, 'f, G: Game> EngineApp<'g, 'f, G> {
    fn new(game: &'g mut G, factory: Option<BackendFactory<'f>>) -> Self {
        // A fresh `Loop` is silent (it opens no audio device, which is what keeps
        // headless runs and tests quiet). The windowed entry is the one place that
        // *wants* speakers, so bind the mixer to the system's default output here —
        // exactly once, at startup, before the game's `init` can load a sound.
        //
        // `with_output_device` cannot fail: a machine with no output device falls
        // back to the same silent no-op, so a windowed run on a headless CI box
        // still runs, just quietly. Audio is intentionally *not* deferred to
        // `resumed` like the render backend is — it needs no window.
        let mut lp = Loop::new();
        lp.audio = kaman_audio::Audio::with_output_device();

        Self {
            game,
            window: None,
            lp,
            renderer: Box::new(NullRenderer::new()),
            factory,
            last_frame: None,
        }
    }

    /// Run one display frame through the shared [`drive_frame`].
    ///
    /// Measures the real seconds since the previous frame from a monotonic
    /// [`Instant`] (the *only* thing this driver adds over the headless one) and
    /// hands that elapsed time to the shared accumulator-based loop, which runs
    /// 0..N fixed `update`s and one `render`. The game records its frame through
    /// the seam (`begin_frame` … `submit`), which the real backend turns into a
    /// GPU present.
    fn drive_frame(&mut self) {
        let now = Instant::now();
        let elapsed = match self.last_frame {
            Some(prev) => now.duration_since(prev),
            // First frame: bank exactly one fixed step so a game still ticks once.
            None => std::time::Duration::from_secs_f64(f64::from(crate::FIXED_DT)),
        };
        self.last_frame = Some(now);

        drive_frame(&mut self.lp, self.game, self.renderer.as_mut(), elapsed);
    }
}

impl<G: Game> ApplicationHandler for EngineApp<'_, '_, G> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let window = create_window(event_loop);

            // Match the engine camera's aspect to the real window before the first
            // frame (KE-0205); resize events keep it current thereafter.
            let size = window.inner_size();
            self.lp
                .camera
                .set_aspect_ratio(size.width as f32 / size.height.max(1) as f32);

            // Construct the real backend now that a window exists. Consuming the
            // factory (an `Option`) makes this happen exactly once.
            if let Some(factory) = self.factory.take() {
                self.renderer = factory(&window, size.width, size.height);
            }

            window.request_redraw();
            self.window = Some(window);
        }

        // First activation: run the game's one-time init against the live seam
        // through the shared loop (idempotent).
        self.lp.init_once(self.game, self.renderer.as_mut());
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if let Some(key) = translate_key(code) {
                        if event.state.is_pressed() {
                            // Escape quits, matching the close button.
                            if key == Key::Escape {
                                event_loop.exit();
                            }
                            self.lp.input.press_key(key);
                        } else {
                            self.lp.input.release_key(key);
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(btn) = translate_mouse_button(button) {
                    match state {
                        ElementState::Pressed => self.lp.input.press_mouse(btn),
                        ElementState::Released => self.lp.input.release_mouse(btn),
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.lp.input.set_cursor_position(position.x, position.y);
            }
            WindowEvent::Resized(size) => {
                // Keep the engine camera's projection correct as the window
                // resizes (KE-0205). The backend below the seam owns no camera, so
                // aspect is maintained here on the engine-owned `Camera`.
                let aspect = size.width as f32 / size.height.max(1) as f32;
                self.lp.camera.set_aspect_ratio(aspect);
            }
            WindowEvent::RedrawRequested => {
                self.drive_frame();
                // Request the next frame to keep the loop pumping.
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

// --- macOS-isolated helpers -------------------------------------------------
//
// These are deliberately small free functions. The KE-0301 platform split will
// wrap them (or provide iOS variants) without touching `EngineApp`'s loop.

/// Create the application window. Isolated so the platform split (KE-0301) can
/// swap window construction per target.
fn create_window(event_loop: &ActiveEventLoop) -> Window {
    let attrs = Window::default_attributes()
        .with_title(WINDOW_TITLE)
        .with_inner_size(winit::dpi::LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT));
    event_loop
        .create_window(attrs)
        .expect("failed to create window")
}

/// Translate a `winit` physical key into the engine's backend-agnostic [`Key`].
///
/// Returns `None` for keys outside the Phase-1 subset (see [`Key`]). Isolated so
/// the mapping is a single, testable place and so the platform split can extend
/// it per target.
fn translate_key(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::KeyW => Key::W,
        KeyCode::KeyA => Key::A,
        KeyCode::KeyS => Key::S,
        KeyCode::KeyD => Key::D,
        KeyCode::KeyQ => Key::Q,
        KeyCode::KeyE => Key::E,
        KeyCode::ArrowLeft => Key::Left,
        KeyCode::ArrowRight => Key::Right,
        KeyCode::ArrowUp => Key::Up,
        KeyCode::ArrowDown => Key::Down,
        KeyCode::Space => Key::Space,
        KeyCode::Escape => Key::Escape,
        _ => return None,
    })
}

/// Translate a `winit` mouse button into the engine's [`MouseButton`].
///
/// Returns `None` for buttons outside the Phase-1 subset.
fn translate_mouse_button(button: WinitMouseButton) -> Option<MouseButton> {
    Some(match button {
        WinitMouseButton::Left => MouseButton::Left,
        WinitMouseButton::Right => MouseButton::Right,
        WinitMouseButton::Middle => MouseButton::Middle,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_translation_covers_the_phase1_subset() {
        assert_eq!(translate_key(KeyCode::KeyW), Some(Key::W));
        assert_eq!(translate_key(KeyCode::ArrowLeft), Some(Key::Left));
        assert_eq!(translate_key(KeyCode::Escape), Some(Key::Escape));
        // A key outside the subset is ignored, not mistranslated.
        assert_eq!(translate_key(KeyCode::F1), None);
    }

    #[test]
    fn mouse_button_translation() {
        assert_eq!(
            translate_mouse_button(WinitMouseButton::Left),
            Some(MouseButton::Left)
        );
        assert_eq!(translate_mouse_button(WinitMouseButton::Back), None);
    }

    /// The backend factory type must produce a live `Box<dyn Renderer>` and the
    /// game must be able to record a frame against it. This exercises the seam
    /// wiring the windowed path uses, without opening a window (which needs a
    /// display). A `NullRenderer` stands in for a real GPU backend.
    #[test]
    fn backend_factory_yields_a_usable_renderer() {
        use kaman_render_api::{MaterialParams, NullRenderer};
        use kaman_math::Transform;

        // A factory shaped exactly like the windowed entry expects. We can't
        // pass a real `&Window` here, so call the closure's body directly with
        // the same output type it must produce.
        let make: fn() -> Box<dyn Renderer> = || Box::new(NullRenderer::new());
        let mut renderer = make();

        let mesh = renderer.create_mesh(&kaman_render_api::MeshData {
            vertices: &[],
            indices: &[],
            layout: kaman_render_api::VertexLayout::default(),
        });
        renderer.begin_frame();
        let pipeline = renderer.create_pipeline(&kaman_render_api::PipelineDescriptor {
            vertex_shader: "vs".into(),
            fragment_shader: "fs".into(),
            vertex_layout: kaman_render_api::VertexLayout::default(),
        });
        renderer.set_pipeline(pipeline);
        renderer.draw_mesh(mesh, &Transform::identity(), &MaterialParams::default());
        renderer.submit();
        // If we got here, the trait object satisfied both seam halves.
    }
}
