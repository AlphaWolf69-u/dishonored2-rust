//! F8 numerical display inputs. Copies run only for an explicit request.
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
            .add_systems(Render, finish.in_set(RenderSystems::Cleanup));
    }
}
impl DisplayCaptureGpu {
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
            TextureFormat::Rg11b10Ufloat => 4,
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
