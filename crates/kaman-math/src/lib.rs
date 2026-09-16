// Copyright (c) 2025 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Shared math layer for KamanEngine.
//!
//! This crate is the zero-coupling leaf that every other engine crate
//! (`kaman-ecs`, `kaman-physics`, `kaman-camera`, `kaman-scene`, …) builds on
//! for spatial math. It is a thin layer over [`glam`]: the glam crate is
//! re-exported here so downstream crates depend on a single, version-pinned
//! copy, and the engine-specific [`Transform`] type layers a TRS
//! (Translation-Rotation-Scale) convenience API on top of it.
//!
//! # Public surface
//!
//! - [`Transform`] — position + rotation + scale, with point/vector transforms
//!   and 4×4 matrix conversion (see the [`transform`] module).
//! - [`glam`] — re-exported so callers can write `kaman_math::glam::Vec3`
//!   without adding their own `glam` dependency.
//!
//! # Example
//!
//! ```rust
//! use kaman_math::Transform;
//! use kaman_math::glam::Vec3;
//!
//! // Place a transform 5 units along +X and map a local point into world space.
//! let transform = Transform::from_position(Vec3::new(5.0, 0.0, 0.0));
//! let world = transform.transform_point(Vec3::new(1.0, 0.0, 0.0));
//! assert_eq!(world, Vec3::new(6.0, 0.0, 0.0));
//! ```

#![deny(missing_docs)]

pub mod transform;

/// Re-export of the [`glam`] linear-algebra crate.
///
/// Downstream crates should reach glam types through this path (e.g.
/// `kaman_math::glam::Vec3`) so the whole workspace shares one pinned version.
pub use glam;

pub use transform::Transform;
