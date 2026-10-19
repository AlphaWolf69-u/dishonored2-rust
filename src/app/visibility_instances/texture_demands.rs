//! Immutable Tome/BWM/material/image associations prepared with the scene.
use super::*;

#[derive(Debug)]
struct Group {
    images: std::ops::Range<usize>,
    error: Option<&'static str>,
}

#[derive(Debug, Default)]
pub(crate) struct PreparedTextureDemands {
    groups: Vec<Group>,
    images: Vec<(usize, bool)>,
}

impl PreparedTextureDemands {
    pub(crate) fn new(
        mapping: &TomeRenderInstanceMap,
        instances: &[RenderInstance],
        bindings: &HashMap<MaterialHandle, Vec<usize>>,
    ) -> Self {
        let mut result = Self::default();
        for range in &mapping.ranges {
            let start = result.images.len();
            let mut error = None;
            for &index in &mapping.indices[range.clone()] {
                let Some(instance) = instances.get(index) else {
                    error = Some("Tome render-instance mapping exceeds scene render queue");
                    break;
                };
                let slots = bindings
                    .get(&instance.material)
                    .map(Vec::as_slice)
                    .unwrap_or_else(|| instance.diffuse_image_slot.as_slice());
                let zero = mapping.zero_metric_instances.contains(&index);
                result.images.extend(slots.iter().map(|&slot| (slot, zero)));
            }
            result.groups.push(Group {
                images: start..result.images.len(),
                error,
            });
        }
        result
    }

    pub(crate) fn submit(
        &self,
        indices: &[u32],
        metrics: &[Option<f32>],
        demands: &[texture::ProjectedAreaDemand],
    ) -> Result<usize, String> {
        if indices.len() != metrics.len() {
            return Err("Tome visibility indices and area outputs have different lengths".into());
        }
        let mut maximum = vec![0.0; demands.len()];
        let result = (|| {
            let mut submitted = 0;
            for (&index, metric) in indices.iter().zip(metrics) {
                let group = self
                    .groups
                    .get(index as usize)
                    .ok_or("visible Tome index exceeds render-instance mapping")?;
                let Some(metric) = metric else {
                    continue;
                };
                for &(slot, zero) in &self.images[group.images.clone()] {
                    let target = maximum
                        .get_mut(slot)
                        .ok_or("render instance references an unknown material image")?;
                    let area = if zero { 0.0 } else { *metric };
                    if area > *target {
                        *target = area;
                    }
                    submitted += 1;
                }
                if let Some(error) = group.error {
                    return Err(error.into());
                }
            }
            Ok(submitted)
        })();
        // As in the reference consumer, an invalid later association retains
        // all contributions made before it, including a partial material list.
        for (demand, area) in demands.iter().zip(maximum) {
            if area > 0.0 {
                demand.submit(area);
            }
        }
        result
    }

    pub(crate) fn verify(
        &self,
        mapping: &TomeRenderInstanceMap,
        indices: &[u32],
        metrics: &[Option<f32>],
        instances: &[RenderInstance],
        bindings: &HashMap<MaterialHandle, Vec<usize>>,
        image_count: usize,
    ) {
        let actual: Vec<_> = (0..image_count)
            .map(|_| texture::ProjectedAreaDemand::default())
            .collect();
        let expected: Vec<_> = (0..image_count)
            .map(|_| texture::ProjectedAreaDemand::default())
            .collect();
        assert_eq!(
            self.submit(indices, metrics, &actual),
            mapping
                .submit_material_texture_demands(indices, metrics, instances, &expected, bindings),
            "prepared image associations changed submission count or error"
        );
        for (slot, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
            assert_eq!(
                actual.area().to_bits(),
                expected.area().to_bits(),
                "prepared image association changed area at image {slot}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_associations_preserve_suppression_lazy_errors_and_partial_materials() {
        let mut mapping = TomeRenderInstanceMap::from_authored_bwm_mapping(
            &[vec![0, 1], vec![2], vec![]],
            &[vec![0], vec![1], vec![0, 77]],
        );
        mapping.zero_metric_instances.insert(1);
        let instances = [0, 1].map(|slot| RenderInstance {
            source: None,
            mesh: Handle::default(),
            material: MaterialHandle::Probe(Handle::default()),
            diffuse_image_slot: Some(slot),
            probe_index: 0,
            transform: Transform::IDENTITY,
            bounds: bevy::camera::primitives::Aabb::from_min_max(Vec3::ZERO, Vec3::ONE),
            pickup_id: None,
            entity: None,
            visible: false,
        });
        let mut bindings = HashMap::new();
        let check = |bindings: &HashMap<MaterialHandle, Vec<usize>>| {
            let prepared = PreparedTextureDemands::new(&mapping, &instances, bindings);
            for (indices, metrics) in [
                (vec![0, 0, 2], vec![Some(0.25), Some(0.5), None]),
                (vec![0, 1, 0], vec![Some(0.3), Some(0.6), Some(1.0)]),
                (vec![1, 0], vec![None, Some(0.125)]),
                (vec![0, 3], vec![Some(0.75), Some(1.0)]),
                (
                    vec![0, 0, 0],
                    vec![Some(f32::NAN), Some(-0.0), Some(f32::INFINITY)],
                ),
                (vec![0], vec![]),
            ] {
                for image_count in [0, 1, 2] {
                    prepared.verify(
                        &mapping,
                        &indices,
                        &metrics,
                        &instances,
                        bindings,
                        image_count,
                    );
                }
            }
        };
        check(&bindings);
        bindings.insert(instances[0].material.clone(), vec![0, 1, 0]);
        check(&bindings);
        bindings.insert(instances[0].material.clone(), vec![0, 9, 1]);
        check(&bindings);
    }
}
