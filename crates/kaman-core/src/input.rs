// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! A minimal, engine-generic input snapshot exposed to games through
//! [`EngineCtx`](crate::EngineCtx).
//!
//! Phase 1 keeps this deliberately small: pressed keys, cursor position, and
//! pressed mouse buttons — the read-only state a game needs to react to input in
//! [`Game::update`](crate::Game::update). The full input abstraction (action
//! mapping, per-frame just-pressed/just-released edges, gamepad, touch) is
//! Phase 3 (KE-0304); this type is intentionally the smallest thing that lets a
//! game read the keyboard and mouse without depending on `winit`.
//!
//! # Backend independence
//!
//! [`Key`] and [`MouseButton`] are engine-owned enums, not `winit` re-exports,
//! so game code never depends on the windowing backend. The windowed entry in
//! [`app`](crate::app) translates `winit` events into these types; the headless
//! driver drives them directly. The set of [`Key`] variants is intentionally
//! partial (a useful subset for the first title) and `#[non_exhaustive]` so it
//! can grow without breaking games.

use std::collections::HashSet;

/// A keyboard key, identified by physical position (backend-agnostic).
///
/// This is a curated subset — enough for the Phase-1 game and camera controls —
/// not the full keyboard. It is [`#[non_exhaustive]`] so future keys can be added
/// without a breaking change; downstream `match` expressions must include a
/// wildcard arm.
///
/// [`#[non_exhaustive]`]: https://doc.rust-lang.org/reference/attributes/type_system.html#the-non_exhaustive-attribute
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Key {
    /// The `W` key.
    W,
    /// The `A` key.
    A,
    /// The `S` key.
    S,
    /// The `D` key.
    D,
    /// The `Q` key.
    Q,
    /// The `E` key.
    E,
    /// The left arrow key.
    Left,
    /// The right arrow key.
    Right,
    /// The up arrow key.
    Up,
    /// The down arrow key.
    Down,
    /// The space bar.
    Space,
    /// The escape key.
    Escape,
}

/// A mouse button (backend-agnostic).
///
/// [`#[non_exhaustive]`] so additional buttons can be added later.
///
/// [`#[non_exhaustive]`]: https://doc.rust-lang.org/reference/attributes/type_system.html#the-non_exhaustive-attribute
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MouseButton {
    /// The left mouse button.
    Left,
    /// The right mouse button.
    Right,
    /// The middle mouse button.
    Middle,
}

/// A read-only snapshot of keyboard and mouse state for the current frame.
///
/// The engine owns one `InputState`, updates it from platform events (windowed
/// entry) or programmatically (headless driver), and hands the game a shared
/// reference through [`EngineCtx::input`](crate::EngineCtx::input). Games read it
/// during [`update`](crate::Game::update); they never mutate it (the engine owns
/// the source of truth), which is why the accessor returns `&InputState`.
///
/// # State model (invariants)
///
/// - A key/button is "down" from the frame its press is recorded until the frame
///   its release is recorded (*level* state).
/// - **Edge detection:** [`is_key_just_pressed`](Self::is_key_just_pressed) is true
///   only on the fixed-update step where a key transitions up→down. The driver
///   calls [`advance_frame`](Self::advance_frame) after each `Game::update` to
///   snapshot the state, so a held key fires "just pressed" exactly once. (The
///   fuller input abstraction — remapping, touch/tilt — remains KE-0304.)
/// - [`cursor_position`](Self::cursor_position) is in window logical pixels,
///   origin top-left. It is `None` until the first cursor movement is seen (e.g.
///   in the pure-headless driver, where no cursor exists).
///
/// # Example
///
/// ```rust
/// use kaman_core::input::{InputState, Key};
///
/// let mut input = InputState::new();
/// input.press_key(Key::W);
/// assert!(input.is_key_down(Key::W));
/// input.release_key(Key::W);
/// assert!(!input.is_key_down(Key::W));
/// ```
#[derive(Debug, Clone, Default)]
pub struct InputState {
    pressed_keys: HashSet<Key>,
    /// Keys that were down at the previous `advance_frame` — the baseline for
    /// just-pressed edge detection.
    prev_pressed_keys: HashSet<Key>,
    pressed_buttons: HashSet<MouseButton>,
    cursor_position: Option<(f64, f64)>,
}

impl InputState {
    /// Create an empty snapshot: no keys or buttons down, cursor position unknown.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `key` is currently held down.
    #[must_use]
    pub fn is_key_down(&self, key: Key) -> bool {
        self.pressed_keys.contains(&key)
    }

    /// Whether `key` transitioned up→down since the last fixed update — a
    /// per-step edge, true exactly once per physical press (a held key does not
    /// keep returning `true`). Use this for one-shot actions (a discrete step, a
    /// menu selection) rather than continuous held-key movement.
    #[must_use]
    pub fn is_key_just_pressed(&self, key: Key) -> bool {
        self.pressed_keys.contains(&key) && !self.prev_pressed_keys.contains(&key)
    }

    /// Whether `key` transitioned down→up since the last fixed update — the release
    /// edge, true exactly once per release.
    #[must_use]
    pub fn is_key_just_released(&self, key: Key) -> bool {
        self.prev_pressed_keys.contains(&key) && !self.pressed_keys.contains(&key)
    }

    /// Whether mouse `button` is currently held down.
    #[must_use]
    pub fn is_mouse_down(&self, button: MouseButton) -> bool {
        self.pressed_buttons.contains(&button)
    }

    /// The cursor position in window logical pixels (top-left origin), or `None`
    /// if no cursor movement has been observed yet.
    #[must_use]
    pub fn cursor_position(&self) -> Option<(f64, f64)> {
        self.cursor_position
    }

    /// Record that `key` was pressed. Idempotent while held.
    ///
    /// Called by the engine (windowed entry or headless driver), not by games.
    pub fn press_key(&mut self, key: Key) {
        self.pressed_keys.insert(key);
    }

    /// Record that `key` was released. Idempotent when already up.
    ///
    /// Called by the engine, not by games.
    pub fn release_key(&mut self, key: Key) {
        self.pressed_keys.remove(&key);
    }

    /// Record that mouse `button` was pressed. Called by the engine.
    pub fn press_mouse(&mut self, button: MouseButton) {
        self.pressed_buttons.insert(button);
    }

    /// Record that mouse `button` was released. Called by the engine.
    pub fn release_mouse(&mut self, button: MouseButton) {
        self.pressed_buttons.remove(&button);
    }

    /// Set the cursor position (window logical pixels, top-left origin). Called
    /// by the engine.
    pub fn set_cursor_position(&mut self, x: f64, y: f64) {
        self.cursor_position = Some((x, y));
    }

    /// Snapshot the current pressed keys as the baseline for the next step's
    /// just-pressed edges. The driver calls this after each `Game::update`, so a
    /// held key reports [`is_key_just_pressed`](Self::is_key_just_pressed) exactly
    /// once. Called by the engine, not by games.
    pub fn advance_frame(&mut self) {
        self.prev_pressed_keys.clone_from(&self.pressed_keys);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_press_and_release_toggles_down_state() {
        let mut input = InputState::new();
        assert!(!input.is_key_down(Key::A));
        input.press_key(Key::A);
        assert!(input.is_key_down(Key::A));
        // Pressing again is idempotent.
        input.press_key(Key::A);
        assert!(input.is_key_down(Key::A));
        input.release_key(Key::A);
        assert!(!input.is_key_down(Key::A));
        // Releasing when already up is idempotent.
        input.release_key(Key::A);
        assert!(!input.is_key_down(Key::A));
    }

    #[test]
    fn mouse_button_state_tracks_independently() {
        let mut input = InputState::new();
        input.press_mouse(MouseButton::Left);
        assert!(input.is_mouse_down(MouseButton::Left));
        assert!(!input.is_mouse_down(MouseButton::Right));
        input.release_mouse(MouseButton::Left);
        assert!(!input.is_mouse_down(MouseButton::Left));
    }

    #[test]
    fn cursor_position_starts_none_then_tracks() {
        let mut input = InputState::new();
        assert_eq!(input.cursor_position(), None);
        input.set_cursor_position(12.0, 34.0);
        assert_eq!(input.cursor_position(), Some((12.0, 34.0)));
    }

    #[test]
    fn just_pressed_is_a_single_edge_per_press() {
        let mut input = InputState::new();
        // A press is an edge until the next advance_frame snapshots it.
        input.press_key(Key::Left);
        assert!(input.is_key_just_pressed(Key::Left));
        assert!(input.is_key_just_pressed(Key::Left), "still the same step");
        input.advance_frame();
        assert!(!input.is_key_just_pressed(Key::Left), "held key: no repeat edge");
        assert!(input.is_key_down(Key::Left), "but still held down");
        // Releasing then pressing again is a fresh edge.
        input.release_key(Key::Left);
        input.advance_frame();
        assert!(!input.is_key_just_pressed(Key::Left));
        input.press_key(Key::Left);
        assert!(input.is_key_just_pressed(Key::Left), "re-press is a new edge");
    }
}
