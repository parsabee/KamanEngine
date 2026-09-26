// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Procedural geometry for the parts of the scene that aren't imported models:
//! the guardrails and the hill terrain (KE-0706).
//!
//! Both are built once (in [`Game::init`](kaman_core::Game::init)) as raw
//! `[pos,normal,color]` vertex bytes ready for
//! [`RenderDevice::create_mesh`](kaman_render_api::RenderDevice::create_mesh),
//! then streamed/positioned every frame by transform alone — no per-frame
//! allocation. [`push_box`] is the shared low-level primitive both build on.

use crate::config::{
    GROUND_Y, GUARDRAIL_H, GUARDRAIL_POST_SPACING, HILL_CREST, TERRAIN_COLUMNS, TERRAIN_FLAT_HALF,
    TERRAIN_W, TERRAIN_Z_FAR, TERRAIN_Z_NEAR,
};

/// Append an axis-aligned box (6 outward-facing quads) at `center` with the given
/// `half`-extents and a flat `color`, packed onto the `[pos,normal,color]` layout.
pub(crate) fn push_box(bytes: &mut Vec<u8>, indices: &mut Vec<u32>, center: [f32; 3], half: [f32; 3], color: [f32; 3]) {
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        ([0.0, 0.0, 1.0], [[-1.0, -1.0, 1.0], [1.0, -1.0, 1.0], [1.0, 1.0, 1.0], [-1.0, 1.0, 1.0]]),
        ([0.0, 0.0, -1.0], [[1.0, -1.0, -1.0], [-1.0, -1.0, -1.0], [-1.0, 1.0, -1.0], [1.0, 1.0, -1.0]]),
        ([0.0, 1.0, 0.0], [[-1.0, 1.0, 1.0], [1.0, 1.0, 1.0], [1.0, 1.0, -1.0], [-1.0, 1.0, -1.0]]),
        ([0.0, -1.0, 0.0], [[-1.0, -1.0, -1.0], [1.0, -1.0, -1.0], [1.0, -1.0, 1.0], [-1.0, -1.0, 1.0]]),
        ([1.0, 0.0, 0.0], [[1.0, -1.0, 1.0], [1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 1.0, 1.0]]),
        ([-1.0, 0.0, 0.0], [[-1.0, -1.0, -1.0], [-1.0, -1.0, 1.0], [-1.0, 1.0, 1.0], [-1.0, 1.0, -1.0]]),
    ];
    for (n, corners) in faces {
        let base = (bytes.len() / 36) as u32;
        for c in corners {
            let pos = [center[0] + c[0] * half[0], center[1] + c[1] * half[1], center[2] + c[2] * half[2]];
            for f in pos {
                bytes.extend_from_slice(&f.to_ne_bytes());
            }
            for f in n {
                bytes.extend_from_slice(&f.to_ne_bytes());
            }
            for f in color {
                bytes.extend_from_slice(&f.to_ne_bytes());
            }
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// Build one guardrail segment `length` units long (along `Z`), local origin at its
/// base center so a spawn transform drops it onto the road deck: two horizontal
/// metal rails plus evenly-spaced vertical posts. Packed on the `[pos,normal,color]`
/// layout (untextured pipeline).
pub(crate) fn guardrail_geometry(length: f32) -> (Vec<u8>, Vec<u32>) {
    let rail_color = [0.62, 0.63, 0.66];
    let post_color = [0.40, 0.41, 0.44];
    let half_len = length / 2.0;

    let mut bytes = Vec::new();
    let mut indices = Vec::new();

    // Two horizontal rails running the length of the segment (thin in X, at the
    // road edge; the segment is placed at ±GUARDRAIL_X).
    push_box(&mut bytes, &mut indices, [0.0, GUARDRAIL_H, 0.0], [0.05, 0.07, half_len], rail_color);
    push_box(&mut bytes, &mut indices, [0.0, GUARDRAIL_H * 0.6, 0.0], [0.05, 0.055, half_len], rail_color);

    // Vertical posts, evenly spaced along the segment (endpoints included).
    let posts = ((length / GUARDRAIL_POST_SPACING).round() as i32).max(1);
    for i in 0..=posts {
        let z = -half_len + (i as f32 / posts as f32) * length;
        push_box(&mut bytes, &mut indices, [0.0, GUARDRAIL_H * 0.5, z], [0.06, GUARDRAIL_H * 0.5, 0.06], post_color);
    }

    (bytes, indices)
}

/// Ground height at lateral position `x`: a flat valley floor at [`GROUND_Y`] out to
/// [`TERRAIN_FLAT_HALF`] (so the road platform and the building rows sit on level
/// ground), then rising into rolling hills that crest near [`HILL_CREST`] at the
/// sheet's edges.
pub(crate) fn terrain_height(x: f32) -> f32 {
    let ax = x.abs();
    let span = (TERRAIN_W * 0.5 - TERRAIN_FLAT_HALF).max(f32::EPSILON);
    let t = ((ax - TERRAIN_FLAT_HALF).max(0.0) / span).clamp(0.0, 1.0);
    // Ease in so the ground leaves the valley floor gently, then climbs.
    let climb = t.powf(1.4) * (HILL_CREST - GROUND_Y);
    // Rolling variation, faded in with the climb so the valley floor stays level.
    let roll = 2.2 * ((x * 0.11).sin() * 0.6 + (x * 0.29 + 1.3).sin() * 0.4);
    GROUND_Y + climb + t * roll
}

/// Build the ground terrain sheet: one quad strip spanning
/// [`TERRAIN_Z_NEAR`]..[`TERRAIN_Z_FAR`] in relative `Z`, with each column's height
/// from [`terrain_height`]. Colored green, lighter toward the hilltops. Packed on
/// the `[pos,normal,color]` layout (untextured pipeline); drawn camera-locked.
pub(crate) fn terrain_geometry() -> (Vec<u8>, Vec<u32>) {
    let valley_color = [0.20, 0.28, 0.17];
    let hill_color = [0.36, 0.42, 0.33];

    let mut bytes: Vec<u8> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let push_vertex = |bytes: &mut Vec<u8>, pos: [f32; 3], color: [f32; 3]| {
        for f in pos {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
        for f in [0.0f32, 1.0, 0.0] {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
        for f in color {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
    };

    for i in 0..=TERRAIN_COLUMNS {
        let u = i as f32 / TERRAIN_COLUMNS as f32;
        let x = (u - 0.5) * TERRAIN_W;
        let y = terrain_height(x);
        // Blend the color with how high this column climbed.
        let t = ((y - GROUND_Y) / (HILL_CREST - GROUND_Y)).clamp(0.0, 1.0);
        let color = [
            valley_color[0] + (hill_color[0] - valley_color[0]) * t,
            valley_color[1] + (hill_color[1] - valley_color[1]) * t,
            valley_color[2] + (hill_color[2] - valley_color[2]) * t,
        ];
        push_vertex(&mut bytes, [x, y, TERRAIN_Z_NEAR], color);
        push_vertex(&mut bytes, [x, y, TERRAIN_Z_FAR], color);
    }

    for i in 0..TERRAIN_COLUMNS {
        let n = 2 * i; // near, this column
        let f = n + 1; // far, this column
        let n2 = n + 2; // near, next column
        let f2 = n + 3; // far, next column
        indices.extend_from_slice(&[n, f, f2, n, f2, n2]);
    }

    (bytes, indices)
}
