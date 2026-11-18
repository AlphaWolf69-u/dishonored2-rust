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
    producers_retired: bool,
    heart_retired: bool,
    throwing_retired: bool,
    acquisitions_retired: bool,
    animation_events_retired: bool,
    animation_effects_retired: bool,
    ai_worlds_retired: bool,
    equipment_retired: bool,
    actors_retired: bool,
    physics_retired: bool,
    player_retired: bool,
    effect_requests_retired: bool,
    effects_retired: bool,
    fx_gameplay_retired: bool,
    wind_retired: bool,
    audio_retirement_started: bool,
    cleared: bool,
    retirement_phase: &'static str,
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

    pub(super) fn retirement_phase(&self) -> &'static str { self.retirement_phase }

    /// Complete accepted component operations while the actual camera and
    /// GPU preparation still exist.1408a4830 destroys their resource scopes
    /// only after the component operations have returned.
    pub(super) fn finish_producers(&mut self, world: &mut World) -> Result<bool, String> {
        if self.cleared { return Ok(true); }
        //Scene publication stays alive until this fence. Its ordinary resource
        //preparation and render schedules run once per frame; do not duplicate
        //them here or advance another gameplay/physics/animation tick.
        // SourceReady/EventPlaying cookies can keep an actual projectile ID
        // alive after FX shutdown. Pump their real audio owner and immediately
        // deliver resulting C6/29 messages before any actor can lose that ID.
        self.retirement_phase = "accepted audio and entity events";
        super::script_spawner::begin_retirement(world)?;
        super::animation_effects::begin_retirement(world);
        super::audio::poll_accepted_effects(world)?;
        super::entity_messages::finish_accepted(world)?;
        if !self.animation_events_retired {
            super::player_animation_events::finish_accepted(world)?;
        }
        super::physics_scene::grenades::finish_accepted(world)?;
        // Stop/Deinit callbacks still use the real reflected-function owner.
        // Closing it with ordinary input would reject grenade/Cues teardown.
        super::entity_functions::update(world)?;
        // The source read callback is synchronous. Finish its already accepted
        // host cursor while the real components and render/audio owners remain.
        self.retirement_phase = "accepted interactions";
        super::interactions::finish_accepted_pickups(world)?;
        // 1408a4830 releases component producers before their shared managers.
        // Keep real player/clock/physics owners until power and equipment Stop
        // have withdrawn their requests and acknowledged actual render/audio.
        self.retirement_phase = "component producers";
        if !self.producers_retired {
            if !super::ai_world::creation::close_admission(world)? { return Ok(false); }
            if !super::power_wheel::retire(world, self.catalog.as_ref())? { return Ok(false); }
            super::physics_scene::simulation::finish_accepted(world)?;
            super::physics_scene::carrying_host::finish_accepted_loot_pull(world)?;
            super::pickups::finish_accepted(world)?;
            super::physics_scene::water_publication::update(world)?;
            super::water_impacts::update(world)?;
            if super::water_impacts::accepted_pending(world) { return Ok(false); }
            super::physics_scene::water_publication::release_queries(world)?;
            super::physics_scene::carrying_host::retire(world)?;
            self.retirement_phase = "accepted throwable births";
            if !super::grenade_cooking::shutdown(world)? { return Ok(false); }
            if !self.throwing_retired {
                if !super::weapon_throwing::shutdown(world)? { return Ok(false); }
                self.throwing_retired = true;
            }
            self.retirement_phase = "Heart speech and sensor";
            if !self.heart_retired {
                if !super::heart::shutdown(world)? { return Ok(false); }
                self.heart_retired = true;
            }
            self.retirement_phase = "accepted weapon and impact callbacks";
            if !super::weapon_firing::finish_accepted(world)? { return Ok(false); }
            if !super::melee::finish_accepted(world)? { return Ok(false); }
            if !super::projectiles::finish_accepted(world)? { return Ok(false); }
            if !super::explosions::finish_accepted(world)? { return Ok(false); }
            if !super::script_spawner::finish_accepted(world)? { return Ok(false); }
            if !super::animation_effects::finish_accepted(world)? { return Ok(false); }
            super::physics_scene::fx_component::update(world)?;
            super::entity_messages::finish_accepted(world)?;
            super::entity_functions::update(world)?;
            if !super::breakables::finish_accepted(world)? { return Ok(false); }
            super::breakables::begin_retirement(world);
            super::water_impacts::begin_retirement(world);
            super::water_impacts::retire(world)?;
            super::fx_gameplay::begin_retirement(world);
            super::controller_feedback::retire(world);
            super::explosions::shutdown(world);
            super::power_execution::retire_blinks(world)?;
            super::power_execution::begin_wind_retirement(world)?;
            super::power_world::retire(world)?;
            self.producers_retired = true;
        }
        self.retirement_phase = "level acquisition owners";
        if !self.acquisitions_retired {
            close_level_acquisitions(world);
            if !retire_level_acquisitions(world) { return Ok(false); }
            self.acquisitions_retired = true;
        }
        Ok(true)
    }

    pub fn clear(&mut self, world: &mut World) -> Result<bool, String> {
        if self.cleared { return Ok(true); }
        if !self.finish_producers(world)? { return Ok(false); }
        // The FX manager acknowledges producer tickets before the equipment
        // and entity resource scopes which prepared those effects can close.
        // Keep physics, player and audio callback owners throughout this drain.
        self.retirement_phase = "FX requests";
        if !self.effect_requests_retired {
            super::physics_scene::fx_component::begin_retirement(world);
            super::physics_scene::fx_component::update(world)?;
            super::usable_effects::begin_retirement(world)?;
            self.effect_requests_retired = true;
        }
        // begin_retirement closes late producers; observe real canceled IO
        // completion before any map-scoped source lease can be purged.
        self.retirement_phase = "FX acquisition";
        if world.get_resource_mut::<super::usable_effects::UsableEffects>()
            .is_some_and(|mut effects| !effects.retire_acquisitions()) { return Ok(false); }
        // Keep accepted FX/audio tickets and their resource owners alive
        // through the real consumer's acknowledgement. Successful shutdown is
        // never replayed if a later phase suspends or fails.
        self.retirement_phase = "FX instances";
        if !self.effects_retired {
            world.get_resource_mut::<crate::rendering::bevy_backend::camera_scene::FxSimulation>()
                .map_or(Ok(()), |mut effects| effects.shutdown())?;
            self.effects_retired = true;
        }
        if !super::script_spawner::shutdown(world)? { return Ok(false); }
        // Real FX shutdown fulfills the tickets; consume those receipts before
        // the component registry attempts Stop/Deinit or releases source IDs.
        super::physics_scene::fx_component::update(world)?;
        if !self.animation_effects_retired {
            if !super::animation_effects::shutdown(world)? { return Ok(false); }
            self.animation_effects_retired = true;
        }
        // GameLocal teardown releases the real bare FX children before their
        // arm/equipment parents. During gameplay their completed headers remain.
        if !super::runtime_entities::retire_effect_actors(world)? { return Ok(false); }
        //226060 retains renderer allocations across flash reuse. The actual
        //FX pool has now drained; free those allocations only at pool teardown.
        self.retirement_phase = "FX light renderer allocations";
        if let Some(mut lights) = world.remove_resource::<super::fx_lights::FxLightAllocations>() {
            if let Err(error) = lights.shutdown(world) {
                world.insert_resource(lights);
                return Err(error);
            }
        }
        self.retirement_phase = "FX gameplay actors";
        if !self.fx_gameplay_retired {
            super::fx_gameplay::retire(world)?;
            self.fx_gameplay_retired = true;
        }
        // An already accepted ordinary batch can await a projectile/ContactFX
        // tail. Finish it only after the real FX manager acknowledges shutdown;
        // waiting for it above would prevent that acknowledgement forever.
        // finish_accepted never admits another ordinary removal snapshot.
        self.retirement_phase = "accepted entity removals";
        if !super::projectiles::finish_retirement(world)? { return Ok(false); }
        if !super::entity_removal::finish_accepted(world)? { return Ok(false); }
        self.retirement_phase = "equipment and weapon scopes";
        if !self.equipment_retired {
            if !super::melee::shutdown(world)? { return Ok(false); }
            if !super::weapon_firing::shutdown(world)? { return Ok(false); }
            if !super::explosions::retirement_complete(world)? { return Ok(false); }
            if !super::runtime_materials::equipment_admission::shutdown(world)? { return Ok(false); }
            self.equipment_retired = true;
        }
        if !self.animation_events_retired {
            super::player_animation_events::shutdown(world)?;
            self.animation_events_retired = true;
        }
        self.retirement_phase = "runtime actor render and component receipts";
        if !self.actors_retired {
            if !super::projectiles::shutdown(world) { return Ok(false); }
            if !super::runtime_entities::begin_retirement(world)? { return Ok(false); }
            self.actors_retired = true;
        }
        // Pending/errors preserve the sole registry, pools and body world.
        self.retirement_phase = "placed physics components";
        //Heart/story producers have retired; withdraw the global projection
        //before its actual managed worldspawn can leave the shared scene.
        world.remove_resource::<crate::gameplay::entity_lifetime::WorldspawnEntity>();
        if !self.physics_retired {
            if !super::physics_scene::lifetime::poll(world)? { return Ok(false); }
            self.physics_retired = true;
        }
        if !super::flash_surface_movie::shutdown(world)? { return Ok(false); }
        self.retirement_phase = "AIWorld identities and shared model scopes";
        if !self.ai_worlds_retired {
            // Constructor-owned managers retain actual child generations until
            // their shared component Stop/Deinit callbacks have completed.
            if !super::runtime_entities::retire_ai_worlds(world)? { return Ok(false); }
            if !super::ai_world::creation::shutdown(world)? { return Ok(false); }
            if !super::world_models::shutdown(world)? { return Ok(false); }
            super::entity_functions::begin_retirement(world);
            self.ai_worlds_retired = true;
        }
        self.retirement_phase = "player components";
        if !self.player_retired {
            super::player_lifetime::retire(world)?;
            super::interactions::stop_level(world);
            self.player_retired = true;
        }
        self.retirement_phase = "audio output";
        if !self.audio_retirement_started {
            super::audio::begin_level_retirement(world)?;
            self.audio_retirement_started = true;
        }
        self.retirement_phase = "audio output";
        if !super::audio::poll_level_retirement(world)? { return Ok(false); }
        // Windblast's real tickets include old render snapshots and audio
        // tails. Their weak leases must expire before its managed identity,
        // clock overrides or source declarations can leave the world.
        self.retirement_phase = "wind effects";
        if !self.wind_retired {
            if !super::power_execution::poll_wind_retirement(world)? { return Ok(false); }
            self.wind_retired = true;
        }
        // Resource removal order is arbitrary; hold the model scope until all
        // level consumers have been dropped after the render acknowledgement.
        self.retirement_phase = "level resource purge";
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
            let progress = (self.producers_retired, self.equipment_retired, self.actors_retired,
                self.physics_retired, self.player_retired, self.effect_requests_retired,
                self.effects_retired, self.fx_gameplay_retired, self.audio_retirement_started, self.wind_retired,
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
    close_map_preparation(world);
    if let Some(actions) = world.get_resource::<super::player_actions::PlayerActions>() {
        actions.close_acquisitions();
    }
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

///Release the unpublished map reader's blocked handoff. The shared image
///mailbox also serves accepted runtime FX uploads and stays alive until the
///published world's component producer fence, alongside its actual camera.
pub(super) fn close_map_preparation(world: &mut World) {
    world.remove_resource::<super::texture_streaming::PreparedBaseImages>();
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
        super::shadow_updates::RuntimeSurfaceCasters,
        super::shadow_updates::ParallelModelDraws,
        super::model_motion::ModelPoseUpdates,
        super::model_motion::ModelMotionBindings,
        super::model_motion::ModelShaderData,
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
        super::projected_decals::ModelProjectionContexts,
        super::material_loading::alpha::AlphaDepthMaterials,
    );
    super::audio::begin_level(world);
    if let Some(mailbox) = world.get_resource::<super::PreparedImageMailbox>() {
        mailbox.retire();
    }
}
