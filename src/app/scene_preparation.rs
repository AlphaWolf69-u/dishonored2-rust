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

pub(super) struct ScenePreparation {
    meshes: AssetBatch<Mesh>,
    images: AssetBatch<Image>,
    probes: AssetBatch<ProbeMaterial>,
    decals: AssetBatch<DecalMaterial>,
    skies: AssetBatch<SkyMaterial>,
    buffers: AssetBatch<ShaderBuffer>,
    allocator: bevy::ecs::entity::RemoteAllocator,
    device: RenderDevice,
    shared_images: super::texture_streaming::SharedImages,
    audio: super::audio::AudioPreparation,
}
impl ScenePreparation {
    pub fn new(world: &mut World) -> Self {
        Self {
            meshes: AssetBatch::new(world.resource::<Assets<Mesh>>()),
            images: AssetBatch::new(world.resource::<Assets<Image>>()),
            probes: AssetBatch::new(world.resource::<Assets<ProbeMaterial>>()),
            decals: AssetBatch::new(world.resource::<Assets<DecalMaterial>>()),
            skies: AssetBatch::new(world.resource::<Assets<SkyMaterial>>()),
            buffers: AssetBatch::new(world.resource::<Assets<ShaderBuffer>>()),
            allocator: world.entity_allocator().build_remote_allocator(),
            device: world.resource::<RenderDevice>().clone(),
            shared_images: world
                .resource::<super::texture_streaming::SharedImages>()
                .clone(),
            audio: super::audio::AudioPreparation::take(world),
        }
    }
    pub fn prepare(mut self, prepared: &mut super::loading_screen::PreparedLevel) {
        let mut scene = prepared
            .take_resource::<SceneData>()
            .expect("prepared scene");
        let mut programs = prepared.take_resource::<ProgramCacheSource>();
        let navigation = prepared.take_resource::<NavigationWorld>();
        let tome = prepared.take_resource::<TomeWorld>();
        let settings = prepared
            .take_resource::<ViewSettings>()
            .expect("prepared view settings");
        let machinery = prepared.take_resource::<super::machinery::AmbientMachinery>();
        let volumes = prepared.take_resource::<super::state::EnvironmentVolumes>();
        let effects = prepared
            .take_resource::<crate::rendering::bevy_backend::world_effects::WorldEffectDefinitions>(
            );
        let mut matrices = super::world_instances::WorldMatrices::default();
        let mut commands = super::setup_commands::SetupCommands::Prepared {
            allocator: self.allocator,
            queue: Default::default(),
        };
        self.audio.prepare(&mut commands);
        commands.insert_resource(super::area_lights::prepare(&scene, &mut self.images));
        super::setup::prepare(
            &mut commands,
            &mut scene,
            programs.as_mut(),
            navigation.as_ref(),
            &mut self.meshes,
            &mut self.images,
            &mut self.probes,
            &mut self.skies,
            &mut self.buffers,
            &self.device,
            &mut self.decals,
            &settings,
            &mut matrices,
            machinery.as_ref(),
            volumes.as_ref(),
            effects.as_ref(),
            Some(&self.shared_images),
        );
        match super::ParallelShadowOwner::from_scene(&scene, &mut self.images) {
            Ok(owner) => commands.insert_resource(owner),
            Err(error) => eprintln!("Parallel shadow owner unavailable: {error}"),
        }
        if let Some(programs) = &mut programs {
            match super::local_shadow_updates::prepare_local_shadow_owner(
                &scene,
                tome.as_ref(),
                &settings,
                &mut self.images,
                programs,
            ) {
                Ok(owner) => commands.insert_resource(owner),
                Err(error) => eprintln!("Local shadow owner unavailable: {error}"),
            }
        }
        prepared.insert_resource(SceneGpuInputs {
            images: Arc::new(
                self.images
                    .ids(|image| image.asset_usage.contains(RenderAssetUsages::RENDER_WORLD)),
            ),
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
        if let Some(value) = volumes {
            prepared.insert_resource(value);
        }
        if let Some(value) = effects {
            prepared.insert_resource(value);
        }
        prepared.on_publish(move |world| {
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
            commands.publish(world);
            super::load_timing::mark("prepared scene published");
        });
    }
}
