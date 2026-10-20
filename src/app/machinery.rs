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
        #[cfg(target_os = "windows")]
        let counter = {
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn QueryPerformanceCounter(value: *mut i64) -> i32;
            }
            let mut value = 0;
            if unsafe { QueryPerformanceCounter(&mut value) } == 0 {
                return Err("scene random counter unavailable".into());
            }
            value as u64
        };
        #[cfg(not(target_os = "windows"))]
        let counter = 0;
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
    machinery.physics.step(std::array::from_fn(|i| {
        clock.delta(i).expect("five clock groups")
    }));
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
