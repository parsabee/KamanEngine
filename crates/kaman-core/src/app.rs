// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The windowed entry — run a [`Game`](crate::Game) inside a `winit` window on
//! macOS.
//!
//! This is the platform half of the engine loop. It opens a window, translates
//! `winit` events into the engine's backend-agnostic [`InputState`], and drives
//! the *same* [`Game`](crate::Game) hooks in the *same* order as the headless
//! driver (`init` once, then `update`/`render` per frame). The only differences
//! from [`headless`](crate::headless) are that `dt` is measured from wall-clock
//! time and that per-frame drawing goes through [`present_frame`], the small
//! isolated function KE-0102 fills with the real Metal backend.
//!
//! # Phase-1 scope and deferrals
//!
//! In Phase 1 there is **no GPU backend** wired to the window yet. The engine
//! still owns a [`NullRenderer`](kaman_render_api::NullRenderer) so the game's
//! `render` hook has a live seam to record against, but [`present_frame`] does
//! not yet blit anything to the surface — that (surface acquisition, Metal
//! command submission, present) is KE-0102. Scene/camera wiring is likewise
//! deferred. The windowed path is therefore usable for input and game-logic
//! bring-up today, and becomes visible once the backend lands.
//!
//! # macOS isolation
//!
//! The window-creation and event-translation helpers are kept as small free
//! functions so the KE-0301 `#[cfg(target_os = ...)]` split can wrap them without
//! disturbing the loop structure. The engine loop itself
//! ([`EngineApp::drive_frame`]) is platform-neutral.

use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton as WinitMouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use kaman_ecs::hecs::World;
use kaman_perf::PerfTracker;
use kaman_render_api::NullRenderer;

use crate::context::EngineCtx;
use crate::game::Game;
use crate::input::{InputState, Key, MouseButton};

/// Open a window and run `game` until the window is closed (macOS).
///
/// Calls [`Game::init`](crate::Game::init) once when the window first becomes
/// active, then [`update`](crate::Game::update)/[`render`](crate::Game::render)
/// each frame — the same contract the [`headless`](crate::headless) driver
/// upholds — with `dt` measured between redraws. Returns when the event loop
/// exits (the user closes the window or presses Escape).
///
/// This requires a display and (once KE-0102 lands) a Metal device, so it is
/// **not** used by the smoke oracle or tests — those use
/// [`headless::run`](crate::headless::run). The doctest below is `no_run` for
/// exactly that reason.
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
    let mut app = EngineApp::new(game);
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
/// Mirrors the ownership of [`Headless`](crate::headless::Headless) — an ECS
/// [`World`], a render seam ([`NullRenderer`] in Phase 1), an [`InputState`], and
/// a [`PerfTracker`] — plus the window and the game reference. It is generic over
/// the [`Game`] and borrows it for the lifetime of the run.
struct EngineApp<'g, G: Game> {
    game: &'g mut G,
    window: Option<Window>,
    world: World,
    renderer: NullRenderer,
    input: InputState,
    perf: PerfTracker,
    last_frame: Option<Instant>,
    initialized: bool,
}

impl<'g, G: Game> EngineApp<'g, G> {
    fn new(game: &'g mut G) -> Self {
        Self {
            game,
            window: None,
            world: World::new(),
            renderer: NullRenderer::new(),
            input: InputState::new(),
            perf: PerfTracker::new(),
            last_frame: None,
            initialized: false,
        }
    }

    /// Run one frame: `update(dt)` then `render`, wrapped in perf timing.
    ///
    /// Platform-neutral: builds an [`EngineCtx`] over the app's engine state and
    /// calls the game hooks, exactly as the headless driver does. `dt` is the
    /// measured seconds since the previous frame.
    fn drive_frame(&mut self) {
        let now = Instant::now();
        let dt = match self.last_frame {
            Some(prev) => now.duration_since(prev).as_secs_f32(),
            None => crate::headless::FIXED_DT,
        };
        self.last_frame = Some(now);

        self.perf.begin_frame();
        let snapshot = self.perf.snapshot();

        {
            let mut ctx =
                EngineCtx::new(&mut self.world, &mut self.renderer, &self.input, snapshot);
            self.game.update(&mut ctx, dt);
            self.game.render(&mut ctx);
        }

        // Present whatever the game recorded. No-op in Phase 1 (KE-0102).
        present_frame(self.window.as_ref(), &mut self.renderer);

        self.perf.end_frame();
    }
}

impl<G: Game> ApplicationHandler for EngineApp<'_, G> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let window = create_window(event_loop);
            window.request_redraw();
            self.window = Some(window);
        }

        // First activation: run the game's one-time init against the live seam.
        if !self.initialized {
            let snapshot = self.perf.snapshot();
            let mut ctx =
                EngineCtx::new(&mut self.world, &mut self.renderer, &self.input, snapshot);
            self.game.init(&mut ctx);
            self.initialized = true;
        }
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
                            self.input.press_key(key);
                        } else {
                            self.input.release_key(key);
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(btn) = translate_mouse_button(button) {
                    match state {
                        ElementState::Pressed => self.input.press_mouse(btn),
                        ElementState::Released => self.input.release_mouse(btn),
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.input.set_cursor_position(position.x, position.y);
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

/// Present the frame the game recorded to the window surface.
///
/// **KE-0102 fills this in.** In Phase 1 there is no Metal backend attached to
/// the window, so this is intentionally a no-op beyond touching the recorder:
/// there is nothing to blit yet. When KE-0102 lands, this acquires the drawable,
/// submits the recorded command stream, and presents. Keeping it as one isolated
/// function is what confines the backend wiring to a single call site.
fn present_frame(_window: Option<&Window>, _renderer: &mut NullRenderer) {
    // No GPU present in Phase 1. The game has already recorded its frame into the
    // seam (`renderer`); KE-0102 will turn that recorded stream into a real Metal
    // present here.
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
        assert_eq!(
            translate_mouse_button(WinitMouseButton::Back),
            None
        );
    }
}
