//! bevy_backend: off-thread scene cooking, with the live world's allocators.
use super::*;
use crate::rendering::bevy_backend::asset_preparation::AssetBatch;
use crate::rendering::bevy_backend::sky_mask::SkyMaterial;
use bevy::prelude::*;

#[derive(Resource)]
pub(super) struct SetupPrepared;

#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
pub(super) struct SceneGpuInputs {
    pub images: Arc<Vec<AssetId<Image>>>,
    pub meshes: Arc<Vec<AssetId<Mesh>>>,
    pub buffers: Arc<Vec<AssetId<ShaderBuffer>>>,
}

impl SceneGpuInputs {
    pub(super) fn install(app: &mut App) {
        use bevy::{
            ecs::schedule::common_conditions::run_once,
            render::{
                extract_resource::ExtractResourcePlugin,
                mesh::allocator::{allocate_and_free_meshes, MeshAllocator},
                Render, RenderApp, RenderSystems,
            },
        };
        app.init_resource::<Self>()
            .add_plugins(ExtractResourcePlugin::<Self>::default());
        if std::env::var_os("DH2_DEBUG_MIX_BOOTSTRAP_MESH_SLABS").is_some_and(|v| v == "1") {
            return;
        }
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            fn scene_has_meshes(inputs: Res<SceneGpuInputs>) -> bool {
                !inputs.meshes.is_empty()
            }
            fn seal_bootstrap_slabs(mut allocator: ResMut<MeshAllocator>) {
                allocator.seal_existing_slabs();
            }
            // World purge can free its buffers even while the bootstrap
            // meshes live. No buffer or existing allocation is moved here.
            render.add_systems(
                Render,
                seal_bootstrap_slabs
                    .in_set(RenderSystems::PrepareAssets)
                    .before(allocate_and_free_meshes)
                    .run_if(scene_has_meshes.and(run_once)),
            );
        }
    }
}

pub(super) struct ScenePreparation {
    meshes: AssetBatch<Mesh>,
    images: super::resident_images::PreparedImages,
    probes: AssetBatch<ProbeMaterial>,
    decals: AssetBatch<DecalMaterial>,
    skies: AssetBatch<SkyMaterial>,
    buffers: AssetBatch<ShaderBuffer>,
    allocator: bevy::ecs::entity::RemoteAllocator,
    device: RenderDevice,
    audio: super::audio::AudioPreparation,
}
impl ScenePreparation {
    pub fn new(world: &mut World) -> Self {
        let resident_images = world.resource::<super::resident_images::ResidentImages>().clone();
        let shared_images = world.resource::<super::texture_streaming::SharedImages>().clone();
        Self {
            meshes: AssetBatch::new(world.resource::<Assets<Mesh>>()),
            images: super::resident_images::PreparedImages::new(
                world.resource::<Assets<Image>>(), resident_images, shared_images),
            probes: AssetBatch::new(world.resource::<Assets<ProbeMaterial>>()),
            decals: AssetBatch::new(world.resource::<Assets<DecalMaterial>>()),
            skies: AssetBatch::new(world.resource::<Assets<SkyMaterial>>()),
            buffers: AssetBatch::new(world.resource::<Assets<ShaderBuffer>>()),
            allocator: world.entity_allocator().build_remote_allocator(),
            device: world.resource::<RenderDevice>().clone(),
            audio: super::audio::AudioPreparation::take(world),
        }
    }
    pub fn prepare(
        mut self,
        prepared: &mut super::loading_screen::PreparedLevel,
        base_image_batch_bytes: Option<usize>,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<super::texture_streaming::BaseImageReader>, String> {
        let check_cancelled = || {
            if cancelled() {
                Err("scene preparation cancelled".to_owned())
            } else {
                Ok(())
            }
        };
        check_cancelled()?;
        let resident_images = self.images.registry().clone();
        let mut scene = prepared
            .take_resource::<SceneData>()
            .expect("prepared scene");
        self.images.begin_acquisition(scene.catalog.as_deref());
        let mut programs = prepared.take_resource::<ProgramCacheSource>();
        let navigation = prepared.take_resource::<NavigationWorld>();
        let tome = prepared.take_resource::<TomeWorld>();
        let settings = prepared
            .take_resource::<ViewSettings>()
            .expect("prepared view settings");
        let machinery = prepared.take_resource::<super::machinery::AmbientMachinery>();
        let interactions = prepared.take_resource::<super::interactions::Interactions>();
        let volumes = prepared.take_resource::<super::state::EnvironmentVolumes>();
        let effects = prepared
            .take_resource::<crate::rendering::bevy_backend::world_effects::WorldEffectDefinitions>(
            );
        let mut matrices = super::world_instances::WorldMatrices::default();
        let mut commands = super::setup_commands::SetupCommands::Prepared {
            allocator: self.allocator,
            queue: Default::default(),
        };
        let physics_texts = prepared.take_resource::<super::level_prepare::PreparedPhysicsTexts>()
            .map_or_else(Default::default, |texts| texts.0);
        self.audio.prepare(&mut commands, physics_texts);
        check_cancelled()?;
        if let Some(programs) = &mut programs {
            let (frame, source) = super::shadow_updates::prepare_screen_shadow_source(
                &scene,
                programs,
                self.images.assets(),
                &resident_images,
            )?;
            commands.insert_resource(frame);
            commands.insert_resource(source);
        }
        commands.insert_resource(super::area_lights::prepare(&scene, self.images.assets(), &resident_images));
        let (images, shared) = self.images.setup_inputs();
        let base_reader = super::setup::prepare(
            &mut commands,
            &mut scene,
            programs.as_mut(),
            navigation.as_ref(),
            &mut self.meshes,
            images,
            &mut self.probes,
            &mut self.skies,
            &mut self.buffers,
            &self.device,
            &mut self.decals,
            &settings,
            &mut matrices,
            machinery.as_ref(),
            interactions.as_ref(),
            volumes.as_ref(),
            effects.as_ref(),
            Some((shared, base_image_batch_bytes)),
            &resident_images,
            &check_cancelled,
        )?;
        check_cancelled()?;
        match super::ParallelShadowOwner::from_scene(&scene, self.images.assets()) {
            Ok(owner) => commands.insert_resource(owner),
            Err(error) => eprintln!("Parallel shadow owner unavailable: {error}"),
        }
        if let Some(programs) = &mut programs {
            match super::local_shadow_updates::prepare_local_shadow_owner(
                &scene,
                tome.as_ref(),
                &settings,
                self.images.assets(),
                programs,
            ) {
                Ok(owner) => commands.insert_resource(owner),
                Err(error) => eprintln!("Local shadow owner unavailable: {error}"),
            }
        }
        check_cancelled()?;
        let mut image_ids = self.images.assets()
            .ids(|image| image.asset_usage.contains(RenderAssetUsages::RENDER_WORLD));
        // The session LUT was reserved outside this batch, but it still
        // participates in the existing display readiness gate.
        if let Some(cube) = resident_images.default_colour_cube() {
            image_ids.push(cube.id());
        }
        prepared.insert_resource(SceneGpuInputs {
            images: Arc::new(image_ids),
            meshes: Arc::new(
                self.meshes
                    .ids(|mesh| mesh.asset_usage.contains(RenderAssetUsages::RENDER_WORLD)),
            ),
            buffers: Arc::new(
                self.buffers
                    .ids(|buffer| buffer.asset_usage.contains(RenderAssetUsages::RENDER_WORLD)),
            ),
        });
        prepared
            .insert_resource(scene)
            .insert_resource(settings)
            .insert_resource(matrices)
            .insert_resource(SetupPrepared);
        if let Some(value) = programs {
            prepared.insert_resource(value);
        }
        if let Some(value) = tome {
            prepared.insert_resource(value);
        }
        if let Some(value) = navigation {
            prepared.insert_resource(value);
        }
        if let Some(value) = machinery {
            prepared.insert_resource(value);
        }
        if let Some(value) = interactions {
            prepared.insert_resource(value);
        }
        if let Some(value) = volumes {
            prepared.insert_resource(value);
        }
        if let Some(value) = effects {
            prepared.insert_resource(value);
        }
        prepared.on_publish(move |world| {
            let assets = super::load_timing::Span::new("asset publication");
            self.meshes
                .publish(&mut world.resource_mut::<Assets<Mesh>>());
            self.images
                .publish(&mut world.resource_mut::<Assets<Image>>());
            self.buffers
                .publish(&mut world.resource_mut::<Assets<ShaderBuffer>>());
            self.probes
                .publish(&mut world.resource_mut::<Assets<ProbeMaterial>>());
            self.decals
                .publish(&mut world.resource_mut::<Assets<DecalMaterial>>());
            self.skies
                .publish(&mut world.resource_mut::<Assets<SkyMaterial>>());
            self.audio.publish(world);
            drop(assets);
            let entities = super::load_timing::Span::new("entity publication");
            commands.publish(world);
            drop(entities);
            super::load_timing::mark("prepared scene published");
        });
        Ok(base_reader)
    }
}
