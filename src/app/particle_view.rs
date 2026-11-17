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
    windblasts: Option<Res<super::windblast::Windblasts>>,
    live: Option<Res<crate::gameplay::entity_lifetime::LiveEntities>>,
    mut error: Local<Option<String>>,
    (dynamic, physics, identities, mut frame_error): (
        Option<Res<crate::rendering::bevy_backend::world_effects::DynamicEffects>>,
        Option<Res<super::physics_scene::PhysicsScene>>,
        Option<Res<super::equipment_identity::Identities>>,
        Local<Option<String>>,
    ),
) {
    effect_frames.publish_player(
        player
            .player
            .as_ref()
            .map(|player| player.reference_frames()),
    );
    effect_frames.begin_ephemeral();
    if let (Some(windblasts), Some(live)) = (windblasts.as_ref(), live.as_ref()) {
        for (name, frame) in windblasts.frames(live) {
            effect_frames.publish_ephemeral_frame(name, "", frame);
        }
    }
    //Resolve only actual attached FX demands. The managed-name table and
    //reference/animation owners already hold the required identities/poses.
    //No level-entity traversal, asset acquisition or second pose is performed.
    let publication = (|| -> Result<(), String> {
        let (Some(dynamic), Some(live)) = (dynamic.as_ref(), live.as_ref()) else {
            return Ok(());
        };
        let mut first_failure = None;
        for effect in dynamic
            .0
            .iter()
            .filter(|effect| effect.initial_world.is_none())
        {
            let crate::gameplay::event_graph::EntityReference::Named(name) = &effect.target else {
                continue;
            };
            if effect_frames.has_authored_entity(name) && effect.tag.is_empty() {
                continue;
            }
            //An existing source owner (e.g. Windblast) may already have
            //published this frame. Do not require another component owner or
            //resolve a repeated attachment demand again.
            if effect_frames.ephemeral_current(name, "")
                && effect_frames.ephemeral_current(name, &effect.tag)
            {
                continue;
            }
            let Some(id) = live.find(name) else {
                continue;
            };
            if live.stage(id) != Some(crate::gameplay::entity_lifetime::Stage::Spawned) {
                continue;
            }
            let demand = (|| -> Result<(), String> {
                let mut reference = None;
                let frame = match identities
                    .as_ref()
                    .and_then(|owner| owner.published_frame(id))
                {
                    Some((frame, _)) => frame,
                    None => {
                        let actual = physics
                            .as_ref()
                            .ok_or("FX reference has no canonical physics-frame owner")?
                            .published_entity_frame(live, id)?;
                        reference = Some(actual);
                        actual.pose
                    }
                };
                effect_frames.publish_ephemeral_frame(name, "", frame);
                if effect.tag.is_empty() {
                    return Ok(());
                }
                let equipment = player
                    .player
                    .as_ref()
                    .map(|player| player.equipment_entity_tag_world(id, &effect.tag))
                    .transpose()?
                    .flatten();
                let tag = if let Some(frame) = equipment {
                    frame
                } else {
                    let physics = physics
                        .as_ref()
                        .ok_or("FX tag has no canonical animation owner")?;
                    let reference = match reference {
                        Some(actual) => actual,
                        None => physics.published_entity_frame(live, id)?,
                    };
                    let rig = physics.runtime_animation_rig(id)?.ok_or_else(|| {
                        format!("FX tag {name}/{} has no actual animation owner", effect.tag)
                    })?;
                    let matrices = physics.runtime_animation_matrices(id)?.ok_or_else(|| {
                        format!("FX tag {name}/{} has no actual joint palette", effect.tag)
                    })?;
                    super::joint_frames::current_tag_frame(
                        &rig.rig,
                        matrices,
                        "_info",
                        &effect.tag,
                        reference.scale,
                        frame,
                    )?
                    .ok_or_else(|| {
                        format!(
                            "FX tag/joint {name}/{} is absent from its actual model",
                            effect.tag
                        )
                    })?
                };
                effect_frames.publish_ephemeral_frame(name, &effect.tag, tag);
                Ok(())
            })();
            //A retiring/malformed target cannot withhold unrelated source
            //publications. The failed tag remains absent and resolves strictly;
            //it is never replaced by an origin or previous-frame palette.
            if let Err(failure) = demand {
                if first_failure.is_none() {
                    first_failure = Some(failure);
                }
            }
        }
        if let Some(failure) = first_failure {
            return Err(failure);
        }
        Ok(())
    })();
    effect_frames.finish_ephemeral();
    match publication {
        Ok(()) => *frame_error = None,
        Err(failure) => {
            if frame_error.as_ref() != Some(&failure) {
                warn!("FX reference publication failed: {failure}");
            }
            *frame_error = Some(failure);
        }
    }
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
