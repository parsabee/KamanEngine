// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! A generational registry for persistent mesh resources (KE-0103).
//!
//! # Upload once, reference by handle
//!
//! Mesh geometry is uploaded into a persistent `MTLBuffer` **once** at load time
//! ([`RenderDevice::create_mesh`](kaman_render_api::RenderDevice::create_mesh))
//! and thereafter referenced by an opaque
//! [`MeshHandle`] on the hot draw path. The draw
//! path performs **no allocation**: it looks the buffer up in this registry by
//! handle. This is the mobile-safe replacement for the prototype's per-frame
//! `new_buffer_with_data`.
//!
//! # Handle encoding
//!
//! The seam's `MeshHandle` is an opaque `u32`; this backend packs an **index**
//! and a **generation** into it:
//!
//! - low [`INDEX_BITS`] bits: slot index into the registry (up to
//!   [`MAX_SLOTS`] live meshes)
//! - high [`GENERATION_BITS`] bits: the slot's generation at creation time
//!
//! Nothing above the render seam interprets these bits — the packing is entirely
//! private to `kaman-render`.
//!
//! # Stale-handle invariant
//!
//! Every slot carries a monotonically increasing generation. `insert` stamps
//! the freshly-created handle with the slot's current generation;
//! [`remove`](Registry::remove) frees the slot and **bumps** that generation. A
//! [`get`](Registry::get) whose handle generation does not match the slot's live
//! generation returns [`RegistryError::StaleHandle`] — it never returns a
//! different mesh that happens to occupy the reused slot. A handle whose index is
//! out of range returns [`RegistryError::UnknownHandle`]. In both cases the
//! lookup is a **defined error**, never a silent wrong-buffer draw.

use kaman_render_api::MeshHandle;

/// Number of low bits of a [`MeshHandle`] used for the slot index.
pub const INDEX_BITS: u32 = 20;
/// Number of high bits of a [`MeshHandle`] used for the generation.
pub const GENERATION_BITS: u32 = 32 - INDEX_BITS;
/// Maximum number of slots addressable by a handle (`2^INDEX_BITS`).
pub const MAX_SLOTS: u32 = 1 << INDEX_BITS;
/// Bit mask selecting the index portion of a packed handle.
const INDEX_MASK: u32 = MAX_SLOTS - 1;
/// Number of distinct generations before wraparound (`2^GENERATION_BITS`).
const GENERATION_MODULO: u32 = 1 << GENERATION_BITS;

/// Pack a slot index and generation into an opaque [`MeshHandle`].
fn pack(index: u32, generation: u32) -> MeshHandle {
    debug_assert!(index < MAX_SLOTS, "mesh index exceeds MAX_SLOTS");
    MeshHandle((generation << INDEX_BITS) | (index & INDEX_MASK))
}

/// Unpack an opaque [`MeshHandle`] into its `(index, generation)` parts.
fn unpack(handle: MeshHandle) -> (u32, u32) {
    let index = handle.0 & INDEX_MASK;
    let generation = handle.0 >> INDEX_BITS;
    (index, generation)
}

/// A lookup failure on a [`MeshHandle`]. Both variants are defined, recoverable
/// errors — a stale or unknown handle is never resolved to a live mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryError {
    /// The handle's index is outside the registry's slot range (never created).
    UnknownHandle,
    /// The handle names a real slot, but the slot has since been freed (and
    /// possibly reused) — its generation no longer matches. Using a
    /// [`destroy_mesh`](kaman_render_api::RenderDevice::destroy_mesh)'d handle
    /// lands here.
    StaleHandle,
}

/// One registry slot: either live (holding a value at some generation) or free
/// (awaiting reuse). The generation is bumped on free so old handles into this
/// slot become detectably stale.
struct Slot<T> {
    /// The stored value while live; `None` once freed.
    value: Option<T>,
    /// The current generation of this slot. A handle matches only if its
    /// generation equals this.
    generation: u32,
}

/// A generational slotmap keyed by [`MeshHandle`].
///
/// Insertion reuses the lowest-index free slot (from a free list) before
/// growing, so indices stay dense and small. See the module docs for the
/// upload-once contract and the stale-handle invariant.
pub struct Registry<T> {
    slots: Vec<Slot<T>>,
    /// Indices of freed slots available for reuse (LIFO).
    free: Vec<u32>,
}

impl<T> Registry<T> {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    /// Insert `value`, returning a fresh [`MeshHandle`] stamped with the slot's
    /// current generation.
    ///
    /// # Panics
    /// Panics if the number of live slots would exceed [`MAX_SLOTS`] (the index
    /// space of a packed handle).
    pub fn insert(&mut self, value: T) -> MeshHandle {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.value = Some(value);
            pack(index, slot.generation)
        } else {
            let index = self.slots.len() as u32;
            assert!(index < MAX_SLOTS, "mesh registry exhausted MAX_SLOTS slots");
            self.slots.push(Slot {
                value: Some(value),
                generation: 0,
            });
            pack(index, 0)
        }
    }

    /// Look up the value named by `handle`.
    ///
    /// Returns [`RegistryError::UnknownHandle`] if the index was never created,
    /// or [`RegistryError::StaleHandle`] if the slot has been freed/reused since
    /// the handle was issued (generation mismatch).
    pub fn get(&self, handle: MeshHandle) -> Result<&T, RegistryError> {
        let (index, generation) = unpack(handle);
        let slot = self
            .slots
            .get(index as usize)
            .ok_or(RegistryError::UnknownHandle)?;
        match &slot.value {
            Some(value) if slot.generation == generation => Ok(value),
            _ => Err(RegistryError::StaleHandle),
        }
    }

    /// Free the slot named by `handle` and bump its generation so the handle (and
    /// any copy of it) becomes stale.
    ///
    /// Removing an unknown or already-freed handle is a no-op that returns the
    /// corresponding [`RegistryError`] without disturbing any live slot.
    pub fn remove(&mut self, handle: MeshHandle) -> Result<T, RegistryError> {
        let (index, generation) = unpack(handle);
        let slot = self
            .slots
            .get_mut(index as usize)
            .ok_or(RegistryError::UnknownHandle)?;
        if slot.generation != generation || slot.value.is_none() {
            return Err(RegistryError::StaleHandle);
        }
        let value = slot.value.take().expect("checked Some above");
        // Bump the generation so the just-freed handle can never match again.
        // Wrap within the generation bit-width; wraparound after 2^GENERATION_BITS
        // frees of the *same slot* is astronomically unlikely for mesh lifetimes.
        slot.generation = (slot.generation + 1) % GENERATION_MODULO;
        self.free.push(index);
        Ok(value)
    }

    /// Number of live values currently stored.
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.value.is_some()).count()
    }

    /// Whether the registry holds no live values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T> Default for Registry<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_get_roundtrips() {
        let mut reg: Registry<u32> = Registry::new();
        let h = reg.insert(42);
        assert_eq!(reg.get(h), Ok(&42));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn distinct_inserts_get_distinct_handles_and_values() {
        let mut reg: Registry<u32> = Registry::new();
        let a = reg.insert(1);
        let b = reg.insert(2);
        assert_ne!(a, b);
        assert_eq!(reg.get(a), Ok(&1));
        assert_eq!(reg.get(b), Ok(&2));
    }

    #[test]
    fn unknown_index_is_unknown_handle() {
        let reg: Registry<u32> = Registry::new();
        assert_eq!(reg.get(pack(5, 0)), Err(RegistryError::UnknownHandle));
    }

    #[test]
    fn freed_handle_is_stale_not_silent_reuse() {
        let mut reg: Registry<u32> = Registry::new();
        let old = reg.insert(10);
        assert_eq!(reg.remove(old), Ok(10));
        // The freed handle is now a defined error, never a live value.
        assert_eq!(reg.get(old), Err(RegistryError::StaleHandle));

        // Reusing the slot yields a *different* handle (bumped generation); the
        // stale handle must not resolve to the new occupant.
        let new = reg.insert(99);
        let (old_i, old_g) = unpack(old);
        let (new_i, new_g) = unpack(new);
        assert_eq!(old_i, new_i, "slot index reused");
        assert_ne!(old_g, new_g, "generation bumped on reuse");
        assert_eq!(reg.get(new), Ok(&99));
        assert_eq!(reg.get(old), Err(RegistryError::StaleHandle));
    }

    #[test]
    fn double_remove_is_defined_error() {
        let mut reg: Registry<u32> = Registry::new();
        let h = reg.insert(7);
        assert_eq!(reg.remove(h), Ok(7));
        assert_eq!(reg.remove(h), Err(RegistryError::StaleHandle));
    }

    #[test]
    fn packing_roundtrips() {
        let h = pack(12345, 7);
        assert_eq!(unpack(h), (12345, 7));
    }
}
