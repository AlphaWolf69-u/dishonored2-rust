//! Publish the current ordinary visibility descriptor to the shared FX renderer.
use crate::rendering::bevy_backend::{
    particle_systems::ParticleQueryFrame, water_scene::WaterSceneFrame,
};
use bevy::prelude::*;

///Advance native view parameters before CameraUpdateSystems derives its
///projection/frustum. The late particle pass borrows this same manager.
pub(super) fn advance_view_effects(
    fx: crate::rendering::bevy_backend::camera_scene::FxUpdateInputs,
    mut simulation: Option<ResMut<crate::rendering::bevy_backend::camera_scene::FxSimulation>>,
    physics: Option<Res<super::physics_scene::PhysicsScene>>,
    clocks: Option<Res<crate::gameplay::game_clock_runtime::ClockRuntime>>,
    player: Res<super::player::PlayerState>,
    actions: Option<Res<super::player_actions::PlayerActions>>,
    cameras: Query<(&mut Transform, &mut Projection), With<Camera3d>>,
    frame: Res<bevy::diagnostic::FrameCount>,
    mut last_error: Local<Option<String>>,
    mut screen_fade: Option<ResMut<crate::gameplay::effects::screen_fade::FxScreenFadeFrame>>,
) {
    let (Some(simulation), Some(physics)) = (simulation.as_mut(), physics.as_ref()) else { return };
    let mut cameras = cameras;
    let Ok((mut pose, mut projection)) = cameras.single_mut() else { return };
    let camera = crate::gameplay::reference_frame::ReferenceFrame {
        position: super::view_to_game(pose.translation),
        basis: [super::view_to_game(pose.rotation*Vec3::NEG_Z),
            super::view_to_game(pose.rotation*Vec3::NEG_X), super::view_to_game(pose.rotation*Vec3::Y)],
    };
    let result = (|| -> Result<(), String> {
        if actions.as_ref().is_some_and(|actions| actions.world_screen_fx_ready()) {
            simulation.handoff_screen_fx()?;
        }
        let world = physics.point_queries()?;
        let world = world.read().map_err(|_| "level physics query owner is poisoned")?;
        let managed=fx.live().map(|live|super::managed_frames::Resolver{live,scene:physics,player:player.player.as_ref(),clocks:clocks.as_deref()});
        simulation.advance_world(&fx, frame.0, camera,
            player.player.as_ref().is_some_and(|player| player.is_underwater()), &world,managed.as_ref().map(|resolver|resolver as &dyn crate::rendering::bevy_backend::world_effects::ManagedFrames))?;
        // 14022dac0 -> 14088c5e0: this update's camera shakes move the view
        // origin and turn its axes (rows: forward, left, up); the FX saw
        // the unshaken camera above.
        let shake = simulation.shake_frame();
        if shake != Default::default() {
            let mut origin = camera.position;
            let mut axes = camera.basis;
            shake.apply(&mut origin, &mut axes);
            *pose = Transform::from_translation(super::view_geometry::game_to_view(origin))
                .looking_to(super::view_geometry::game_to_view(axes[0]), super::view_geometry::game_to_view(axes[2]));
        }
        if let Some(fade) = screen_fade.as_mut() {
            fade.0 = simulation.screen_fade_frame();
        }
        //The base remains the Player camera's unmodified gameplay FOV.
        //Percentage FX never feed their previous output back into the mixer.
        if let Some(player) = player.player.as_ref() {
            if let Projection::Custom(custom) = &*projection {
                if let Some(game) = custom.get::<super::camera_projection::GameProjection>() {
                    let fov = simulation.apply_fov(player.camera.fov);
                    if game.reference_horizontal_degrees != fov {
                        *projection = Projection::custom(super::camera_projection::GameProjection::new(fov, game.near, game.far));
                    }
                }
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => *last_error = None,
        Err(error) => {
            if last_error.as_ref() != Some(&error) { warn!("World FX view preparation pending: {error}"); }
            *last_error = Some(error);
        }
    }
}

pub(super) fn publish_reference_frames(
    player: Res<super::player::PlayerState>,
    mut effect_frames: ResMut<crate::rendering::bevy_backend::world_effects::WorldEffectFrames>,
    windblasts: Option<Res<super::windblast::Windblasts>>,
    live: Option<Res<crate::gameplay::entity_lifetime::LiveEntities>>,
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
        for (target, tag, component_target) in dynamic.0.iter().flat_map(|effect| {
            effect.initial_world.is_none().then_some((&effect.target, effect.tag.as_str(), false))
                .into_iter().chain(effect.component_parameters.as_ref().and_then(|parameters|parameters.target.as_ref()).map(|(target,tag)|(target,tag.as_str(),true)))
        })
        {
            let crate::gameplay::event_graph::EntityReference::Named(name) = target else {
                continue;
            };
            if effect_frames.has_authored_entity(name) && tag.is_empty() {
                continue;
            }
            //An existing source owner (e.g. Windblast) may already have
            //published this frame. Do not require another component owner or
            //resolve a repeated attachment demand again.
            if effect_frames.ephemeral_current(name, "")
                && effect_frames.ephemeral_current(name, tag)
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
                if tag.is_empty() {
                    return Ok(());
                }
                let equipment = player
                    .player
                    .as_ref()
                    .map(|player| if component_target {player.animation_entity_tag_world(id,tag)} else {player.equipment_entity_tag_world(id, tag)})
                    .transpose()?
                    .flatten();
                let tag_frame = if let Some(frame) = equipment {
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
                        format!("FX tag {name}/{tag} has no actual animation owner")
                    })?;
                    let matrices = physics.runtime_animation_matrices(id)?.ok_or_else(|| {
                        format!("FX tag {name}/{tag} has no actual joint palette")
                    })?;
                    super::joint_frames::current_tag_frame(
                        &rig.rig,
                        matrices,
                        "_info",
                        tag,
                        reference.scale,
                        frame,
                    )?
                    .ok_or_else(|| {
                        format!(
                            "FX tag/joint {name}/{tag} is absent from its actual model"
                        )
                    })?
                };
                effect_frames.publish_ephemeral_frame(name, tag, tag_frame);
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
}

pub(super) fn publish(
    scene: Res<super::SceneRenderQueue>,
    data: Res<super::SceneData>,
    water: Res<WaterSceneFrame>,
    mut output: ResMut<ParticleQueryFrame>,
    world_wind: Res<super::fog_scene::WorldWind>,
    wind: Option<Res<super::vegetation_wind::VegetationWind>>,
    mut error: Local<Option<String>>,
) {
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
