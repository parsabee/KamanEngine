// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Expanding an indexed mesh into the flat vertex stream the backend draws.
//!
//! The backend draws non-indexed triangles, so
//! [`create_mesh`](kaman_render_api::RenderDevice::create_mesh) copies one
//! vertex record per index. This module holds that expansion as a pure function,
//! kept apart from the Metal code so its edge cases can be tested on their own.

/// Expand `indices` over `vertices` (records of `stride` bytes) into one record
/// per index, triangle by triangle.
///
/// A triangle with an index past the last vertex is dropped **whole**, and so
/// is a trailing partial triangle (fewer than three indices left). Skipping a
/// single bad index instead would shift every later triangle by one vertex,
/// stitching triangles that were never in the input.
///
/// Returns an empty buffer when `stride` is zero or nothing remains.
#[must_use]
pub fn deindex(vertices: &[u8], stride: usize, indices: &[u32]) -> Vec<u8> {
    if stride == 0 {
        return Vec::new();
    }
    let vertex_count = vertices.len() / stride;
    let mut expanded = Vec::with_capacity(indices.len() / 3 * 3 * stride);
    for triangle in indices.chunks_exact(3) {
        if triangle.iter().any(|&i| i as usize >= vertex_count) {
            continue;
        }
        for &i in triangle {
            let start = i as usize * stride;
            expanded.extend_from_slice(&vertices[start..start + stride]);
        }
    }
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eight one-byte "vertices" whose byte is their own index.
    const VERTICES: [u8; 8] = [0, 1, 2, 3, 4, 5, 6, 7];

    #[test]
    fn copies_one_record_per_index() {
        let bytes = [10, 11, 20, 21, 30, 31];
        assert_eq!(deindex(&bytes, 2, &[2, 0, 1]), vec![30, 31, 10, 11, 20, 21]);
    }

    #[test]
    fn a_bad_index_drops_only_its_own_triangle() {
        // The middle triangle names vertex 99; the triangles around it survive
        // unchanged instead of shifting by one vertex.
        let indices = [0, 1, 2, 3, 4, 99, 5, 6, 7];
        assert_eq!(deindex(&VERTICES, 1, &indices), vec![0, 1, 2, 5, 6, 7]);
    }

    #[test]
    fn a_trailing_partial_triangle_is_dropped() {
        assert_eq!(deindex(&VERTICES, 1, &[0, 1, 2, 3, 4]), vec![0, 1, 2]);
    }

    #[test]
    fn a_zero_stride_or_no_whole_triangle_is_empty() {
        assert!(deindex(&VERTICES, 0, &[0, 1, 2]).is_empty());
        assert!(deindex(&VERTICES, 1, &[0, 1]).is_empty());
        assert!(deindex(&VERTICES, 1, &[]).is_empty());
    }

    #[test]
    fn a_short_final_record_is_out_of_range() {
        // Five bytes at stride 2 hold two whole vertices; index 2 would read the
        // half record past them.
        assert!(deindex(&[0, 1, 2, 3, 4], 2, &[0, 1, 2]).is_empty());
    }
}
