// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Generational handle tables for the C API.
//!
//! A handle id packs `kind (8 bits) | index (24 bits) | generation (32 bits)`.
//! Kinds start at 1, so no valid id is 0 (the C null handle). A slot's
//! generation goes up by one each time it is freed; a slot whose generation
//! would wrap is **retired** instead of reused, so an id is never issued twice.
//! Every C handle is validated here before the backend handle it maps to is
//! touched, so C callers never reach the backend registry's own (shorter)
//! generations.

/// The kind byte of a handle id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Kind {
    Mesh = 1,
    Texture = 2,
    Model = 3,
    Font = 4,
}

/// The largest slot index: 24 bits.
const MAX_INDEX: u32 = (1 << 24) - 1;

fn encode(kind: Kind, index: u32, generation: u32) -> u64 {
    (u64::from(kind as u8) << 56) | (u64::from(index) << 32) | u64::from(generation)
}

/// The kind byte of `id`, if it names one of the four kinds.
pub(crate) fn kind_of(id: u64) -> Option<Kind> {
    match (id >> 56) as u8 {
        1 => Some(Kind::Mesh),
        2 => Some(Kind::Texture),
        3 => Some(Kind::Model),
        4 => Some(Kind::Font),
        _ => None,
    }
}

struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// A table of live values of one kind, addressed by handle id.
pub(crate) struct Slots<T> {
    kind: Kind,
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    live: u32,
}

/// The table is full (2^24 - 1 live values, or every slot retired).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Full;

impl<T> Slots<T> {
    pub(crate) fn new(kind: Kind) -> Self {
        Self {
            kind,
            slots: Vec::new(),
            free: Vec::new(),
            live: 0,
        }
    }

    /// Store `value`, returning its new id.
    pub(crate) fn insert(&mut self, value: T) -> Result<u64, Full> {
        let index = match self.free.pop() {
            Some(index) => index,
            None => {
                let index = u32::try_from(self.slots.len()).map_err(|_| Full)?;
                if index > MAX_INDEX {
                    return Err(Full);
                }
                self.slots.push(Slot {
                    generation: 1,
                    value: None,
                });
                index
            }
        };
        let slot = &mut self.slots[index as usize];
        slot.value = Some(value);
        self.live += 1;
        Ok(encode(self.kind, index, slot.generation))
    }

    fn locate(&self, id: u64) -> Option<usize> {
        if kind_of(id) != Some(self.kind) {
            return None;
        }
        let index = ((id >> 32) & u64::from(MAX_INDEX)) as usize;
        let generation = id as u32;
        let slot = self.slots.get(index)?;
        (slot.generation == generation && slot.value.is_some()).then_some(index)
    }

    pub(crate) fn get(&self, id: u64) -> Option<&T> {
        self.locate(id).and_then(|i| self.slots[i].value.as_ref())
    }

    pub(crate) fn get_mut(&mut self, id: u64) -> Option<&mut T> {
        let i = self.locate(id)?;
        self.slots[i].value.as_mut()
    }

    /// Remove and return the value `id` names; the id is never valid again.
    pub(crate) fn remove(&mut self, id: u64) -> Option<T> {
        let i = self.locate(id)?;
        let slot = &mut self.slots[i];
        let value = slot.value.take();
        self.live -= 1;
        if slot.generation == u32::MAX {
            // Retire the slot: reusing it would wrap the generation and could
            // reissue an id that a stale handle still holds.
        } else {
            slot.generation += 1;
            self.free.push(i as u32);
        }
        value
    }

    pub(crate) fn len(&self) -> u32 {
        self.live
    }

    #[cfg(test)]
    fn force_generation(&mut self, index: usize, generation: u32) {
        self.slots[index].generation = generation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_never_zero_and_encode_their_kind() {
        let mut meshes = Slots::new(Kind::Mesh);
        let id = meshes.insert(()).unwrap();
        assert_ne!(id, 0);
        assert_eq!(kind_of(id), Some(Kind::Mesh));
        assert_eq!(kind_of(0), None);
    }

    #[test]
    fn a_removed_id_stays_invalid_even_after_its_slot_is_reused() {
        let mut t = Slots::new(Kind::Texture);
        let first = t.insert("a").unwrap();
        assert_eq!(t.remove(first), Some("a"));
        let second = t.insert("b").unwrap();
        assert_ne!(first, second, "a reused slot gets a new generation");
        assert_eq!(t.get(first), None);
        assert_eq!(t.get(second), Some(&"b"));
        assert_eq!(t.remove(first), None, "double remove is a no-op");
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn ids_of_another_kind_are_rejected() {
        let mut meshes = Slots::new(Kind::Mesh);
        let mut textures = Slots::new(Kind::Texture);
        let mesh = meshes.insert(1).unwrap();
        let texture = textures.insert(2).unwrap();
        // Same slot index and generation, different kind byte.
        assert_eq!(
            mesh & 0x00FF_FFFF_FFFF_FFFF,
            texture & 0x00FF_FFFF_FFFF_FFFF
        );
        assert_eq!(meshes.get(texture), None);
        assert_eq!(textures.get(mesh), None);
    }

    #[test]
    fn a_slot_whose_generation_would_wrap_is_retired() {
        let mut t = Slots::new(Kind::Font);
        let id = t.insert(()).unwrap();
        t.force_generation(0, u32::MAX);
        let last = encode(Kind::Font, 0, u32::MAX);
        assert_eq!(
            t.get(id),
            None,
            "the forced generation invalidated the old id"
        );
        assert!(t.get(last).is_some());
        t.remove(last);
        let next = t.insert(()).unwrap();
        assert_eq!((next >> 32) & u64::from(MAX_INDEX), 1, "slot 0 was retired");
        assert_eq!(t.get(last), None);
    }
}
