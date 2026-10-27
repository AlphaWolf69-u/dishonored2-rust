//! Publish the current ordinary visibility descriptor to the shared FX renderer.
use crate::rendering::bevy_backend::{
    particle_systems::ParticleQueryFrame, water_scene::WaterSceneFrame,
};
use bevy::prelude::*;

pub(super) fn publish(
    scene: Res<super::SceneRenderQueue>,
    player: Res<super::player::PlayerState>,
    mut effect_frames: ResMut<crate::rendering::bevy_backend::world_effects::WorldEffectFrames>,
    data: Res<super::SceneData>,
    water: Res<WaterSceneFrame>,
    mut output: ResMut<ParticleQueryFrame>,
    world_wind: Res<super::fog_scene::WorldWind>,
    wind: Option<Res<super::vegetation_wind::VegetationWind>>,
    mut error: Local<Option<String>>,
) {
    effect_frames.publish_player(player.player.as_ref().map(|player| player.reference_frames()));
    let snapshot = crate::gameplay::wind::WindSnapshot::capture(
        world_wind.0.as_ref(),
        wind.as_ref().map(|wind| wind.manager()),
    );
    let snapshot = match snapshot {
        Ok(snapshot) => {
            *error = None;
            snapshot
        }
        Err(failure) => {
            if error.as_ref() != Some(&failure) {
                warn!("Particle wind publication pending: {failure}");
                *error = Some(failure);
            }
            output.0 = None;
            return;
        }
    };
    output.0 = water.0.as_ref().map(|water| {
        std::sync::Arc::new(
            crate::rendering::bevy_backend::particle_systems::ParticleQueryPublication {
                camera: water.camera,
                frame: water.matrices.frame,
                wind: snapshot,
                probes: data.environment_probes.clone(),
                state: scene
                    .shadow_ordinary_query
                    .as_ref()
                    .and_then(|query| query.state.clone()),
            },
        )
    });
}
