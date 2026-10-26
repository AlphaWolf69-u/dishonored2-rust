//! Resolve ordered world hits through their actual collision component ownership.
use super::{EnvironmentContact, EnvironmentContactSelection, EnvironmentVolumeContact};
use crate::gameplay::{
    body_storage::BodyStorage,
    broadphase::PackedQuantization,
    collision_queries::{CollisionQueries, QueryId},
    collision_user_data::CollisionUserData,
    component_state::{entity::ComponentHandle, ComponentOwner},
    shape_point_query::ShapePointContact,
};

/// Facts supplied by the entity/component owner. The trigger class gate and
/// area-volume class gate in88d060 are independent; a water component alone
/// does not make an arbitrary entity eligible for the trigger test.
pub struct ContactEntity {
    pub trigger_derived: bool,
    pub water_component_flags: Option<u32>,
    pub environment: Option<(usize, bool)>,
}

impl EnvironmentContactSelection {
    /// Complete88d060 contact consumption: clear this view first, consume only
    /// a completed query, then resolve its ordered hits through live components.
    /// Pending/invalid queries keep the retained height but no selected volume.
    pub fn consume_query<'a, Request>(
        &mut self,
        queries: &mut CollisionQueries<Request, ShapePointContact>,
        id: &mut QueryId,
        component: impl FnMut(ComponentHandle) -> Option<&'a ComponentOwner>,
        bodies: &BodyStorage,
        packing: &PackedQuantization,
        entity: impl FnMut(usize) -> Result<ContactEntity, String>,
    ) -> Result<bool, String> {
        self.update(None);
        if !id.is_valid() {
            return Ok(false);
        }
        let mut hits = Vec::new();
        if !queries
            .take_completed(id, &mut hits)
            .map_err(|e| e.to_string())?
        {
            return Ok(false);
        }
        let mut contacts = Vec::with_capacity(hits.len());
        resolve_contacts(&hits, component, bodies, packing, entity, &mut contacts)?;
        self.update(Some(&contacts));
        Ok(true)
    }
}

/// Preserve query order and duplicates. Bounds come from the hit body through
/// c85490's packed getter, never from the entity model or its current wrapper.
pub fn resolve_contacts<'a>(
    hits: &[ShapePointContact],
    mut component: impl FnMut(ComponentHandle) -> Option<&'a ComponentOwner>,
    bodies: &BodyStorage,
    packing: &PackedQuantization,
    mut entity: impl FnMut(usize) -> Result<ContactEntity, String>,
    output: &mut Vec<EnvironmentContact>,
) -> Result<(), String> {
    let start = output.len();
    let result = (|| {
        for hit in hits {
            let owner = CollisionUserData::decode(hit.target.user_data)?.entity(&mut component)?;
            let entity = entity(owner)?;
            let environment = if let Some((index, is_water_volume)) = entity.environment {
                let body = hit.target.body_id.ok_or("environment hit has no body ID")?;
                let body = bodies
                    .get_by_index(body & 0xffffff)
                    .ok_or("environment hit body is missing")?;
                let bounds = packing.unpack_bounds(body.bounds);
                Some(EnvironmentVolumeContact {
                    index,
                    is_water_volume,
                    bounds: bounds.map(|v| [v[0], v[1], v[2]]),
                })
            } else {
                None
            };
            output.push(EnvironmentContact {
                water_component_flags: entity
                    .trigger_derived
                    .then_some(entity.water_component_flags)
                    .flatten(),
                environment,
            });
        }
        Ok(())
    })();
    if result.is_err() {
        output.truncate(start);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::{
        broadphase::{BroadphaseQuantization, PackedBodyBounds},
        collision_hit::ShapeHitInfo,
    };

    #[test]
    fn view_consumes_completed_query_once_and_clears_selection_while_pending() {
        let handle = ComponentHandle {
            class_id: 17,
            slot: 0,
        };
        let owner = ComponentOwner {
            entity: Some(42),
            ..Default::default()
        };
        let mut bodies = BodyStorage::new(2).unwrap();
        let body = bodies.reserve(&mut []).unwrap();
        let packing = BroadphaseQuantization::new([[-100.; 4], [100.; 4]]);
        bodies.get_mut(body).unwrap().bounds = packing
            .packed
            .pack_bounds([[-2., -2., -2., 0.], [2., 2., 3., 0.]], [0.; 4]);
        let hit = ShapePointContact {
            target: ShapeHitInfo {
                body_id: Some(body),
                material_id: 0,
                shape_key: u32::MAX,
                filter_info: 0,
                user_data: CollisionUserData { component: handle }.encode(),
            },
        };
        let mut queries = CollisionQueries::default();
        let mut id = queries.submit((), true).unwrap();
        let saved_id = id;
        let component = |key| (key == handle).then_some(&owner);
        let entity = |entity| {
            assert_eq!(entity, 42);
            Ok(ContactEntity {
                trigger_derived: true,
                water_component_flags: Some(4),
                environment: Some((7, true)),
            })
        };
        let mut selection = EnvironmentContactSelection::new(20.);
        selection.water = Some(999);
        selection.areas.push(999);
        selection.touches_water_trigger = true;
        assert!(!selection
            .consume_query(
                &mut queries,
                &mut id,
                component,
                &bodies,
                &packing.packed,
                entity
            )
            .unwrap());
        assert_eq!(id, saved_id);
        assert!(selection.areas.is_empty());
        assert_eq!(selection.water, None);
        assert!(!selection.touches_water_trigger);
        assert_eq!(selection.water_level, 20.);
        queries.resolve_pending(|_, hits| {
            hits.push(hit);
            hits.push(hit);
        });
        assert!(selection
            .consume_query(
                &mut queries,
                &mut id,
                component,
                &bodies,
                &packing.packed,
                entity
            )
            .unwrap());
        assert_eq!(id, QueryId::INVALID);
        assert_eq!(selection.water, Some(7));
        assert!(selection.touches_water_trigger);
        assert!(!selection.update_ocean_height());
        assert!(selection.water_level > 2.99 && selection.water_level < 3.01);
        let height = selection.water_level;
        assert!(!selection
            .consume_query(
                &mut queries,
                &mut id,
                component,
                &bodies,
                &packing.packed,
                entity
            )
            .unwrap());
        assert_eq!(selection.water, None);
        assert!(!selection.touches_water_trigger);
        assert_eq!(selection.water_level, height);
        let mut stale = saved_id;
        assert!(selection
            .consume_query(
                &mut queries,
                &mut stale,
                component,
                &bodies,
                &packing.packed,
                entity
            )
            .is_err());
    }

    #[test]
    fn contacts_use_hit_body_bounds_and_retained_component_owner() {
        let handle = ComponentHandle {
            class_id: 17,
            slot: 0,
        };
        let component_owner = ComponentOwner {
            entity: Some(42),
            ..Default::default()
        };
        let mut bodies = BodyStorage::new(3).unwrap();
        bodies.reserve(&mut []).unwrap();
        let id = bodies.reserve(&mut []).unwrap();
        let hit = ShapePointContact {
            target: ShapeHitInfo {
                body_id: Some(id),
                material_id: 0,
                shape_key: u32::MAX,
                filter_info: 0,
                user_data: CollisionUserData { component: handle }.encode(),
            },
        };
        let mut count = 0;
        for row in include_str!("../../../tests/fixtures/body_world_bounds.txt").lines() {
            let w: Vec<_> = row
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let world =
                std::array::from_fn(|i| std::array::from_fn(|j| f32::from_bits(w[i * 4 + j])));
            let packing = BroadphaseQuantization::new(world);
            bodies.get_mut(id).unwrap().bounds = PackedBodyBounds {
                words: w[8..12].try_into().unwrap(),
            };
            let mut output = Vec::new();
            let trigger_derived = count % 2 == 0;
            resolve_contacts(
                &[hit, hit],
                |key| (key == handle).then_some(&component_owner),
                &bodies,
                &packing.packed,
                |entity| {
                    assert_eq!(entity, 42);
                    Ok(ContactEntity {
                        trigger_derived,
                        water_component_flags: Some(4),
                        environment: Some((9, true)),
                    })
                },
                &mut output,
            )
            .unwrap();
            assert_eq!(output.len(), 2);
            for contact in &output {
                assert_eq!(contact.water_component_flags, trigger_derived.then_some(4));
                let volume = contact.environment.unwrap();
                assert_eq!(volume.index, 9);
                assert!(volume.is_water_volume);
                let actual: Vec<_> = volume
                    .bounds
                    .iter()
                    .flatten()
                    .map(|f| f.to_bits())
                    .collect();
                let expected: Vec<_> = w[12..]
                    .chunks_exact(4)
                    .flat_map(|v| v[..3].iter().copied())
                    .collect();
                assert_eq!(actual, expected, "bounds {count}");
            }
            let mut invalid = hit;
            invalid.target.user_data = 0;
            assert!(resolve_contacts(
                &[hit, invalid],
                |key| (key == handle).then_some(&component_owner),
                &bodies,
                &packing.packed,
                |_| Ok(ContactEntity {
                    trigger_derived: true,
                    water_component_flags: None,
                    environment: None
                }),
                &mut output
            )
            .is_err());
            assert_eq!(output.len(), 2);
            count += 1;
        }
        assert_eq!(count, 1536);
    }
}
