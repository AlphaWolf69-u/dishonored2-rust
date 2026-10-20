//! Shader-buffer upload ordering and lifetime checks without game assets.
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::{
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        storage::{GpuShaderBuffer, ShaderBuffer, ShaderBufferUploadBatch},
        RenderApp,
    },
};

#[test]
#[ignore = "requires a GPU; checks batched uploads, in-flight reuse, resizing and removal"]
fn shader_buffer_updates_preserve_gpu_data_and_order() {
    for batched in [false, true] {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
        );
        if batched {
            app.sub_app_mut(RenderApp)
                .init_resource::<ShaderBufferUploadBatch>();
        }
        app.finish();
        app.cleanup();
        let device = app
            .sub_app(RenderApp)
            .world()
            .resource::<RenderDevice>()
            .clone();
        let queue = app
            .sub_app(RenderApp)
            .world()
            .resource::<RenderQueue>()
            .clone();
        let sizes = [4usize, 256, 4096, 65536];
        let mut handles: Vec<_> = (0..16)
            .map(|i| {
                app.world_mut()
                    .resource_mut::<Assets<ShaderBuffer>>()
                    .add(ShaderBuffer::new(
                        &vec![0; sizes[i % 4]],
                        RenderAssetUsages::default(),
                    ))
            })
            .collect();
        let gpu_owned = app
            .world_mut()
            .resource_mut::<Assets<ShaderBuffer>>()
            .add(ShaderBuffer::with_size(16, RenderAssetUsages::default()));
        for _ in 0..3 {
            app.update();
        }
        let gpu_buffer = |app: &App, handle: &Handle<ShaderBuffer>| {
            app.sub_app(RenderApp)
                .world()
                .resource::<RenderAssets<GpuShaderBuffer>>()
                .get(handle.id())
                .unwrap()
                .buffer
                .clone()
        };
        let ids: Vec<_> = handles.iter().map(|h| gpu_buffer(&app, h).id()).collect();
        queue.write_buffer(&gpu_buffer(&app, &gpu_owned), 0, &[0x5a; 16]);
        let previous_gpu_id = gpu_buffer(&app, &gpu_owned).id();
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some("shader buffer epochs readback"),
            size: 32 * 16 * 8 + 32,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        // No per-frame waits: multiple epochs may be in flight while chunks recycle.
        for epoch in 0..32u32 {
            {
                let mut assets = app.world_mut().resource_mut::<Assets<ShaderBuffer>>();
                for (i, h) in handles.iter().enumerate() {
                    let word = ((epoch << 16) | i as u32).to_le_bytes();
                    let mut data = vec![0; sizes[i % 4]];
                    for chunk in data.chunks_exact_mut(4) {
                        chunk.copy_from_slice(&word);
                    }
                    assets.get_mut(h).unwrap().data = Some(data);
                }
                if epoch == 8 {
                    assets.get_mut(&gpu_owned).unwrap().resize_in_place(32);
                }
            }
            app.update();
            // An independent consumer submitted immediately after preparation must
            // observe this epoch, rather than the next upload or the previous one.
            let mut encoder = device.create_command_encoder(&default());
            for (i, h) in handles.iter().enumerate() {
                let buffer = gpu_buffer(&app, h);
                assert_eq!(buffer.id(), ids[i]);
                let offset = (epoch as u64 * 16 + i as u64) * 8;
                encoder.copy_buffer_to_buffer(&buffer, 0, &readback, offset, 4);
                encoder.copy_buffer_to_buffer(
                    &buffer,
                    (sizes[i % 4] - 4) as u64,
                    &readback,
                    offset + 4,
                    4,
                );
            }
            queue.submit([encoder.finish()]);
        }
        let resized = gpu_buffer(&app, &gpu_owned);
        assert_ne!(resized.id(), previous_gpu_id);
        let mut encoder = device.create_command_encoder(&default());
        encoder.copy_buffer_to_buffer(&resized, 0, &readback, 32 * 16 * 8, 32);
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(MapMode::Read, move |result| tx.send(result).unwrap());
        device.poll(PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range();
        for epoch in 0..32usize {
            for i in 0..16 {
                for offset in [(epoch * 16 + i) * 8, (epoch * 16 + i) * 8 + 4] {
                    assert_eq!(
                        u32::from_le_bytes(mapped[offset..offset + 4].try_into().unwrap()),
                        ((epoch as u32) << 16) | i as u32
                    );
                }
            }
        }
        assert_eq!(&mapped[4096..4112], &[0x5a; 16]);
        assert_eq!(&mapped[4112..4128], &[0; 16]);
        drop(mapped);
        readback.unmap();
        // CPU replacement with a new size must still allocate/upload normally.
        app.world_mut()
            .resource_mut::<Assets<ShaderBuffer>>()
            .get_mut(&handles[0])
            .unwrap()
            .data = Some(vec![0x37; 32]);
        app.update();
        let replacement = gpu_buffer(&app, &handles[0]);
        assert_ne!(replacement.id(), ids[0]);
        let mut encoder = device.create_command_encoder(&default());
        encoder.copy_buffer_to_buffer(&replacement, 0, &readback, 0, 32);
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(MapMode::Read, move |result| tx.send(result).unwrap());
        device.poll(PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range();
        assert_eq!(&mapped[..32], &[0x37; 32]);
        drop(mapped);
        readback.unmap();
        let removed = handles.remove(0);
        let removed_id = removed.id();
        drop(removed);
        for _ in 0..3 {
            app.update();
        }
        assert!(app
            .sub_app(RenderApp)
            .world()
            .resource::<RenderAssets<GpuShaderBuffer>>()
            .get(removed_id)
            .is_none());
    }
}
