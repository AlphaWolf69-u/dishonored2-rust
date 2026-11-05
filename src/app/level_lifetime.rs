//! bevy_backend: map world lifetime inside the persistent host application.
//! 1400410f0 destroys the outgoing world separately from the conditional
//! resource-class purge. Device, configuration and shared resource owners
//! belong to the session; entities and mutable scene publications do not.
use bevy::{ecs::component::ComponentId, prelude::*};
use std::collections::HashSet;

/// A host service entity whose lifetime spans loaded worlds (audio output).
#[derive(Component)]
pub(super) struct SessionEntity;

#[derive(Default)]
pub(super) struct LevelLifetime {
    host_entities: HashSet<Entity>,
    resources: Vec<ComponentId>,
}

impl LevelLifetime {
    pub fn begin(world: &mut World) -> (Self, HashSet<ComponentId>) {
        let host_entities = world.query::<Entity>().iter(world).collect();
        let resources = world.iter_resources().map(|(info, _)| info.id()).collect();
        (
            Self {
                host_entities,
                resources: Vec::new(),
            },
            resources,
        )
    }

    pub fn published(&mut self, world: &World, before: &HashSet<ComponentId>) {
        self.resources = world
            .iter_resources()
            .map(|(info, _)| info.id())
            .filter(|id| !before.contains(id))
            .collect();
    }

    pub fn clear(&mut self, world: &mut World) {
        // Resource removal order is arbitrary; hold the model scope until all
        // level consumers have been dropped after the render acknowledgement.
        let model_history = world.remove_resource::<super::level_prepare::ModelHistoryScope>();
        let material_history = world.remove_resource::<super::level_prepare::MaterialHistoryScope>();
        if let Some(mut patches) = world.get_resource_mut::<crate::rendering::bevy_backend::shader_buffer_patches::ShaderBufferPatches>() {
            patches.release_level();
        }
        let timing = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
        let entities_started = timing.then(std::time::Instant::now);
        world.resource_scope(
            |world, mut shared: Mut<super::texture_streaming::SharedImages>| {
                if let Some(queue) = world.get_resource::<super::SceneRenderQueue>() {
                    shared.capture(queue);
                }
                if let Some(mut ui) = world.get_resource_mut::<super::resident_images::UiImages>() {
                    ui.release_level(&shared);
                }
            },
        );
        let entities: Vec<_> = world
            .query_filtered::<Entity, (
                Without<SessionEntity>,
                Without<bevy::ecs::resource::IsResource>,
            )>()
            .iter(world)
            .filter(|entity| !self.host_entities.contains(entity))
            .collect();
        // Parents can recursively despawn their children; contains avoids a
        // second despawn of an already removed child.
        for entity in entities {
            if world.entities().contains(entity) {
                world.despawn(entity);
            }
        }
        if let Some(started) = entities_started {
            eprintln!("LOAD teardown entities: {:.3} ms", started.elapsed().as_secs_f64() * 1000.0);
        }
        for id in self.resources.drain(..) {
            let resource_timing = timing.then(|| (std::time::Instant::now(),
                world.components().get_info(id).map(|info| info.name().to_string()).unwrap_or_default()));
            world.remove_resource_by_id(id);
            if let Some((started, name)) = resource_timing {
                let elapsed = started.elapsed();
                if elapsed.as_millis() >= 10 {
                    eprintln!("LOAD teardown resource {name}: {:.3} ms", elapsed.as_secs_f64() * 1000.0);
                }
            }
        }
        crate::edge_animation::release_level();
        let _reset = super::load_timing::Span::new("scene publication reset");
        reset_scene_publications(world);
        if let (Some(images), Some(shared)) = (
            world.get_resource::<super::resident_images::UiImages>(),
            world.get_resource::<super::texture_streaming::SharedImages>(),
        ) {
            images.release_read_history(shared, world.get_resource::<Assets<Image>>());
        }
        drop(model_history);
        drop(material_history);
    }
}

/// Plugin registration creates these empty owners before the first map.
/// Reset their map payloads while retaining the plugins/device themselves.
fn reset_scene_publications(world: &mut World) {
    use crate::rendering::bevy_backend as backend;
    macro_rules! reset { ($($ty:ty),* $(,)?) => { $(world.insert_resource(<$ty>::default());)* }; }
    reset!(
        super::scene_queue::VisibleSceneInstances,
        super::visibility_queries::EarlyTomeQuery,
        super::world_instances::WorldMatrices,
        super::shadow_updates::ParallelShadowCpu,
        super::local_shadow_updates::LocalShadowCpu,
        super::local_shadow_updates::LocalShadowViewControls,
        super::local_shadow_updates::LocalShadowDraws,
        super::shadow_updates::ParallelWorldDraws,
        super::shadow_updates::ModelCasterFrame,
        super::shadow_updates::ParallelModelDraws,
        super::model_motion::ModelPoseUpdates,
        super::model_motion::ModelMotionBindings,
        super::LocalLightCullStats,
        super::VisibilityElapsedTiming,
        backend::fog::FogSceneFrame,
        backend::sun_rays::SunRaySources,
        backend::fog_light_scattering::LocalFogFrame,
        backend::display_scene::DisplaySceneFrame,
        backend::water_scene::WaterMaterials,
        backend::water_scene::WaterStencilMaterials,
        backend::water_scene::WaterModelColours,
        backend::water_scene::WaterModelLighting,
        backend::water_scene::WaterLighting,
        backend::water_scene::WaterWorldMeshes,
        backend::water_scene::WaterImageSources,
        backend::water_scene::WaterSceneFrame,
        backend::water_scene::SwimmingSurfaceMailbox,
        backend::water_scene::WaterViewSelection,
        backend::ocean_scene::OceanSources,
        backend::boat_scene::BoatMaterials,
        backend::screen_effects::ScreenEffectMaterials,
        backend::world_effects::WorldEffectSources,
        backend::world_effects::WorldEffectDefinitions,
        backend::world_effects::WorldEffectFrames,
        backend::world_sounds::WorldSounds,
        backend::particle_pass::ParticleMaterialSampler,
        backend::particle_images::SceneParticleImages,
        backend::particle_systems::ParticleQueryFrame,
        backend::physics_queries::PhysicsQueryFrame,
        backend::shadow_pass::ShadowSceneFrame,
        backend::shadow_pass::ShadowSceneCompletion,
        backend::shadow_pass::LocalShadowSceneFrame,
        backend::shadow_pass::LocalShadowCompletion,
        backend::shadow_pass::ShadowCaptureReadiness,
        backend::coarse_depth::CoarseDepthMailbox,
        backend::projected_decals::DecalScene,
        super::material_loading::alpha::AlphaDepthMaterials,
    );
    super::audio::begin_level(world);
    if let Some(mailbox) = world.get_resource::<super::PreparedImageMailbox>() {
        let mut state = mailbox.0.lock().unwrap();
        let generation = state.generation.wrapping_add(1);
        *state = super::texture_streaming::PreparedImageState { generation, ..Default::default() };
    }
}
