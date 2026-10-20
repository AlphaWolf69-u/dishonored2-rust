//! Client wiring for archive-defined constrained ambient machinery.
use super::model_motion::{ModelPose, ModelPoseUpdates};
use crate::{
    archive::Catalog,
    entities::{Document, PlacedModel},
    gameplay::{game_clock_runtime::ClockRuntime, machinery_world::MachineryWorld},
};
use bevy::prelude::*;
#[derive(Resource, Default)]
pub(super) struct AmbientMachinery(MachineryWorld);
impl AmbientMachinery {
    pub(super) fn load(
        catalog: &Catalog,
        document: Option<&Document>,
        placements: &[PlacedModel],
    ) -> Result<Self, String> {
        let Some(document) = document else {
            return Ok(Self::default());
        };
        let placements = super::expanded_model_placements(placements, catalog);
        let recipes = crate::entities::machinery::recipes::load(catalog, document, &placements)?;
        for diagnostic in &recipes.unsupported {
            warn!("Ambient machinery assembly unsupported: {diagnostic}");
        }
        let world = MachineryWorld::from_recipes(&recipes)?;
        info!("Ambient machinery: {} assemblies, {} bodies, {} constraints, {} unsupported assemblies",recipes.assemblies.len(),world.bodies.iter().filter(|b|b.placement.is_some()).count(),world.joint_count(),recipes.unsupported.len());
        Ok(Self(world))
    }
}
pub(super) fn update_machinery(
    clock: Res<ClockRuntime>,
    mut machinery: ResMut<AmbientMachinery>,
    mut pending: ResMut<ModelPoseUpdates>,
) {
    if !clock.advanced() {
        return;
    }
    let Ok(clock) = clock.clock() else {
        return;
    };
    machinery.0.step(std::array::from_fn(|i| {
        clock.delta(i).expect("five clock groups")
    }));
    pending
        .0
        .extend(machinery.0.bodies.iter().filter_map(|body| {
            body.placement.map(|placement| ModelPose {
                placement,
                delta: body.delta(),
            })
        }));
}
