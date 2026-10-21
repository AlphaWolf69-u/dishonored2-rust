//! On-demand numerical fog inputs. No image encoding or continuous readback.
use bevy::{
    prelude::*,
    render::{
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_resource::*,
        renderer::RenderDevice,
        Render, RenderApp, RenderSystems,
    },
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Resource, Clone, Default, ExtractResource)]
pub struct FogCaptureRequest(Arc<Mutex<Option<PathBuf>>>);
impl FogCaptureRequest {
    pub fn request(&self, path: PathBuf) {
        *self.0.lock().unwrap() = Some(path);
    }
}
#[derive(Resource, Default)]
pub(super) struct FogCaptureGpu {
    pending: Option<Pending>,
}
struct Pending {
    buffer: Buffer,
    path: PathBuf,
    metadata: String,
    mapped: bool,
    result: Arc<Mutex<Option<Result<(), String>>>>,
}
pub(super) fn register(app: &mut App) {
    app.init_resource::<FogCaptureRequest>()
        .add_plugins(ExtractResourcePlugin::<FogCaptureRequest>::default());
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .init_resource::<FogCaptureGpu>()
            .add_systems(Render, finish.in_set(RenderSystems::Cleanup));
    }
}
impl FogCaptureGpu {
    pub(super) fn record(
        &mut self,
        request: &FogCaptureRequest,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        view: super::fog::FogView,
        frame: &super::fog::FogFrame,
        depth: &TextureView,
        atlas: &super::shadow_pass::ShadowSceneFrame,
        atlas_image: &bevy::render::texture::GpuImage,
        parallel: &Buffer,
        fog: &super::fog_pass::FogPass,
    ) -> Result<(), String> {
        if self.pending.is_some() {
            return Ok(());
        }
        let Some(path) = request.0.lock().unwrap().take() else {
            return Ok(());
        };
        let full = fog.targets.extents.full;
        let shadow = fog.targets.extents.shadow;
        let size = atlas_image.texture.size();
        let (staging, counts) = copy_sources(
            device,
            encoder,
            full,
            shadow,
            size,
            depth,
            &atlas_image.texture_view,
            &fog.targets.shadow.view,
            parallel,
        )?;
        let metadata = format!("Fog GPU capture v1\nBinary: little-endian u32 words, contiguous depth/full, atlas/layer-major, fog-shadow, sampling-buffer planes; depth/shadow words are IEEE float32 bits.\ncounts={counts:?}\nfull={full:?}\natlas_extent={size:?}\nshadow={shadow:?}\nfog_revision={}\natlas_revision={}\nworld_from_clip_cols={:?}\neye={:?}\nview={view:?}\nsettings={:?}\nparameters={:?}\nprojected={} temporal={} projected_extent={}\n",frame.revision,atlas.revision,view.world_from_clip.to_cols_array(),view.eye.to_array(),frame.settings,frame.parameters,frame.shadow.projected,frame.shadow.temporal,frame.shadow.extent);
        let metadata = format!("{metadata}ray_settings={:?}\n", frame.rays);
        self.pending = Some(Pending {
            buffer: staging,
            path,
            metadata,
            mapped: false,
            result: default(),
        });
        Ok(())
    }
}
fn copy_sources(
    device: &RenderDevice,
    encoder: &mut CommandEncoder,
    full: UVec2,
    shadow: UVec2,
    size: Extent3d,
    depth: &TextureView,
    atlas: &TextureView,
    fog_shadow: &TextureView,
    parallel: &Buffer,
) -> Result<(Buffer, Vec<u32>), String> {
    let counts = [
        full.x.checked_mul(full.y),
        size.width
            .checked_mul(size.height)
            .and_then(|v| v.checked_mul(size.depth_or_array_layers)),
        shadow.x.checked_mul(shadow.y),
        Some((parallel.size() / 4) as u32),
    ];
    let counts: Vec<u32> = counts
        .into_iter()
        .collect::<Option<_>>()
        .ok_or("fog numeric capture extent overflow")?;
    let total = counts
        .iter()
        .try_fold(0u32, |a, b| a.checked_add(*b))
        .ok_or("fog numeric capture length overflow")?;
    let bytes = u64::from(total) * 4;
    if bytes > u64::from(device.limits().max_storage_buffer_binding_size)
        || bytes > device.limits().max_buffer_size
    {
        return Err(format!(
            "fog numeric capture {bytes} bytes exceeds adapter buffer limit"
        ));
    }
    let output = device.create_buffer(&BufferDescriptor {
        label: Some("F8 fog numeric sources"),
        size: bytes,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("F8 fog numeric staging"),
        size: bytes,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let words = [
        full.x,
        full.y,
        size.width,
        size.height,
        size.depth_or_array_layers,
        shadow.x,
        shadow.y,
        total,
    ];
    let uniform = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &words
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
        usage: BufferUsages::UNIFORM,
    });
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("F8 fog numeric copy"),
        source: ShaderSource::Wgsl(SHADER.into()),
    });
    let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: Some("F8 fog numeric copy"),
        layout: None,
        module: &module,
        entry_point: Some("capture"),
        compilation_options: default(),
        cache: None,
    });
    let binding = device.create_bind_group(
        None,
        &pipeline.get_bind_group_layout(0).into(),
        &[
            BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(depth),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::TextureView(atlas),
            },
            BindGroupEntry {
                binding: 3,
                resource: BindingResource::TextureView(fog_shadow),
            },
            BindGroupEntry {
                binding: 4,
                resource: parallel.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 5,
                resource: output.as_entire_binding(),
            },
        ],
    );
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("F8 fog numeric sources"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &binding, &[]);
        pass.dispatch_workgroups(256, total.div_ceil(16384), 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, bytes);
    Ok((staging, counts))
}
// Cleanup runs after render submission. Mapping never waits on the game thread.
fn finish(mut gpu: ResMut<FogCaptureGpu>, device: Res<RenderDevice>) {
    let Some(pending) = &mut gpu.pending else {
        return;
    };
    if !pending.mapped {
        let result = pending.result.clone();
        pending
            .buffer
            .slice(..)
            .map_async(MapMode::Read, move |value| {
                *result.lock().unwrap() = Some(value.map_err(|e| e.to_string()));
            });
        pending.mapped = true;
    }
    let _ = device.poll(PollType::Poll);
    let result = pending.result.lock().unwrap().take();
    if let Some(result) = result {
        let pending = gpu.pending.take().unwrap();
        if let Err(error) = result {
            warn!("Fog numeric capture failed: {error}");
            return;
        }
        let bytes = pending.buffer.slice(..).get_mapped_range().to_vec();
        pending.buffer.unmap();
        bevy::tasks::IoTaskPool::get()
            .spawn(async move {
                let write = || -> std::io::Result<()> {
                    std::fs::write(pending.path.with_extension("bin"), bytes)?;
                    std::fs::write(pending.path.with_extension("txt"), pending.metadata)
                };
                match write() {
                    Ok(()) => info!("Fog numeric capture saved: {}", pending.path.display()),
                    Err(error) => warn!("Fog numeric capture write failed: {error}"),
                }
            })
            .detach();
    }
}
const SHADER: &str = r#"
struct Sizes { full:vec2<u32>, atlas:vec2<u32>, layers:u32, shadow_width:u32, shadow_height:u32, total:u32, }
@group(0) @binding(0) var<uniform> sizes:Sizes;
@group(0) @binding(1) var depth:texture_depth_2d;
@group(0) @binding(2) var atlas:texture_depth_2d_array;
@group(0) @binding(3) var shadow:texture_2d<f32>;
@group(0) @binding(4) var<storage,read> parallel:array<u32>;
@group(0) @binding(5) var<storage,read_write> output:array<u32>;
@compute @workgroup_size(64) fn capture(@builtin(global_invocation_id) id:vec3<u32>) {
    let index=id.x+id.y*16384u;
    if index>=sizes.total {return;}
    var i=index;
    let n=sizes.full.x*sizes.full.y;
    if i<n {output[index]=bitcast<u32>(textureLoad(depth,vec2<i32>(vec2<u32>(i%sizes.full.x,i/sizes.full.x)),0));return;}
    i-=n;let area=sizes.atlas.x*sizes.atlas.y;
    if i<area*sizes.layers {output[index]=bitcast<u32>(textureLoad(atlas,vec2<i32>(vec2<u32>(i%sizes.atlas.x,(i/sizes.atlas.x)%sizes.atlas.y)),i32(i/area),0));return;}
    i-=area*sizes.layers;
    if i<sizes.shadow_width*sizes.shadow_height {output[index]=bitcast<u32>(textureLoad(shadow,vec2<i32>(vec2<u32>(i%sizes.shadow_width,i/sizes.shadow_width)),0).x);return;}
    i-=sizes.shadow_width*sizes.shadow_height;output[index]=parallel[i];
}
"#;

#[cfg(test)]
#[path = "fog_capture/tests.rs"]
mod tests;
