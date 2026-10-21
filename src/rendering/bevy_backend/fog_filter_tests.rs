//! Render the actual filtering shader against installed HLSL/D3D11 outputs.
use super::fog_filter::{FogBlurUniform, FogFilter, FogFilterPipelines, FogFilterSources};
use super::fog_tests::check_rgba16_pixels;
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};
pub(super) fn check_filter(device: &RenderDevice, queue: &RenderQueue) {
    let pipelines = FogFilterPipelines::new(device);
    let upload = |format, data: &[u8]| {
        device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("fog filter oracle input"),
                size: Extent3d {
                    width: 8,
                    height: 8,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::TEXTURE_BINDING
                    | TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::COPY_SRC,
                view_formats: &[],
            },
            TextureDataOrder::LayerMajor,
            data,
        )
    };
    let mut color = Vec::new();
    let mut depth = Vec::new();
    let mut mask = Vec::new();
    for y in 0..8 {
        for x in 0..8 {
            for value in [
                x as f32 * 0.05,
                y as f32 * 0.04,
                (x + y) as f32 * 0.03,
                1. - x as f32 * 0.1,
            ] {
                color.extend(half::f16::from_f32(value).to_bits().to_le_bytes());
            }
            let near = 1. + x as f32 * 0.125 + y as f32 * 0.05 + if x >= 4 { 20. } else { 0. };
            for value in [near, near + 0.5] {
                depth.extend(half::f16::from_f32(value).to_bits().to_le_bytes());
            }
            mask.push(((x * 37 + y * 19) % 256) as u8);
        }
    }
    let color_bytes = color;
    let color = upload(TextureFormat::Rgba16Float, &color_bytes);
    let depth = upload(TextureFormat::Rg16Float, &depth);
    let mask = upload(TextureFormat::R8Unorm, &mask);
    let color = color.create_view(&default());
    let depth = depth.create_view(&default());
    let mask = mask.create_view(&default());
    let sampler = |filter| {
        device.create_sampler(&SamplerDescriptor {
            min_filter: filter,
            mag_filter: filter,
            mipmap_filter: match filter {
                FilterMode::Linear => MipmapFilterMode::Linear,
                FilterMode::Nearest => MipmapFilterMode::Nearest,
            },
            ..default()
        })
    };
    let linear = sampler(FilterMode::Linear);
    let point = sampler(FilterMode::Nearest);
    let output = device.create_texture(&TextureDescriptor {
        label: Some("fog filter output"),
        size: Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba16Float,
        usage: TextureUsages::RENDER_ATTACHMENT
            | TextureUsages::COPY_SRC
            | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = output.create_view(&default());
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 8 * 256,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let cases = include_str!("../../../tests/fixtures/fog_blur.txt")
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert_eq!(cases.len(), 1024);
    for group in cases.chunks_exact(64) {
        let first = &group[0];
        let radius = first[0].parse::<f32>().unwrap();
        let gaussian = first[1] == "1";
        let horizontal = first[2] == "1";
        let rays = first[3] == "1";
        let mut settings = crate::platform::game_settings::fog_render_settings("").unwrap();
        settings.blur_radius = radius;
        let mut parameters =
            UniformBuffer::from(FogBlurUniform::new(settings, UVec2::splat(8), horizontal));
        parameters.write_buffer(device, queue);
        let bind = device.create_bind_group(
            None,
            &pipelines.layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: parameters.buffer().unwrap().as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&color),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(&depth),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::TextureView(&mask),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: BindingResource::Sampler(&linear),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: BindingResource::Sampler(&point),
                },
            ],
        );
        let mut encoder = device.create_command_encoder(&default());
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(LinearRgba::NONE.into()),
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
            pass.set_pipeline(pipelines.pipeline(gaussian, rays));
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        check_rgba16_pixels(
            device,
            queue,
            encoder,
            &output,
            &staging,
            group,
            5,
            &format!("radius={radius} gaussian={gaussian} horizontal={horizontal} rays={rays}"),
        );
    }
    let cases = include_str!("../../../tests/fixtures/fog_filter_chain.txt")
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert_eq!(cases.len(), 512);
    let mut filter = FogFilter::new(device);
    for group in cases.chunks_exact(64) {
        let mut settings = crate::platform::game_settings::fog_render_settings("").unwrap();
        settings.blur_radius = group[0][0].parse().unwrap();
        settings.gaussian_blur = group[0][1] == "1";
        let rays = group[0][2] == "1";
        let target = upload(TextureFormat::Rgba16Float, &color_bytes);
        let target_view = target.create_view(&default());
        let mut encoder = device.create_command_encoder(&default());
        filter.record(
            device,
            queue,
            &mut encoder,
            settings,
            FogFilterSources {
                color: &target_view,
                scratch: &view,
                depth: &depth,
                rays: rays.then_some(&mask),
                extent: UVec2::splat(8),
            },
        );
        check_rgba16_pixels(
            device,
            queue,
            encoder,
            &target,
            &staging,
            group,
            4,
            &format!(
                "two passes radius={} gaussian={} rays={rays}",
                settings.blur_radius, settings.gaussian_blur
            ),
        );
    }
}
