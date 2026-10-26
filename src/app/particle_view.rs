//! Publish the current ordinary visibility descriptor to the shared FX renderer.
use crate::rendering::bevy_backend::{
    particle_systems::ParticleQueryFrame, water_scene::WaterSceneFrame,
};
use bevy::prelude::*;

pub(super) fn publish(
    scene: Res<super::SceneRenderQueue>,
    water: Res<WaterSceneFrame>,
    mut output: ResMut<ParticleQueryFrame>,
) {
    output.0 = water.0.as_ref().map(|water| {
        std::sync::Arc::new(
            crate::rendering::bevy_backend::particle_systems::ParticleQueryPublication {
                camera: water.camera,
                frame: water.matrices.frame,
                state: scene
                    .shadow_ordinary_query
                    .as_ref()
                    .and_then(|query| query.state.clone()),
            },
        )
    });
}
