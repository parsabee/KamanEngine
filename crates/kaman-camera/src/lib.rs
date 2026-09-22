// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Camera state and controller math for KamanEngine.
//!
//! This crate owns the engine's camera model: a target-based perspective
//! [`Camera`] (view / projection / view-projection matrices, aspect update,
//! position / target setters, basis vectors) and the controllers that drive it.
//! It is engine-generic — it depends only on [`kaman_math`] and names no game
//! type or render backend — so any game or engine layer can compute a
//! view-projection matrix and hand it across the render seam.
//!
//! # Contents
//!
//! - [`Camera`] — the perspective look-at camera (migrated from the prototype,
//!   trimmed to the seam-facing core). See the [`camera`] module.
//! - [`ChaseController`] — a follow camera that trails a target from behind and
//!   above, with optional smoothing. See the [`chase`] module.
//!
//! # Example
//!
//! ```rust
//! use kaman_camera::{Camera, ChaseController};
//! use kaman_math::glam::Vec3;
//!
//! let mut camera = Camera::new(16.0 / 9.0);
//! let chase = ChaseController::new(8.0, 4.0);
//!
//! // Each frame: point the chase camera at the moving target.
//! chase.follow(&mut camera, Vec3::new(0.0, 0.0, -50.0), Vec3::new(0.0, 0.0, -1.0));
//! let view_projection = camera.view_projection_matrix();
//! assert!(view_projection.is_finite());
//! ```

#![deny(missing_docs)]

pub mod camera;
pub mod chase;

pub use camera::Camera;
pub use chase::ChaseController;
