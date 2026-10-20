//! Native local-light static cache metadata and atlas lifetime. Pixel validity
//! is owned separately by the render submission/acknowledgement, never inferred
//! from the existence of one of these CPU allocations.
use crate::render_shadow::{AtlasAllocation, ShadowAtlasAllocator};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
pub struct LocalShadowCacheSource {
    pub position: [f32; 3],
    pub forward: [f32; 3],
    pub up: [f32; 3],
    pub crop: [f32; 4],
    pub width: u32,
}

/// 2cd190's componentwise absolute difference and2cd500's pose tolerance.
/// Unordered differences fail. Cache width may exceed the requested width.
pub fn source_matches(stored: &LocalShadowCacheSource, requested: &LocalShadowCacheSource) -> bool {
    requested.width <= stored.width
        && stored
            .position
            .iter()
            .chain(&stored.forward)
            .chain(&stored.up)
            .zip(
                requested
                    .position
                    .iter()
                    .chain(&requested.forward)
                    .chain(&requested.up),
            )
            .all(|(&a, &b)| (a - b).abs() <= f32::from_bits(0x38d1_b717))
        && stored
            .crop
            .iter()
            .zip(requested.crop)
            .all(|(&a, b)| (a - b).abs() <= f32::from_bits(0x3a83_126f))
}

pub fn cache_key(light_id: u16, side: u32) -> u32 {
    u32::from(light_id) | (side << 28)
}

/// View-definition +97/+98, copied to prepared view +3a67/+3a68.
/// 523af0 and Midnight StartEntity/StopEntity set these only after an actual
/// actor/model dynamic-caster transition; they are not camera-motion flags.
pub use crate::render_model::LocalShadowCacheControls;

#[derive(Clone, Debug)]
pub struct LocalShadowCacheEntry {
    pub source: LocalShadowCacheSource,
    pub allocation: AtlasAllocation,
    pub frame: u32,
    pub contains_models: bool,
}

pub struct LocalShadowCache {
    banks: Vec<BTreeMap<u32, LocalShadowCacheEntry>>,
    pub atlas: ShadowAtlasAllocator,
}

impl LocalShadowCache {
    /// 351a69: full invalidation takes precedence when both signals are set.
    pub fn apply_view_controls(&mut self, controls: LocalShadowCacheControls) {
        if controls.clear_all {
            self.clear();
        } else if controls.discard_models {
            self.discard_model_entries();
        }
    }
    ///32cea0 publishes this bit only for the allocation that actually received
    ///static model draws. A completed older allocation cannot mark its reuse.
    pub fn acknowledge_models(&mut self, allocations: &std::collections::HashMap<u32, bool>) {
        for entries in &mut self.banks {
            for entry in entries.values_mut() {
                if let Some(&contains_models) = allocations.get(&entry.allocation.id) {
                    entry.contains_models = contains_models;
                }
            }
        }
    }
    pub fn new(rectangles: &[Vec<[i32; 4]>]) -> Self {
        Self {
            banks: rectangles.iter().map(|_| BTreeMap::new()).collect(),
            atlas: ShadowAtlasAllocator::new_banks(
                &rectangles.iter().map(Vec::as_slice).collect::<Vec<_>>(),
                0,
            ),
        }
    }

    /// 2cd500: a matching lookup touches its original frame stamp only.
    pub fn lookup(
        &mut self,
        bank: usize,
        key: u32,
        source: &LocalShadowCacheSource,
        frame: u32,
    ) -> Option<&LocalShadowCacheEntry> {
        let entry = self.banks.get_mut(bank)?.get_mut(&key)?;
        if !source_matches(&entry.source, source) {
            return None;
        }
        entry.frame = frame;
        Some(entry)
    }

    /// 2cca20 removes/relinquishes an existing key BEFORE trying the new
    /// allocation. Failure does not retain the superseded source or pixels.
    pub fn insert(
        &mut self,
        bank: usize,
        key: u32,
        source: LocalShadowCacheSource,
        height: u32,
        frame: u32,
    ) -> Option<&mut LocalShadowCacheEntry> {
        let entries = self.banks.get_mut(bank)?;
        if let Some(old) = entries.remove(&key) {
            self.atlas.release_in(bank, old.allocation.id);
        }
        let allocation = self.atlas.allocate_in(bank, source.width, height)?;
        entries.insert(
            key,
            LocalShadowCacheEntry {
                source,
                allocation,
                frame,
                contains_models: false,
            },
        );
        entries.get_mut(&key)
    }

    /// 2cc940/2cd4c0 use a strict UNSIGNED older-than stamp comparison.
    pub fn evict_before(&mut self, bank: usize, frame: u32) {
        let Some(entries) = self.banks.get_mut(bank) else {
            return;
        };
        entries.retain(|_, entry| {
            if entry.frame < frame {
                self.atlas.release_in(bank, entry.allocation.id);
                false
            } else {
                true
            }
        });
    }

    /// 2cc860/2cd200 discard static cache entries marked by32cea0 as having
    /// model geometry. Their WORLD-only peers remain available.
    pub fn discard_model_entries(&mut self) {
        for (bank, entries) in self.banks.iter_mut().enumerate() {
            entries.retain(|_, entry| {
                if entry.contains_models {
                    self.atlas.release_in(bank, entry.allocation.id);
                    false
                } else {
                    true
                }
            });
        }
    }

    ///2cd270 releases every allocation before clearing the hash banks.
    ///The allocator's generation/counter is retained, unlike2ce970 reset.
    pub fn clear(&mut self) {
        for (bank, entries) in self.banks.iter_mut().enumerate() {
            for entry in entries.values() {
                self.atlas.release_in(bank, entry.allocation.id);
            }
            entries.clear();
        }
    }

    /// 2cd350 invalidates every side of one actual light across cache banks.
    pub fn invalidate_light(&mut self, light_id: u16, kind: i32, quality: i32) {
        let count = match kind {
            i if i < 2 => 6,
            2 => u32::from(quality == 1) + 1,
            3 => 1,
            _ => return,
        };
        for (bank, entries) in self.banks.iter_mut().enumerate() {
            for side in 0..count {
                if let Some(entry) = entries.remove(&cache_key(light_id, side)) {
                    self.atlas.release_in(bank, entry.allocation.id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actor_view_controls_preserve_world_cache_and_allocation_generations() {
        let mut cache = LocalShadowCache::new(&[vec![[0, 0, 63, 63]]]);
        let source = LocalShadowCacheSource {
            position: [0.; 3],
            forward: [1., 0., 0.],
            up: [0., 0., 1.],
            crop: [-1., -1., 1., 1.],
            width: 16,
        };
        cache.insert(0, 10, source, 16, 1).unwrap().contains_models = true;
        let world = cache.insert(0, 11, source, 16, 1).unwrap().allocation;
        let mut controls = LocalShadowCacheControls::default();
        controls.actor_caster_changed(false, true);
        cache.apply_view_controls(controls);
        assert!(cache.lookup(0, 10, &source, 2).is_some());
        controls.actor_caster_changed(true, false);
        cache.apply_view_controls(controls);
        assert!(cache.lookup(0, 10, &source, 3).is_none());
        assert_eq!(cache.lookup(0, 11, &source, 3).unwrap().allocation, world);
        controls.actor_caster_changed(true, true);
        cache.apply_view_controls(controls);
        assert!(cache.lookup(0, 11, &source, 4).is_none());
        assert_eq!(cache.insert(0, 12, source, 16, 4).unwrap().allocation.id, 2);
    }
    #[test]
    fn cache_lifetime_matches_insert_lookup_and_invalidation() {
        let mut cache = LocalShadowCache::new(&vec![vec![[0, 0, 63, 63]; 2]; 2]);
        let mut hits = 0;
        for (case, line) in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/local_shadow_cache_cases.txt"
        ))
        .lines()
        .enumerate()
        {
            let (input, expected) = line.split_once('|').unwrap();
            let mut parts = input.split_whitespace();
            let tag = parts.next().unwrap();
            let words = parts
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            let expected = expected
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            let source = |at, width| LocalShadowCacheSource {
                position: std::array::from_fn(|i| f32::from_bits(words[at + i])),
                forward: std::array::from_fn(|i| f32::from_bits(words[at + 3 + i])),
                up: std::array::from_fn(|i| f32::from_bits(words[at + 6 + i])),
                crop: std::array::from_fn(|i| f32::from_bits(words[at + 9 + i])),
                width,
            };
            let result = match tag {
                "I" => cache
                    .insert(
                        words[0] as usize,
                        cache_key(words[1] as u16, words[2]),
                        source(6, words[3]),
                        words[4],
                        words[5],
                    )
                    .is_some(),
                "L" => {
                    let hit = cache
                        .lookup(
                            words[0] as usize,
                            cache_key(words[1] as u16, words[2]),
                            &source(5, words[3]),
                            words[4],
                        )
                        .is_some();
                    hits += usize::from(hit);
                    hit
                }
                "M" => {
                    if let Some(entry) = cache.banks[words[0] as usize].get_mut(&words[1]) {
                        entry.contains_models = true;
                    }
                    false
                }
                "E" => {
                    cache.evict_before(words[0] as usize, words[1]);
                    false
                }
                "D" => {
                    cache.discard_model_entries();
                    false
                }
                "C" => {
                    cache.clear();
                    false
                }
                "X" => {
                    cache.invalidate_light(words[0] as u16, words[1] as i32, words[2] as i32);
                    false
                }
                _ => panic!("unknown native cache fixture operation {tag}"),
            };
            assert_eq!(
                u32::from(result),
                expected[0],
                "cache operation {tag} case{case}"
            );
            let mut snapshot = vec![cache.banks.iter().map(BTreeMap::len).sum::<usize>() as u32];
            for (bank, entries) in cache.banks.iter().enumerate() {
                for (&key, entry) in entries {
                    snapshot.extend([
                        bank as u32,
                        key,
                        entry.source.width,
                        entry.frame,
                        u32::from(entry.contains_models),
                        entry.allocation.id,
                        entry.allocation.slice as u32,
                    ]);
                    snapshot.extend(entry.allocation.rect.map(|v| v as u32));
                }
            }
            assert_eq!(
                snapshot,
                expected[1..],
                "cache metadata/atlas state after {tag} case{case}"
            );
        }
        assert!(
            hits > 10,
            "fixture must exercise successful cross-operation cache reuse"
        );
    }
}
