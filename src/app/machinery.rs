//! Client wiring for archive-defined constrained ambient machinery.
use super::model_motion::{ModelPose, ModelPoseUpdates};
use crate::{
    archive::Catalog,
    entities::{Document, PlacedModel},
    gameplay::{game_clock_runtime::ClockRuntime, machinery_world::MachineryWorld},
};
use bevy::prelude::*;
#[derive(Resource, Default)]
pub(super) struct AmbientMachinery {
    physics: MachineryWorld,
    scenes: crate::gameplay::scene_motion::SceneMotion,
}
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
        let graphs = crate::entities::scene_motion::load(catalog, document)?;
        let counter = crate::platform::performance_counter::counter()? as u64;
        let scenes = crate::gameplay::scene_motion::SceneMotion::new(graphs, &placements, counter);
        for diagnostic in &scenes.diagnostics {
            warn!("Ambient scene motion: {diagnostic}");
        }
        info!(
            "Ambient scene motion: {} playing groups",
            scenes.playing_count()
        );
        Ok(Self {
            physics: world,
            scenes,
        })
    }
}
pub(super) fn update_machinery(
    clock: Res<ClockRuntime>,
    mut machinery: ResMut<AmbientMachinery>,
    mut pending: ResMut<ModelPoseUpdates>,
    world_wind: Res<super::fog_scene::WorldWind>,
    winds: Option<Res<super::vegetation_wind::VegetationWind>>,
) {
    if !clock.advanced() {
        return;
    }
    let Ok(clock) = clock.clock() else {
        return;
    };
    let machinery = &mut *machinery;
    machinery.scenes.advance_motors(
        clock.delta(3).expect("five clock groups"),
        &mut machinery.physics,
    );
    let manager = winds.as_deref().map(|winds| winds.manager());
    // 7b8340 -> 807680, before the physics step: a body inside trigger winds
    // uses the first (if started); otherwise the registered world wind.
    let actor_seconds = clock.delta(3).expect("five clock groups");
    if actor_seconds > 0. {
        for body in &mut machinery.physics.bodies {
            let Some(state) = &body.wind else {
                continue;
            };
            let world = world_wind.0.as_ref();
            let wind = if !state.triggers.is_empty() || world.is_none() {
                state.triggers.first().and_then(|&index| {
                    let manager = manager?;
                    manager.trigger_shapes[index].1.then(|| &manager.triggers[index])
                })
            } else {
                world
            };
            if let Some(wind) = wind {
                let velocity = wind.velocity(Vec3::from_array(body.mass_center()));
                body.apply_wind(velocity.to_array(), actor_seconds);
            }
        }
    }
    machinery.physics.step(std::array::from_fn(|i| {
        clock.delta(i).expect("five clock groups")
    }));
    if let Some(manager) = manager {
        for body in &mut machinery.physics.bodies {
            body.update_wind_triggers(&manager.trigger_shapes);
        }
    }
    pending
        .0
        .extend(machinery.physics.bodies.iter().filter_map(|body| {
            body.placement.map(|placement| ModelPose {
                placement,
                delta: body.delta(),
            })
        }));
    machinery.scenes.step(
        clock.delta(3).expect("five clock groups"),
        |placement, delta| pending.0.push(ModelPose { placement, delta }),
    );
}
