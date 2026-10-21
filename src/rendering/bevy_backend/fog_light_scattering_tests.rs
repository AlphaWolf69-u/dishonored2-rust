//! Draw the actual opaque scattering pass against the installed-shader oracle.
use super::fog_light_scattering::{LightScatterView, LightScatteringInputs, LightScatteringPass};
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

pub(super) fn check_pass(device: &RenderDevice, queue: &RenderQueue) {
    for shadowed in [false, true] {
        check_variant(device, queue, shadowed);
    }
}

fn check_variant(device: &RenderDevice, queue: &RenderQueue, shadowed: bool) {
    let (fixture, result_start) = if shadowed {
        (
            include_str!("../../../tests/fixtures/fog_scattering_shadow_pass.txt"),
            847,
        )
    } else {
        (
            include_str!("../../../tests/fixtures/fog_scattering_pass.txt"),
            721,
        )
    };
    let words: Vec<u32> = fixture
        .split_whitespace()
        .map(|s| u32::from_str_radix(s, 16).unwrap())
        .collect();
    assert_eq!(words.len(), (result_start + 64) * 4);
    let buffer = |words: &[u32]| {
        device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("scattering oracle records"),
            contents: &words
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .collect::<Vec<_>>(),
            usage: BufferUsages::STORAGE,
        })
    };
    let lights = buffer(&words[..693 * 4]);
    let pool = buffer(&words[693 * 4..693 * 4 + 99]);
    let heads = buffer(&words[718 * 4..721 * 4]);
    let shadows = if shadowed {
        buffer(&words[721 * 4..result_start * 4])
    } else {
        buffer(&[0; 36])
    };
    let make = |format, usage, data: Option<&[u8]>| {
        let desc = TextureDescriptor {
            label: Some("scattering oracle texture"),
            size: Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        };
        if let Some(data) = data {
            device.create_texture_with_data(queue, &desc, TextureDataOrder::LayerMajor, data)
        } else {
            device.create_texture(&desc)
        }
    };
    let depth_data: Vec<u8> = (0..8)
        .flat_map(|y| {
            (0..8).flat_map(move |x| {
                [0.125, 4. + (x + y) as f32 * 0.25]
                    .into_iter()
                    .flat_map(|v| half::f16::from_f32(v).to_le_bytes())
            })
        })
        .collect();
    let first_data: Vec<u8> = (0..8)
        .flat_map(|y| (0..8).map(move |x| ((x * 37 + y * 19) % 256) as u8))
        .collect();
    let second_data: Vec<u8> = (0..8)
        .flat_map(|y| (0..8).map(move |x| (255i32 - x * 17 - y * 23).rem_euclid(256) as u8))
        .collect();
    let depth = make(
        TextureFormat::Rg16Float,
        TextureUsages::TEXTURE_BINDING,
        Some(&depth_data),
    );
    let first = make(
        TextureFormat::R8Unorm,
        TextureUsages::TEXTURE_BINDING,
        Some(&first_data),
    );
    let second = make(
        TextureFormat::R8Unorm,
        TextureUsages::TEXTURE_BINDING,
        Some(&second_data),
    );
    let atlas = device.create_texture(&TextureDescriptor {
        label: Some("scattering comparison atlas"),
        size: Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 2,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Depth32Float,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let atlas_view = atlas.create_view(&TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    let view = LightScatterView {
        world_from_clip: Mat4::IDENTITY,
        camera_far: Vec4::new(0., 0., -2., 2.),
        position_to_view_texture: Vec4::new(0., 0., 0.125, 0.125),
        canvas_scale_jitter: Vec4::new(1., 1., 0., 0.),
        grid: UVec4::new(2, 2, 2, 1),
        black_white: Vec4::ZERO,
    };
    let mut scattering = LightScatteringPass::new(device, TextureFormat::Rgba16Float, view);
    scattering.prepare(
        device,
        queue,
        view,
        LightScatteringInputs {
            lights: &lights,
            pool: &pool,
            heads: &heads,
            shadows: &shadows,
            atlas: &atlas_view,
            depth: &depth.create_view(&default()),
            first_canvas: &first.create_view(&default()),
            second_canvas: &second.create_view(&default()),
        },
    );
    let target = make(
        TextureFormat::Rgba16Float,
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        None,
    );
    let target_view = target.create_view(&default());
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 256 * 8,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&default());
    for (layer, depth) in [(0, 0.25), (1, 0.75)] {
        let view = atlas.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2),
            base_array_layer: layer,
            array_layer_count: Some(1),
            ..default()
        });
        let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("scattering oracle atlas layer"),
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &view,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(depth),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..default()
        });
    }
    {
        let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("scattering additive baseline"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &target_view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(bevy::color::LinearRgba::new(0.1, 0.2, 0.3, 0.625).into()),
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
    }
    scattering.record(&mut encoder, &target_view);
    let expected: Vec<Vec<String>> = (0..64)
        .map(|pixel| {
            (0..4)
                .map(|channel| {
                    let value = if channel == 3 {
                        0.625
                    } else {
                        half::f16::from_f32([0.1, 0.2, 0.3][channel]).to_f32()
                            + f32::from_bits(words[(result_start + pixel) * 4 + channel])
                    };
                    format!("{:08x}", value.to_bits())
                })
                .collect()
        })
        .collect();
    let borrowed: Vec<Vec<&str>> = expected
        .iter()
        .map(|r| r.iter().map(String::as_str).collect())
        .collect();
    super::fog_tests::check_rgba16_pixels(
        device,
        queue,
        encoder,
        &target,
        &staging,
        &borrowed,
        0,
        if shadowed {
            "eight-tap local scattering, per-step omni side and split-spot base side"
        } else {
            "local light scattering, depth.y and 32-light cap"
        },
    );
}
