// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Generational handle tables for the C API.
//!
//! A handle id packs, from the top bit down,
//! `kind (4 bits) | context (12 bits) | index (24 bits) | generation (24 bits)`.
//! Kinds start at 1, so no valid id is 0 (the C null handle). Every live
//! context holds a different context tag ([`ContextTag`]), so a context never
//! accepts a handle another live context issued. A slot's generation goes up
//! by one each time it is freed; a slot whose generation would wrap is
//! **retired** instead of reused, so an id is never issued twice. Every C
//! handle is validated here before the backend handle it maps to is touched, so
//! C callers never reach the backend registry's own (shorter) generations.

use std::sync::Mutex;

/// The kind of a handle id, in its top 4 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Kind {
    Mesh = 1,
    Texture = 2,
    Model = 3,
    Font = 4,
}

const KIND_SHIFT: u32 = 60;
const CONTEXT_SHIFT: u32 = 48;
const INDEX_SHIFT: u32 = 24;

/// The largest slot index: 24 bits.
const MAX_INDEX: u32 = (1 << 24) - 1;
/// The largest generation: 24 bits. A slot that reaches it is retired once freed.
const MAX_GENERATION: u32 = (1 << 24) - 1;
/// The largest context tag: 12 bits. Tags start at 1, so at most this many
/// contexts can be alive at once.
pub(crate) const MAX_CONTEXTS: u16 = (1 << 12) - 1;

fn encode(kind: Kind, context: u16, index: u32, generation: u32) -> u64 {
    (u64::from(kind as u8) << KIND_SHIFT)
        | (u64::from(context) << CONTEXT_SHIFT)
        | (u64::from(index) << INDEX_SHIFT)
        | u64::from(generation)
}

/// The kind of `id`, if it names one of the four kinds.
pub(crate) fn kind_of(id: u64) -> Option<Kind> {
    match (id >> KIND_SHIFT) as u8 {
        1 => Some(Kind::Mesh),
        2 => Some(Kind::Texture),
        3 => Some(Kind::Model),
        4 => Some(Kind::Font),
        _ => None,
    }
}

/// Which context tags are taken, and where the next search starts.
struct TagPool {
    /// Bit `t` is set while tag `t` is held. Tag 0 is never issued.
    taken: [u64; 64],
    /// The tag after the last one issued. Tags are handed out round-robin, so
    /// a destroyed context's tag is reused only after every other tag has been,
    /// and a stale handle from it is very unlikely to resolve in its successor.
    next: u16,
}

impl TagPool {
    const fn new() -> Self {
        Self {
            taken: [0; 64],
            next: 1,
        }
    }

    /// Take the next free tag in `1..=MAX_CONTEXTS`, or `None` if all are taken.
    fn take(&mut self) -> Option<u16> {
        for step in 0..MAX_CONTEXTS {
            let tag = (self.next - 1 + step) % MAX_CONTEXTS + 1;
            let (word, bit) = (usize::from(tag / 64), tag % 64);
            if self.taken[word] & (1 << bit) == 0 {
                self.taken[word] |= 1 << bit;
                self.next = tag % MAX_CONTEXTS + 1;
                return Some(tag);
            }
        }
        None
    }

    fn give_back(&mut self, tag: u16) {
        let (word, bit) = (usize::from(tag / 64), tag % 64);
        self.taken[word] &= !(1 << bit);
    }
}

/// The tags of every live context in the process.
static TAGS: Mutex<TagPool> = Mutex::new(TagPool::new());

/// A context's tag: unique among live contexts, and given back when dropped.
pub(crate) struct ContextTag(u16);

impl ContextTag {
    /// Take the next free tag, or `None` if [`MAX_CONTEXTS`] contexts are live.
    pub(crate) fn acquire() -> Option<Self> {
        // The pool is plain bits, valid even if a holder panicked.
        TAGS.lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .map(Self)
    }

    pub(crate) fn value(&self) -> u16 {
        self.0
    }
}

impl Drop for ContextTag {
    fn drop(&mut self) {
        TAGS.lock()
            .unwrap_or_else(|e| e.into_inner())
            .give_back(self.0);
    }
}

struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// A table of live values of one kind in one context, addressed by handle id.
pub(crate) struct Slots<T> {
    kind: Kind,
    /// The owning context's tag (see [`ContextTag`]).
    context: u16,
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    live: u32,
}

/// The table is full (2^24 - 1 live values, or every slot retired).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Full;

impl<T> Slots<T> {
    pub(crate) fn new(kind: Kind, context: &ContextTag) -> Self {
        Self {
            kind,
            context: context.value(),
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
        Ok(encode(self.kind, self.context, index, slot.generation))
    }

    fn locate(&self, id: u64) -> Option<usize> {
        if kind_of(id) != Some(self.kind) {
            return None;
        }
        if (id >> CONTEXT_SHIFT) as u16 & MAX_CONTEXTS != self.context {
            return None; // another context's handle
        }
        let index = ((id >> INDEX_SHIFT) & u64::from(MAX_INDEX)) as usize;
        let generation = (id as u32) & MAX_GENERATION;
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
        if slot.generation == MAX_GENERATION {
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

    /// A tag from the process-wide pool, like a real context holds.
    fn tag() -> ContextTag {
        ContextTag::acquire().expect("a free context tag")
    }

    fn index_of(id: u64) -> u64 {
        (id >> INDEX_SHIFT) & u64::from(MAX_INDEX)
    }

    #[test]
    fn ids_are_never_zero_and_encode_their_kind() {
        let ctx = tag();
        let mut meshes = Slots::new(Kind::Mesh, &ctx);
        let id = meshes.insert(()).unwrap();
        assert_ne!(id, 0);
        assert_eq!(kind_of(id), Some(Kind::Mesh));
        assert_eq!(kind_of(0), None);
    }

    #[test]
    fn a_removed_id_stays_invalid_even_after_its_slot_is_reused() {
        let ctx = tag();
        let mut t = Slots::new(Kind::Texture, &ctx);
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
        let ctx = tag();
        let mut meshes = Slots::new(Kind::Mesh, &ctx);
        let mut textures = Slots::new(Kind::Texture, &ctx);
        let mesh = meshes.insert(1).unwrap();
        let texture = textures.insert(2).unwrap();
        // Same context, slot index and generation; only the kind differs.
        let below_kind = (1u64 << KIND_SHIFT) - 1;
        assert_eq!(mesh & below_kind, texture & below_kind);
        assert_eq!(meshes.get(texture), None);
        assert_eq!(textures.get(mesh), None);
    }

    #[test]
    fn ids_of_another_context_are_rejected() {
        let (a, b) = (tag(), tag());
        assert_ne!(a.value(), b.value(), "live contexts hold different tags");
        let mut in_a = Slots::new(Kind::Mesh, &a);
        let mut in_b = Slots::new(Kind::Mesh, &b);
        let from_a = in_a.insert("a").unwrap();
        let from_b = in_b.insert("b").unwrap();
        // Same kind, slot index and generation; only the context differs.
        let context_bits = u64::from(MAX_CONTEXTS) << CONTEXT_SHIFT;
        assert_eq!(from_a & !context_bits, from_b & !context_bits);
        assert_eq!(in_b.get(from_a), None);
        assert_eq!(in_a.get(from_b), None);
        assert_eq!(in_b.remove(from_a), None, "and can't destroy it");
        assert_eq!(in_a.get(from_a), Some(&"a"));
    }

    #[test]
    fn a_slot_whose_generation_would_wrap_is_retired() {
        let ctx = tag();
        let mut t = Slots::new(Kind::Font, &ctx);
        let id = t.insert(()).unwrap();
        t.force_generation(0, MAX_GENERATION);
        let last = encode(Kind::Font, ctx.value(), 0, MAX_GENERATION);
        assert_eq!(
            t.get(id),
            None,
            "the forced generation invalidated the old id"
        );
        assert!(t.get(last).is_some());
        t.remove(last);
        let next = t.insert(()).unwrap();
        assert_eq!(index_of(next), 1, "slot 0 was retired");
        assert_eq!(t.get(last), None);
    }

    #[test]
    fn the_largest_fields_encode_without_spilling_into_their_neighbours() {
        let id = encode(Kind::Font, MAX_CONTEXTS, MAX_INDEX, MAX_GENERATION);
        assert_eq!(kind_of(id), Some(Kind::Font));
        assert_eq!((id >> CONTEXT_SHIFT) as u16 & MAX_CONTEXTS, MAX_CONTEXTS);
        assert_eq!(index_of(id), u64::from(MAX_INDEX));
        assert_eq!(id as u32 & MAX_GENERATION, MAX_GENERATION);
    }

    #[test]
    fn tags_are_unique_until_the_pool_is_exhausted() {
        // A private pool, so this can't starve the real one other tests use.
        let mut pool = TagPool::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..MAX_CONTEXTS {
            let tag = pool.take().expect("a free tag");
            assert!((1..=MAX_CONTEXTS).contains(&tag), "tag {tag} in range");
            assert!(seen.insert(tag), "tag {tag} issued twice");
        }
        assert_eq!(pool.take(), None, "every tag is taken");
        pool.give_back(7);
        assert_eq!(pool.take(), Some(7), "a returned tag is reusable");
    }

    #[test]
    fn a_returned_tag_is_reused_only_after_the_others() {
        let mut pool = TagPool::new();
        let first = pool.take().unwrap();
        pool.give_back(first);
        let second = pool.take().unwrap();
        assert_ne!(first, second, "round-robin, not most-recently freed");
    }
}
