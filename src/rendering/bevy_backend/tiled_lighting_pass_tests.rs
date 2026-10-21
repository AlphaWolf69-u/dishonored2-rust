//! Exercise the live owner: real separate depth inputs, both list dispatches,
//! GPU head clears, one counter reset, stale-frame clearing and viewport resize.
use super::*;
use crate::tiled_lighting::{PackedLightNode, END_OF_LIGHT_LIST};

pub(in crate::rendering::bevy_backend) fn check_submission(
    device: &RenderDevice,
    queue: &RenderQueue,
    pipelines: &TlfPipelines,
) {
    let drain = || {
        queue.submit([]);
        device
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
    };
    drain();
    let baseline = device
        .wgpu_device()
        .generate_allocator_report()
        .map(|report| report.total_allocated_bytes);
    for _ in 0..3 {
        check_view_lifetime(device, queue, pipelines);
        drain();
        if let (Some(baseline), Some(report)) =
            (baseline, device.wgpu_device().generate_allocator_report())
        {
            // The depth readback deliberately starts at byte 256. DX12 needs
            // an alignment buffer: reclaim both its resource and allocation.
            assert!(
                report.total_allocated_bytes <= baseline,
                "TLF view teardown retained GPU allocations: {baseline} -> {report:?}"
            );
        }
    }
}

fn check_view_lifetime(device: &RenderDevice, queue: &RenderQueue, pipelines: &TlfPipelines) {
    let grid = TileGrid::new(32, 32, 16).unwrap();
    let mut owner = TiledLightingPass::new(device, pipelines, grid);
    assert_eq!(owner.normal_heads.size(), 2 * 2 * 4 * 4);
    assert_eq!(owner.scattering_heads.size(), 2 * 2 * 3 * 4);
    assert_eq!(TiledLightView::min_size().get(), 176);
    let make = || {
        device.create_texture(&TextureDescriptor {
            label: Some("TLF separate depth fixture"),
            size: Extent3d {
                width: 32,
                height: 32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth32Float,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    };
    let opaque = make();
    let alpha = make();
    let opaque_view = opaque.create_view(&default());
    let alpha_view = alpha.create_view(&default());
    let mut light = [0u32; 28];
    light[2] = (-2f32).to_bits();
    light[7] = 1000f32.to_bits();
    light[13] = 1f32.to_bits();
    light[24] = u32::MAX;
    let projection = crate::tome::render_perspective_rows([-1., 1.], [-1., 1.], 1., 10.).unwrap();
    let unproject = crate::tiled_lighting::depth_unprojection(&projection);
    let view = TiledLightView {
        uv_to_ray: Vec4::new(0., 0., 1., 1.),
        view: [Vec4::X, Vec4::Y, Vec4::Z, Vec4::W],
        projection: projection.map(Vec4::from_array),
        ..default()
    };
    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("TLF owner numeric readback"),
        size: 768,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let fixed = [
        owner.lights.id(),
        owner.shadows.id(),
        owner.buffers.packets.id(),
        owner.buffers.pool.id(),
        owner.buffers.counter.id(),
    ];
    let mut uniform_ids = None;
    for active in [true, false, true] {
        owner
            .prepare(
                device,
                queue,
                pipelines,
                view,
                &opaque_view,
                &alpha_view,
                if active {
                    std::slice::from_ref(&light)
                } else {
                    &[]
                },
                &[],
            )
            .unwrap();
        let ids = [
            owner.depth_view.buffer().unwrap().id(),
            owner.normal_view.buffer().unwrap().id(),
            owner.scattering_view.buffer().unwrap().id(),
        ];
        if let Some(previous) = uniform_ids {
            assert_eq!(ids, previous, "TLF uniforms must persist across frames");
        }
        uniform_ids = Some(ids);
        let mut encoder = device.create_command_encoder(&default());
        for (view, depth) in [(&opaque_view, 0.8), (&alpha_view, 0.3)] {
            let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("TLF distinct opaque/alpha depths"),
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(depth),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..default()
            });
        }
        owner.record(&mut encoder, pipelines).unwrap();
        encoder.copy_buffer_to_buffer(&owner.buffers.counter, 0, &staging, 0, 4);
        encoder.copy_buffer_to_buffer(&owner.normal_heads, 0, &staging, 4, 64);
        encoder.copy_buffer_to_buffer(&owner.scattering_heads, 0, &staging, 68, 48);
        encoder.copy_buffer_to_buffer(&owner.buffers.pool, 0, &staging, 116, 32);
        encoder.copy_texture_to_buffer(
            owner.tile_depth.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset: 256,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(2),
                },
            },
            owner.tile_depth.size(),
        );
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = staging.slice(..).get_mapped_range();
        let word =
            |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        assert_eq!(
            word(0),
            if active { 8 } else { 0 },
            "normal and scatter share one counter"
        );
        for (base, types, first, last) in [(4, 4, 0, 4), (68, 3, 4, 8)] {
            for y in 0..2 {
                for kind in 0..types {
                    for x in 0..2 {
                        let head = word(base + (y * types * 2 + kind * 2 + x) * 4);
                        if !active || kind != 0 {
                            assert_eq!(head, END_OF_LIGHT_LIST);
                            continue;
                        }
                        assert!(
                            head >= first && head < last,
                            "pool reset between passes or wrong head stride"
                        );
                        let node = PackedLightNode(word(116 + head as usize * 4));
                        assert_eq!(node.light_index(), 0);
                        assert_eq!(node.next(), END_OF_LIGHT_LIST);
                    }
                }
            }
        }
        let depths: Vec<f32> = [vec![0.8; 256], vec![0.3; 256]].concat();
        let expected = crate::tiled_lighting::reduce_depth_tile(&depths, unproject);
        for y in 0..2 {
            for x in 0..2 {
                for channel in 0..4 {
                    let actual = word(256 + y * 256 + x * 16 + channel * 4);
                    if channel == 3 {
                        assert_eq!(actual, expected[3].to_bits());
                    } else {
                        assert!(
                            (f32::from_bits(actual) - expected[channel]).abs() < 0.00001,
                            "separate opaque/alpha depth channel {channel}"
                        );
                    }
                }
            }
        }
        drop(bytes);
        staging.unmap();
        assert_eq!(
            [
                owner.lights.id(),
                owner.shadows.id(),
                owner.buffers.packets.id(),
                owner.buffers.pool.id(),
                owner.buffers.counter.id()
            ],
            fixed
        );
    }
    let prior_heads = [owner.normal_heads.id(), owner.scattering_heads.id()];
    owner.resize(device, pipelines, TileGrid::new(32, 16, 16).unwrap());
    assert_ne!(
        [owner.normal_heads.id(), owner.scattering_heads.id()],
        prior_heads
    );
    assert_eq!(owner.normal_heads.size(), 2 * 4 * 4);
    assert_eq!(owner.scattering_heads.size(), 2 * 3 * 4);
    assert_eq!(
        [
            owner.lights.id(),
            owner.shadows.id(),
            owner.buffers.packets.id(),
            owner.buffers.pool.id(),
            owner.buffers.counter.id()
        ],
        fixed
    );
    assert!(owner
        .record(&mut device.create_command_encoder(&default()), pipelines)
        .is_err());
    drop(owner);
    // The ignored test ends after replacing its viewport resources. Submit
    // deferred destruction before the app's device/allocator is torn down.
    queue.submit([]);
    device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
}
