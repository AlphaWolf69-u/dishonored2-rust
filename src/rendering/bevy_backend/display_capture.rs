//! F8 numerical display inputs. Copies run only for an explicit request.
use bevy::{
    camera::NormalizedRenderTarget,
    prelude::*,
    render::{
        camera::ExtractedCamera,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_resource::*,
        renderer::RenderDevice,
        renderer::{RenderGraph, RenderGraphSystems, RenderQueue},
        view::{window::ExtractedWindows, ViewTarget},
        Render, RenderApp, RenderSystems,
    },
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[path = "display_capture/motion.rs"]
mod motion;

#[derive(Resource, Clone, Default, ExtractResource)]
pub struct DisplayCaptureRequest(Arc<Mutex<Option<PathBuf>>>);
impl DisplayCaptureRequest {
    pub fn request(&self, path: PathBuf) {
        *self.0.lock().unwrap() = Some(path);
    }
    pub(super) fn is_pending(&self) -> bool {
        self.0.lock().unwrap().is_some()
    }
}
#[derive(Resource, Default)]
pub(super) struct DisplayCaptureGpu {
    pending: Option<Pending>,
    motion: motion::MotionCapture,
    window_copy: Option<Texture>,
}
struct Pending {
    buffer: Buffer,
    path: PathBuf,
    metadata: String,
    mapped: bool,
    result: Arc<Mutex<Option<Result<(), String>>>>,
}
pub(super) fn register(app: &mut App) {
    app.init_resource::<DisplayCaptureRequest>()
        .add_plugins(ExtractResourcePlugin::<DisplayCaptureRequest>::default());
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .init_resource::<DisplayCaptureGpu>()
            .add_systems(
                RenderGraph,
                record_window_history.in_set(RenderGraphSystems::Finish),
            )
            .add_systems(Render, finish.in_set(RenderSystems::Cleanup));
    }
}
fn record_window_history(
    mut gpu: ResMut<DisplayCaptureGpu>,
    windows: Res<ExtractedWindows>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    targets: Query<(&ViewTarget, &ExtractedCamera)>,
) {
    if !gpu.motion_enabled() {
        return;
    }
    let Some(window) = windows.primary.and_then(|entity| windows.get(&entity)) else {
        return;
    };
    let Some(surface) = window
        .swap_chain_texture
        .as_ref()
        .filter(|_| window.drawable)
    else {
        return;
    };
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("final window history"),
    });
    let size = surface.texture.size();
    let usage = surface.texture.usage();
    // F8 temporarily renders into a sampleable screenshot attachment. Its
    // identity blit to the swapchain runs after this graph schedule.
    let redirected = targets.iter().filter(|(_, camera)| {
        matches!(camera.target, Some(NormalizedRenderTarget::Window(w)) if w.entity() == window.entity)
    }).filter_map(|(target, _)| target.out_texture()).find(|view| {
        window.swap_chain_texture_view.as_ref().is_some_and(|surface_view| view.id() != surface_view.id())
    });
    let view: TextureView = if let Some(redirected) = redirected {
        redirected.clone()
    } else if usage.contains(TextureUsages::TEXTURE_BINDING) {
        surface
            .texture
            .create_view(&TextureViewDescriptor {
                format: window.swap_chain_texture_view_format,
                ..default()
            })
            .into()
    } else if usage.contains(TextureUsages::COPY_SRC) {
        // DX12 surfaces expose copying but not shader sampling in wgpu.
        // One reusable copy is allocated only for the requested diagnostic.
        let format = window
            .swap_chain_texture_view_format
            .unwrap_or(surface.texture.format());
        if gpu
            .window_copy
            .as_ref()
            .is_none_or(|t| t.size() != size || t.format() != format)
        {
            gpu.window_copy = Some(device.create_texture(&TextureDescriptor {
                label: Some("window history sampling copy"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            }));
        }
        let copy = gpu.window_copy.as_ref().unwrap();
        encoder.copy_texture_to_texture(
            surface.texture.as_image_copy(),
            copy.as_image_copy(),
            size,
        );
        copy.create_view(&default())
    } else {
        return;
    };
    if gpu.motion.record_window(
        &device,
        &queue,
        &mut encoder,
        view,
        UVec2::new(surface.texture.width(), surface.texture.height()),
    ) {
        queue.submit([encoder.finish()]);
    }
}
impl DisplayCaptureGpu {
    pub(super) fn motion_enabled(&self) -> bool {
        self.motion.enabled()
    }

    pub(super) fn record_motion(
        &mut self,
        request: &DisplayCaptureRequest,
        device: &RenderDevice,
        queue: &bevy::render::renderer::RenderQueue,
        encoder: &mut CommandEncoder,
        textures: [&Texture; 3],
        exposure: &Buffer,
        controls: [f32; 4],
    ) {
        self.motion.record(
            request, device, queue, encoder, textures, exposure, controls,
        );
    }

    pub(super) fn record(
        &mut self,
        request: &DisplayCaptureRequest,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        planes: &[(&str, &Texture)],
        exposure: &Buffer,
        details: String,
    ) -> Result<(), String> {
        if self.pending.is_some() {
            return Ok(());
        }
        let Some(path) = request.0.lock().unwrap().take() else {
            return Ok(());
        };
        let (buffer, metadata) = copy_sources(device, encoder, planes, exposure)?;
        self.pending = Some(Pending {
            buffer,
            path,
            metadata: format!("{metadata}{details}"),
            mapped: false,
            result: default(),
        });
        Ok(())
    }
}
fn copy_sources(
    device: &RenderDevice,
    encoder: &mut CommandEncoder,
    planes: &[(&str, &Texture)],
    exposure: &Buffer,
) -> Result<(Buffer, String), String> {
    let mut total = 0u64;
    let mut copies = Vec::with_capacity(planes.len());
    let mut metadata=String::from("Display GPU capture v1\nBinary: little-endian texture storage with padded rows; final 32 bytes are the exact GPU exposure record.\n");
    for (name, texture) in planes {
        let bytes = match texture.format() {
            TextureFormat::Rgba16Float => 8,
            TextureFormat::Rgba32Float => 16,
            TextureFormat::Rg11b10Ufloat | TextureFormat::R32Float => 4,
            TextureFormat::Rg32Float => 8,
            TextureFormat::Rg16Float => 4,
            TextureFormat::R16Float | TextureFormat::Rg8Unorm => 2,
            TextureFormat::R8Unorm => 1,
            format => return Err(format!("display capture unsupported format {format:?}")),
        };
        if texture.sample_count() != 1 || !texture.usage().contains(TextureUsages::COPY_SRC) {
            return Err(format!(
                "display capture {name} is not a resolved copy source"
            ));
        }
        let row = texture
            .width()
            .checked_mul(bytes)
            .and_then(|v| v.checked_add(255))
            .map(|v| v / 256 * 256)
            .ok_or("display capture row overflow")?;
        let size = u64::from(row)
            * u64::from(texture.height())
            * u64::from(texture.depth_or_array_layers());
        metadata.push_str(&format!(
            "plane={name} offset={total} row_bytes={row} extent={:?} format={:?}\n",
            texture.size(),
            texture.format()
        ));
        copies.push((total, row, *texture));
        total = total
            .checked_add(size)
            .ok_or("display capture size overflow")?;
    }
    let exposure_offset = total;
    total = total
        .checked_add(32)
        .ok_or("display capture exposure overflow")?;
    if exposure.size() < 32
        || !exposure.usage().contains(BufferUsages::COPY_SRC)
        || total > device.limits().max_buffer_size
    {
        return Err("display capture buffer exceeds its source/adapter limits".into());
    }
    metadata.push_str(&format!(
        "exposure_offset={exposure_offset} total_bytes={total}\n"
    ));
    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("F8 display numeric staging"),
        size: total,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    for (offset, row, texture) in copies {
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(texture.height()),
                },
            },
            texture.size(),
        );
    }
    encoder.copy_buffer_to_buffer(exposure, 0, &staging, exposure_offset, 32);
    Ok((staging, metadata))
}
// Render submission precedes Cleanup; never wait for the GPU on the game thread.
fn finish(mut gpu: ResMut<DisplayCaptureGpu>, device: Res<RenderDevice>) {
    let gpu = &mut *gpu;
    finish_pending(&mut gpu.pending, &device);
    finish_pending(&mut gpu.motion.pending, &device);
}

fn finish_pending(slot: &mut Option<Pending>, device: &RenderDevice) {
    let Some(pending) = slot else {
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
        let pending = slot.take().unwrap();
        if let Err(error) = result {
            warn!("Display numeric capture failed: {error}");
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
                    Ok(()) => info!("Display numeric capture saved: {}", pending.path.display()),
                    Err(error) => warn!("Display numeric capture write failed: {error}"),
                }
            })
            .detach();
    }
}
#[cfg(test)]
#[path = "display_capture/tests.rs"]
mod tests;
