//! Render-view query and retained selection share one lifetime. Physics owns
//! the world and geometry; this view never allocates a second collision world.
use super::{ContactEntity, EnvironmentContactSelection, EnvironmentViewQuery};
use crate::gameplay::{
    collision_queries::CollisionQueries,
    component_state::{entity::ComponentHandle, ComponentOwner},
    physics_world::storage::WorldStorage,
    shape_point_query::ShapePointContact,
    world_point_query::{WorldPointQuery, WorldPointRequest, WorldQueryShape},
};
use std::sync::Arc;

pub struct EnvironmentViewContacts {
    pub query: EnvironmentViewQuery,
    pub selection: EnvironmentContactSelection,
    /// Started water components at the camera/head query, independently of
    /// render environment volumes. 707c50 selects the highest water plane.
    pub head_water_surface: Option<f32>,
    pub head_water_is_ocean: bool,
}
impl EnvironmentViewContacts {
    pub fn new(retained_water_height: f32) -> Self {
        Self {
            query: EnvironmentViewQuery::default(),
            selection: EnvironmentContactSelection::new(retained_water_height),
            head_water_surface: None,
            head_water_is_ocean: false,
        }
    }

    /// Execute queued geometry work against the canonical world, then88d060
    /// consumes only this view's completed request through its live owners.
    pub(crate) fn resolve<'a, 'component>(
        &mut self,
        world: &WorldStorage,
        executor: &mut WorldPointQuery,
        queries: &mut CollisionQueries<WorldPointRequest, ShapePointContact>,
        geometry: impl FnMut(usize) -> Result<WorldQueryShape<'a>, String>,
        component: impl FnMut(ComponentHandle) -> Option<&'component ComponentOwner>,
        entity: impl FnMut(usize) -> Result<ContactEntity, String>,
    ) -> Result<bool, String> {
        world.resolve_point_queries(executor, queries, geometry)?;
        self.selection.consume_query(
            queries,
            &mut self.query.id,
            component,
            &world.bodies,
            &world.quantization.packed,
            entity,
        )
    }
}

///88e220's water publication occurs after ordinary environment tracks. A
/// selected volume only enables its surface when it supplies a real material.
#[derive(Clone, Debug)]
pub struct WaterViewPublication {
    pub surface_material: Option<Arc<str>>,
    pub surface_height: Option<f32>,
    pub update_ocean_height: bool,
}
impl EnvironmentContactSelection {
    pub fn water_publication(
        &self,
        mut material: impl FnMut(usize) -> Result<Option<Arc<str>>, String>,
    ) -> Result<WaterViewPublication, String> {
        let surface_material = self.water.map(&mut material).transpose()?.flatten();
        Ok(WaterViewPublication {
            surface_height: surface_material.as_ref().map(|_| self.water_level),
            surface_material,
            update_ocean_height: self.update_ocean_height(),
        })
    }
}
impl WaterViewPublication {
    /// Numeric publication is independent of the material's pass bindings.
    /// The latter retain the same material identity for the surface draw owner.
    pub fn apply(&self, settings: &mut crate::rendering::ocean::OceanViewInput) {
        settings.water_volume = self.surface_material.is_some();
        if let Some(height) = self.surface_height {
            settings.surface_water_level = height;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn water_publication_matches_executed_post_track_override() {
        for (case, row) in include_str!("../../../tests/fixtures/water_view_publication.txt")
            .lines()
            .enumerate()
        {
            let words: Vec<_> = row
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let mut selection = EnvironmentContactSelection::new(f32::from_bits(words[3]));
            selection.water = (words[0] != 0).then_some(7);
            selection.touches_water_trigger = words[2] != 0;
            let mut events = Vec::new();
            let publication = selection
                .water_publication(|index| {
                    assert_eq!(index, 7);
                    events.push(10);
                    Ok((words[1] != 0).then(|| Arc::from("water/test")))
                })
                .unwrap();
            let input = &mut crate::rendering::ocean::OceanViewInput {
                enabled: true,
                enable_ssr: false,
                disable_ssr_on_rough_water: false,
                use_sky_cube_map: false,
                water_volume: true,
                surface_water_level: 123.,
                ocean_water_level: 99.,
                max_wave_amplitude: 1.,
                wind: [0.; 2],
                foam_type: 0.,
                wind_speed: 0.,
                wind_dependency: 0.,
                small_waves: 0.,
                choppiness: 0.,
            };
            publication.apply(input);
            if publication.surface_material.is_some() {
                events.extend([
                    1,
                    input.surface_water_level.to_bits(),
                    input.surface_water_level.to_bits(),
                    input.surface_water_level.to_bits(),
                    input.surface_water_level.to_bits(),
                ]);
            } else {
                assert_eq!(
                    input.surface_water_level, 123.,
                    "unselected height must remain retained"
                );
            }
            events.extend([
                2,
                u32::from(input.water_volume),
                u32::from(input.water_volume),
                u32::from(input.water_volume),
                u32::from(input.water_volume),
            ]);
            if publication.surface_material.is_some() {
                events.extend([3, 1]);
            }
            assert_eq!(
                u32::from(publication.update_ocean_height),
                words[4],
                "case {case}"
            );
            assert_eq!(events.len(), words[5] as usize);
            assert_eq!(events, words[6..], "case {case}");
        }
    }
}
