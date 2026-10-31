//! The patched `RenderDevice::create_texture_with_data` (one upload buffer)
//! produces the same texels as wgpu's per-subresource writes.
use bevy::{
    prelude::*,
    render::{renderer::{RenderDevice, RenderQueue}, RenderApp},
};
use wgpu::util::DeviceExt;

fn read_back(device: &RenderDevice, queue: &RenderQueue, texture: &wgpu::Texture, desc: &wgpu::TextureDescriptor) -> Vec<u8> {
    let (bw, bh) = desc.format.block_dimensions();
    let block = desc.format.block_copy_size(None).unwrap();
    let mut out = Vec::new();
    for layer in 0..desc.array_layer_count() {
        for mip in 0..desc.mip_level_count {
            let size = desc.mip_level_size(mip).unwrap().physical_size(desc.format);
            let row = (size.width / bw * block) as usize;
            let padded = row.next_multiple_of(256);
            let rows = (size.height / bh) as usize;
            let buffer = device.wgpu_device().create_buffer(&wgpu::BufferDescriptor { label: None, size: (padded * rows) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
            let mut encoder = device.wgpu_device().create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture, mip_level: mip, origin: wgpu::Origin3d { x: 0, y: 0, z: layer }, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded as u32), rows_per_image: Some(rows as u32) } },
                wgpu::Extent3d { width: size.width, height: size.height, depth_or_array_layers: 1 },
            );
            queue.submit([encoder.finish()]);
            buffer.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.wgpu_device().poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let view = buffer.slice(..).get_mapped_range();
            for r in 0..rows {
                out.extend_from_slice(&view[r * padded..r * padded + row]);
            }
        }
    }
    out
}

#[test]
#[ignore = "requires a GPU"]
fn single_buffer_upload_matches_per_subresource_writes() {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(bevy::window::WindowPlugin { primary_window: None, exit_condition: bevy::window::ExitCondition::DontExit, ..default() })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
    );
    app.finish();
    app.cleanup();
    let device = app.sub_app(RenderApp).world().resource::<RenderDevice>().clone();
    let queue = app.sub_app(RenderApp).world().resource::<RenderQueue>().clone();
    // A BC1 chain with non-block-multiple mips, an RGBA8 array, both orders.
    for (format, width, height, layers, mips, order) in [
        (wgpu::TextureFormat::Bc1RgbaUnorm, 100, 36, 1, 7, wgpu::util::TextureDataOrder::LayerMajor),
        (wgpu::TextureFormat::Rgba8Unorm, 37, 21, 3, 5, wgpu::util::TextureDataOrder::LayerMajor),
        (wgpu::TextureFormat::Rgba8Unorm, 37, 21, 3, 5, wgpu::util::TextureDataOrder::MipMajor),
    ] {
        if !device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC) && format.is_compressed() { continue; }
        let desc = wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width, height, depth_or_array_layers: layers },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let (bw, bh) = format.block_dimensions();
        let block = format.block_copy_size(None).unwrap();
        let bytes: usize = (0..mips).map(|m| { let s = desc.mip_level_size(m).unwrap().physical_size(format); (s.width / bw * s.height / bh * block) as usize }).sum::<usize>() * layers as usize;
        let data: Vec<u8> = (0..bytes).map(|i| (i * 2654435761usize >> 13) as u8).collect();
        let patched = device.create_texture_with_data(&queue, &desc, order, &data);
        let reference = device.wgpu_device().create_texture_with_data(&queue, &desc, order, &data);
        assert_eq!(read_back(&device, &queue, &patched, &desc), read_back(&device, &queue, &reference, &desc), "{format:?} {order:?}");
    }
}
