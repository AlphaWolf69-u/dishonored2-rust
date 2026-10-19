//! Host memoization of immutable, fully specified caster queries. Raw float
//! words are compared; no camera-motion threshold or elapsed-time assumption.
use crate::tome::shadow_caster::{
    NativeCasterDirection, NativeGlobalShadowCasterHit, NativeGlobalShadowCasterInput,
    NativeReceiverDepthProjection, NativeShadowObjectDistanceQuery, VerifiedShadowReceiverState,
};
use std::sync::Arc;

pub(super) type CasterResults = Arc<Vec<Vec<NativeGlobalShadowCasterHit>>>;

#[derive(PartialEq, Eq)]
struct ReceiverKey {
    bounds: [[u32; 3]; 2],
    planes: Vec<[u32; 4]>,
    active: Vec<usize>,
}
impl ReceiverKey {
    fn new(input: VerifiedShadowReceiverState<'_>) -> Self {
        let VerifiedShadowReceiverState {
            bounds,
            planes,
            active_planes,
        } = input;
        Self {
            bounds: bounds.map(|row| row.map(f32::to_bits)),
            planes: planes.iter().map(|row| row.map(f32::to_bits)).collect(),
            active: active_planes.to_vec(),
        }
    }
}
#[derive(PartialEq, Eq)]
struct DepthKey {
    rows: [[u32; 4]; 4],
    transposed: [[u32; 4]; 4],
    mask: Box<[u16; 128 * 128]>,
}
impl DepthKey {
    fn new(input: &NativeReceiverDepthProjection) -> Self {
        let NativeReceiverDepthProjection {
            receiver_rows,
            transposed_rows,
            mask,
        } = input;
        Self {
            rows: receiver_rows.map(|row| row.map(f32::to_bits)),
            transposed: transposed_rows.map(|row| row.map(f32::to_bits)),
            mask: mask.clone(),
        }
    }
}
#[derive(PartialEq, Eq)]
pub(super) struct CasterQueryKey {
    source_address: usize,
    source_length: usize,
    source_versions: (u32, u32),
    parsed_tome: usize,
    primary: ReceiverKey,
    cascades: Vec<ReceiverKey>,
    distance: ([u32; 3], [u32; 4]),
    direction: NativeCasterDirection,
    depth: Option<DepthKey>,
    job: u32,
    jobs: u32,
    physical_cascades: Vec<usize>,
}
impl CasterQueryKey {
    /// Snapshot common inputs with job=0; results contain all aligned jobs.
    /// Immutable source/parsed-Tome owners are identified by address and their
    /// Bevy resource change versions, so an in-place source edit invalidates.
    pub(super) fn new(
        input: NativeGlobalShadowCasterInput<'_>,
        source_versions: (u32, u32),
        parsed_tome: usize,
        physical: &[usize],
    ) -> Self {
        let NativeGlobalShadowCasterInput {
            source,
            primary,
            cascades,
            distance,
            direction,
            depth,
            job,
            jobs,
        } = input;
        let NativeShadowObjectDistanceQuery {
            point,
            scale_squared,
        } = distance;
        Self {
            source_address: source.as_ptr() as usize,
            source_length: source.len(),
            source_versions,
            parsed_tome,
            primary: ReceiverKey::new(primary),
            cascades: cascades.iter().copied().map(ReceiverKey::new).collect(),
            distance: (point.map(f32::to_bits), scale_squared.map(f32::to_bits)),
            direction,
            depth: depth.map(DepthKey::new),
            job,
            jobs,
            physical_cascades: physical.to_vec(),
        }
    }
}
#[derive(Default)]
pub(super) struct CasterQueryCache {
    entry: Option<(CasterQueryKey, CasterResults)>,
    pub(super) hits: u64,
    pub(super) misses: u64,
}
impl CasterQueryCache {
    pub(super) fn get(&mut self, key: &CasterQueryKey) -> Option<CasterResults> {
        if let Some((old, value)) = &self.entry {
            if old == key {
                self.hits += 1;
                return Some(value.clone());
            }
        }
        self.misses += 1;
        None
    }
    // Only successfully gathered/published lists reach this boundary.
    pub(super) fn store(&mut self, key: CasterQueryKey, value: CasterResults) {
        self.entry = Some((key, value));
    }
}

#[derive(Default)]
pub(super) struct ProjectionMaskCache {
    entry: Option<([[u32; 4]; 4], u32, Arc<Vec<u8>>)>,
    pub(super) hits: u64,
    pub(super) misses: u64,
}
impl ProjectionMaskCache {
    pub(super) fn query(
        &mut self,
        rows: [[f32; 4]; 4],
        margin: f32,
        verify: bool,
        compute: impl FnOnce() -> Vec<u8>,
    ) -> Arc<Vec<u8>> {
        let key = rows.map(|row| row.map(f32::to_bits));
        if let Some((old, old_margin, value)) = &self.entry {
            if old == &key && *old_margin == margin.to_bits() {
                self.hits += 1;
                if verify {
                    assert_eq!(
                        value.as_slice(),
                        compute(),
                        "cached world projection mask differs from original query"
                    );
                }
                return value.clone();
            }
        }
        self.misses += 1;
        let value = Arc::new(compute());
        self.entry = Some((key, margin.to_bits(), value.clone()));
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn caster_key_captures_depth_versions_planes_partitions_and_raw_float_words() {
        let source = [0u8; 32];
        let planes = [[1.0, 0.0, 0.0, f32::from_bits(0x7fc01234)]];
        let primary = VerifiedShadowReceiverState {
            bounds: [[0.0; 3]; 2],
            planes: &planes,
            active_planes: &[0],
        };
        let cascades = [primary];
        let mut depth = NativeReceiverDepthProjection {
            receiver_rows: [[0.0; 4]; 4],
            transposed_rows: [[0.0; 4]; 4],
            mask: Box::new([0; 128 * 128]),
        };
        let input = NativeGlobalShadowCasterInput {
            source: &source,
            primary,
            cascades: &cascades,
            distance: NativeShadowObjectDistanceQuery {
                point: [0.0; 3],
                scale_squared: [1.0; 4],
            },
            direction: NativeCasterDirection {
                face: 1,
                x: 2,
                y: 3,
            },
            depth: None,
            job: 0,
            jobs: 8,
        };
        let key = CasterQueryKey::new(
            NativeGlobalShadowCasterInput {
                depth: Some(&depth),
                ..input
            },
            (1, 2),
            3,
            &[0],
        );
        assert!(
            key == CasterQueryKey::new(
                NativeGlobalShadowCasterInput {
                    depth: Some(&depth),
                    ..input
                },
                (1, 2),
                3,
                &[0]
            )
        );
        assert!(
            key != CasterQueryKey::new(
                NativeGlobalShadowCasterInput {
                    depth: Some(&depth),
                    ..input
                },
                (1, 3),
                3,
                &[0]
            )
        );
        assert!(
            key != CasterQueryKey::new(
                NativeGlobalShadowCasterInput {
                    depth: Some(&depth),
                    ..input
                },
                (1, 2),
                4,
                &[0]
            )
        );
        assert!(
            key != CasterQueryKey::new(
                NativeGlobalShadowCasterInput {
                    depth: Some(&depth),
                    ..input
                },
                (1, 2),
                3,
                &[2]
            )
        );
        assert!(key != CasterQueryKey::new(input, (1, 2), 3, &[0]));
        let changed_planes = [[1.0, 0.0, 0.0, f32::from_bits(0x7fc01235)]];
        let changed_primary = VerifiedShadowReceiverState {
            planes: &changed_planes,
            ..primary
        };
        assert!(
            key != CasterQueryKey::new(
                NativeGlobalShadowCasterInput {
                    primary: changed_primary,
                    depth: Some(&depth),
                    ..input
                },
                (1, 2),
                3,
                &[0]
            )
        );
        let mut changed = NativeGlobalShadowCasterInput {
            depth: Some(&depth),
            ..input
        };
        changed.distance.point[0] = -0.0;
        assert!(key != CasterQueryKey::new(changed, (1, 2), 3, &[0]));
        let mut changed = NativeGlobalShadowCasterInput {
            depth: Some(&depth),
            ..input
        };
        changed.jobs = 4;
        assert!(key != CasterQueryKey::new(changed, (1, 2), 3, &[0]));
        depth.mask[1023] = 1;
        assert!(
            key != CasterQueryKey::new(
                NativeGlobalShadowCasterInput {
                    depth: Some(&depth),
                    ..input
                },
                (1, 2),
                3,
                &[0]
            )
        );
        let mut cache = CasterQueryCache::default();
        assert!(cache.get(&key).is_none());
        let results = Arc::new(vec![vec![NativeGlobalShadowCasterHit {
            tome_index: 7,
            cascade_mask: 1,
        }]]);
        cache.store(key, results.clone());
        let same = CasterQueryKey::new(input, (1, 2), 3, &[0]);
        assert!(cache.get(&same).is_none());
        depth.mask[1023] = 0;
        let same = CasterQueryKey::new(
            NativeGlobalShadowCasterInput {
                depth: Some(&depth),
                ..input
            },
            (1, 2),
            3,
            &[0],
        );
        assert!(Arc::ptr_eq(&cache.get(&same).unwrap(), &results));
    }
    #[test]
    fn projection_cache_updates_changed_geometry_and_checks_reused_masks() {
        let mut cache = ProjectionMaskCache::default();
        let first = cache.query([[0.0; 4]; 4], 0.0, false, || vec![1, 0, 1]);
        assert_eq!(
            cache.query([[0.0; 4]; 4], 0.0, true, || vec![1, 0, 1]),
            first
        );
        assert_eq!(cache.hits, 1);
        assert_eq!(
            cache
                .query([[0.0; 4]; 4], -0.0, false, || vec![0, 1, 0])
                .as_slice(),
            [0, 1, 0]
        );
        assert_eq!(cache.misses, 2);
    }
}
