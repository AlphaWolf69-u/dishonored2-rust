//! bevy_backend: map world lifetime inside the persistent host application.
//! 1408a4830 destroys the outgoing GameLocal; 1400410f0 separately
//! performs conditional resource-class purge. Device and shared owners
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
    /// Keep the exact archive namespace that prepared this level.
    catalog: Option<std::sync::Arc<crate::assets::archive::Catalog>>,
    effects_retired: bool,
    audio_retirement_started: bool,
    cleared: bool,
}

impl LevelLifetime {
    pub fn begin(world: &mut World, catalog: std::sync::Arc<crate::assets::archive::Catalog>) -> (Self, HashSet<ComponentId>) {
        let host_entities = world.query::<Entity>().iter(world).collect();
        let resources = world.iter_resources().map(|(info, _)| info.id()).collect();
        (
            Self {
                host_entities,
                catalog: Some(catalog),
                ..Self::default()
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

    pub fn clear(&mut self, world: &mut World) -> Result<bool, String> {
        if self.cleared { return Ok(true); }
        // The source read callback is synchronous. Finish its already accepted
        // host cursor while the real components and render/audio owners remain.
        super::interactions::finish_accepted_pickups(world)?;
        super::player_actions::retire_effects(world)?;
        super::power_world::retire(world)?;
        // Poll while all actual support owners remain published. Pending and
        // errors preserve the sole registry, component pools and body world.
        if !super::physics_scene::lifetime::poll(world)? { return Ok(false); }
        super::player_lifetime::retire(world)?;
        super::interactions::stop_level(world);
        // Keep accepted FX/audio tickets and their resource owners alive
        // through the real consumer's acknowledgement. Successful shutdown is
        // never replayed if a later phase suspends or fails.
        if !self.effects_retired {
            world.get_resource_mut::<crate::rendering::bevy_backend::camera_scene::FxSimulation>()
                .map_or(Ok(()), |mut effects| effects.shutdown())?;
            self.effects_retired = true;
        }
        if !self.audio_retirement_started {
            super::audio::begin_level_retirement(world)?;
            self.audio_retirement_started = true;
        }
        if !super::audio::poll_level_retirement(world)? { return Ok(false); }
        // Resource removal order is arbitrary; hold the model scope until all
        // level consumers have been dropped after the render acknowledgement.
        let model_history = world.remove_resource::<super::level_prepare::ModelHistoryScope>();
        let material_history = world.remove_resource::<super::level_prepare::MaterialHistoryScope>();
        let skeletal_history = world.remove_resource::<super::level_prepare::SkeletalHistoryScope>();
        let shape_history = world.remove_resource::<super::level_prepare::ShapeHistoryScope>();
        let blueprint_history = world.remove_resource::<super::level_prepare::BlueprintHistoryScope>();
        let level_reads = world.remove_resource::<super::level_prepare::LevelSourceReads>();
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
        // A quality change can create an owner that was absent when
        // published() recorded the startup resources.
        world.remove_resource::<super::ParallelShadowOwner>();
        world.remove_resource::<super::local_shadow_updates::LocalShadowOwner>();
        world.remove_resource::<crate::rendering::bevy_backend::camera_scene::FxSimulation>();
        let _reset = super::load_timing::Span::new("scene publication reset");
        reset_scene_publications(world);
        if let (Some(catalog), Some(images), Some(shared)) = (
            self.catalog.as_ref(),
            world.get_resource::<super::resident_images::UiImages>(),
            world.get_resource::<super::texture_streaming::SharedImages>(),
        ) {
            images.release_read_history(catalog.identity(), shared, world.get_resource::<Assets<Image>>());
        }
        drop(model_history);
        drop(material_history);
        drop(skeletal_history);
        drop(shape_history);
        drop(blueprint_history);
        drop(level_reads);
        crate::rendering::bevy_backend::entity_preparation::drain(world);
        if let Some(catalog) = self.catalog.take() {
            catalog.release_level_resources();
        }
        self.cleared = true;
        Ok(true)
    }

    /// Source-acquisition observers have no output device or application
    /// schedule. Advance the actual retirement FIFO synchronously only when
    /// each pass demonstrably finishes another finite owner transition.
    pub(super) fn clear_without_output(&mut self, world: &mut World) -> Result<(), String> {
        if super::audio::has_output(world) { return Err("windowless retirement cannot drive an active output stream".into()); }
        let mut previous = None;
        let mut passes = 0usize;
        loop {
            passes += 1;
            if self.clear(world)? {
                eprintln!("RESOURCE_SCENE_RETIREMENT phase=complete passes={passes} output=false");
                return Ok(());
            }
            let progress = (self.effects_retired, self.audio_retirement_started,
                super::physics_scene::lifetime::retirement_progress(world));
            if previous.is_some_and(|previous| progress <= previous) {
                return Err(format!("windowless retirement made no synchronous owner progress: {progress:?}"));
            }
            previous = Some(progress);
            // Same production consumer/cursor as gameplay. With no output
            // object there is no callback thread to await or substitute.
            super::audio::poll_object_retirements(world)?;
        }
    }
}

pub(super) fn close_level_acquisitions(world: &mut World) {
    if let Some(actions) = world.get_resource::<super::player_actions::PlayerActions>() {
        actions.close_acquisitions();
    }
    // Release a blocked base-image handoff before joining the map reader.
    world.remove_resource::<super::texture_streaming::PreparedBaseImages>();
    if let Some(effects) = world.get_resource::<super::usable_effects::UsableEffects>() {
        effects.close_acquisitions();
    }
    if let Some(interactions) = world.get_resource::<super::interactions::Interactions>() {
        interactions.close_acquisitions();
    }
    if let Some(mut textures) = world.get_resource_mut::<super::texture_streaming::TextureReadWorker>() {
        textures.close_acquisitions();
    }
    if let Some(mailbox) = world.get_resource::<super::PreparedImageMailbox>() {
        mailbox.retire();
    }
}

/// Called while the loading screen is active, before purging map identities.
/// Outgoing decoders can still own source data or write catalog entries after
/// their results are no longer wanted. Observe their retirement before purge.
pub(super) fn retire_level_acquisitions(world: &mut World) -> bool {
    let equipment = world.get_resource_mut::<super::player_actions::PlayerActions>()
        .is_none_or(|mut actions| actions.retire_acquisitions());
    let sources = world.get_resource_mut::<super::usable_effects::UsableEffects>()
        .is_none_or(|mut effects| effects.retire_acquisitions());
    let materials = world.get_resource_mut::<super::runtime_materials::RuntimeMaterialCoordinator>()
        .is_none_or(|mut materials| materials.retire_acquisition());
    let readable = world.get_resource_mut::<super::interactions::Interactions>()
        .is_none_or(|mut interactions| interactions.retire_acquisition());
    let usable_animations = world.get_resource_mut::<super::usable_animation::UsableAnimations>()
        .is_none_or(|mut animations| animations.retire_acquisitions());
    let textures = world.get_resource_mut::<super::texture_streaming::TextureReadWorker>()
        .is_none_or(|mut textures| textures.retire_acquisitions());
    let retired = sources && materials && readable && usable_animations && textures && equipment;
    if retired { crate::rendering::bevy_backend::entity_preparation::drain(world); }
    retired
}

/// Plugin registration creates these empty owners before the first map.
/// Reset their map payloads while retaining the plugins/device themselves.
fn reset_scene_publications(world: &mut World) {
    use crate::rendering::bevy_backend as backend;
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        if let Some(inputs) = world.get_resource::<super::scene_preparation::SceneGpuInputs>() {
            let bytes = inputs.images.capacity() * std::mem::size_of::<bevy::asset::AssetId<Image>>()
                + inputs.meshes.capacity() * std::mem::size_of::<bevy::asset::AssetId<Mesh>>()
                + inputs.buffers.capacity() * std::mem::size_of::<bevy::asset::AssetId<bevy::render::storage::ShaderBuffer>>();
            eprintln!("MEMORY scene GPU IDs retired: images={} meshes={} buffers={} capacity_bytes={} owners={}/{}/{}",
                inputs.images.len(), inputs.meshes.len(), inputs.buffers.len(), bytes,
                std::sync::Arc::strong_count(&inputs.images), std::sync::Arc::strong_count(&inputs.meshes),
                std::sync::Arc::strong_count(&inputs.buffers));
        }
    }
    super::loading_screen::retire_scene_image_requests(world);
    if std::env::var_os("DH2_TRACE_DECALS").is_some() {
        let handles = world.get_resource::<super::projected_decals::GeometryLayouts>()
            .map_or(0, super::projected_decals::GeometryLayouts::sample_handle_count);
        eprintln!("DECAL_LIFETIME world_layout_retired sample_mesh_handles={handles}");
    }
    macro_rules! reset { ($($ty:ty),* $(,)?) => { $(world.insert_resource(<$ty>::default());)* }; }
    reset!(
        super::scene_queue::VisibleSceneInstances,
        super::scene_preparation::SceneGpuInputs,
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
        backend::world_effects::DynamicEffects,
        backend::world_effects::DynamicMaterialPrewarm,
        backend::world_sounds::WorldSounds,
        backend::particle_pass::ParticleMaterialSampler,
        backend::particle_images::SceneParticleImages,
        backend::particle_systems::ParticleQueryFrame,
        backend::camera_scene::FxPublication,
        backend::physics_queries::PhysicsQueryFrame,
        backend::shadow_pass::ShadowSceneFrame,
        backend::shadow_pass::ShadowSceneCompletion,
        backend::shadow_pass::LocalShadowSceneFrame,
        backend::shadow_pass::LocalShadowCompletion,
        backend::shadow_pass::ShadowCaptureReadiness,
        backend::coarse_depth::CoarseDepthMailbox,
        backend::projected_decals::DecalScene,
        backend::projected_decals::DynamicProxyReadiness,
        super::projected_decals::GeometryLayouts,
        super::material_loading::alpha::AlphaDepthMaterials,
    );
    super::audio::begin_level(world);
    if let Some(mailbox) = world.get_resource::<super::PreparedImageMailbox>() {
        mailbox.retire();
    }
}
