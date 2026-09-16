//! Backend-agnostic render seam for KamanEngine: RenderDevice / FrameRecorder traits.
//!
//! No crate above this seam may import `metal` types — the seam is the blast-radius firewall
//! that contains a renderer rewrite. Stub crate — the trait skeleton lands in KE-0006.
#![deny(missing_docs)]
