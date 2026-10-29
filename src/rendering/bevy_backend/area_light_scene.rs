//! One scene area-light pool and packet publication shared by opaque, fog and FX.
use super::{
    area_light_pool::AreaLightPool, fog_light_scattering::LocalFogFrame,
    tiled_lighting_gpu::TlfPipelines,
};
use bevy::{
    prelude::*,
    render::{
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        texture::GpuImage,
        Render, RenderApp, RenderSystems,
    },
};
use std::sync::Arc;

const SHADER: Handle<Shader> = bevy::asset::uuid_handle!("d49e4f31-3979-49c0-a9de-f9d69b31a532");
#[derive(Resource, Default)]
pub(super) struct AreaLightScene {
    pub pool: Option<AreaLightPool>,
    pub packets: Arc<Vec<[u32; 28]>>,
    source: Option<Arc<Vec<[u32; 28]>>>,
    camera: Option<Entity>,
}
impl AreaLightScene {
    pub fn matches(&self, frame: &LocalFogFrame) -> bool {
        self.camera == frame.camera
            && self
                .source
                .as_ref()
                .is_some_and(|source| Arc::ptr_eq(source, &frame.lights))
    }
}
const TILE_LIST_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("5f0c3a77-91e2-4d1b-a8c6-2b7e0d4f9e13");
/// The opaque area-light evaluator and tile-list iterator the patched PBR lighting imports.
pub(super) fn register_shaders(app: &mut App) {
    bevy::asset::load_internal_asset!(
        app,
        TILE_LIST_SHADER,
        "../../../assets/shaders/dh2_tlf_list.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        SHADER,
        "../../../assets/shaders/dh2_area_light_opaque.wgsl",
        |source: &str, path| Shader::from_wgsl(
            format!(
                "{source}\n{}",
                include_str!("../../../assets/shaders/dh2_area_light.wgsl")
            ),
            path
        )
    );
}
pub(super) fn register(app: &mut App) {
    register_shaders(app);
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .init_resource::<AreaLightScene>()
            .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources));
    }
}
fn prepare(
    mut scene: ResMut<AreaLightScene>,
    frame: Res<LocalFogFrame>,
    programs: Option<Res<TlfPipelines>>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut bindings: ResMut<bevy::pbr::AreaLightBindings>,
    mut last_error: Local<Option<String>>,
) {
    if scene.matches(&frame) {
        return;
    }
    let result = (|| -> Result<(), String> {
        if frame.camera.is_none() {
            return Err("area lights await a camera publication".into());
        }
        if frame.source_indices.len() != frame.lights.len() {
            return Err("area-light scene identities do not match packets".into());
        }
        let programs = programs
            .as_ref()
            .ok_or("area-light copy pipeline awaits preparation")?;
        let mut packets = frame.lights.as_ref().clone();
        let pool = scene
            .pool
            .get_or_insert_with(|| AreaLightPool::new(&device, &programs.area_copy));
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("shared area-light pool preparation"),
        });
        let revision = pool.copy_revision();
        let copied = pool.prepare(
            &device,
            &queue,
            &mut encoder,
            &images,
            &frame.area_images,
            &mut packets,
        );
        // Preserve successful partial uploads if another image is still pending.
        if pool.copy_revision() != revision {
            queue.submit([encoder.finish()]);
        }
        copied?;
        bindings.texture = Some(pool.view.clone());
        scene.packets = Arc::new(packets);
        scene.source = Some(frame.lights.clone());
        scene.camera = frame.camera;
        Ok(())
    })();
    match result {
        Ok(()) => *last_error = None,
        Err(error) => {
            scene.source = None;
            scene.camera = None;
            if last_error.as_ref() != Some(&error) {
                warn!("Area-light preparation pending: {error}");
                *last_error = Some(error);
            }
        }
    }
}
