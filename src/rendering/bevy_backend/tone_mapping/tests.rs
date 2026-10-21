use super::*;
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        RenderApp,
    },
};

#[test]
#[ignore = "requires GPU; original filmic curve, colour-cube lookup and gamma"]
fn final_colour_matches_installed_mapping_rules() {
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
    app.finish();
    app.cleanup();
    let world = app.sub_app(RenderApp).world();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let pixels: Vec<u8> = (0..32)
        .flat_map(|z| {
            (0..32).flat_map(move |y| {
                (0..32).flat_map(move |x| {
                    [
                        (x * 17 + y * 3 + z * 7 + 74) % 256,
                        (x * 5 + y * 19 + z * 11 + 58) % 256,
                        (x * 13 + y * 7 + z * 23 + 82) % 256,
                        (x + y + z + 34) % 256,
                    ]
                    .map(|v| v as u8)
                })
            })
        })
        .collect();
    let texture = device.create_texture_with_data(
        queue,
        &TextureDescriptor {
            label: None,
            size: Extent3d {
                width: 32,
                height: 32,
                depth_or_array_layers: 32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D3,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        TextureDataOrder::MipMajor,
        &pixels,
    );
    let cube = texture.create_view(&default());
    let linear = device.create_sampler(&SamplerDescriptor {
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        ..default()
    });
    let buffer = |size, usage| {
        device.create_buffer(&BufferDescriptor {
            label: None,
            size,
            usage,
            mapped_at_creation: false,
        })
    };
    let params = buffer(80, BufferUsages::UNIFORM | BufferUsages::COPY_DST);
    let queries = buffer(64 * 16, BufferUsages::STORAGE | BufferUsages::COPY_DST);
    let output = buffer(64 * 16, BufferUsages::STORAGE | BufferUsages::COPY_SRC);
    let staging = buffer(64 * 16, BufferUsages::MAP_READ | BufferUsages::COPY_DST);
    let adaptation = buffer(32, BufferUsages::STORAGE | BufferUsages::COPY_DST);
    let raster_staging = buffer(256 * 8, BufferUsages::MAP_READ | BufferUsages::COPY_DST);
    let image = |usage| {
        device.create_texture(&TextureDescriptor {
            label: None,
            size: Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba32Float,
            usage,
            view_formats: &[],
        })
    };
    let hdr = image(TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST);
    let hdr_view = hdr.create_view(&default());
    let display_output = image(TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC);
    let display_view = display_output.create_view(&default());
    let mut display_pass = ToneMappingPass::new(device, TextureFormat::Rgba32Float);
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("standard display transform comparison"),
        source: ShaderSource::Wgsl(
            format!(
                "{SHADER_SOURCE}\n{}",
                r#"
@group(0) @binding(0) var<uniform> params:ToneMappingParameters;
@group(0) @binding(1) var cube:texture_3d<f32>;
@group(0) @binding(2) var<storage,read> queries:array<vec4<f32>>;
@group(0) @binding(3) var<storage,read_write> result:array<vec4<f32>>;
@group(0) @binding(4) var linear_clamp:sampler;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    result[id.x]=vec4(standard_display_colour(params,queries[id.x].xyz,cube,linear_clamp),1.0);
}
"#
            )
            .into(),
        ),
    });
    let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: default(),
        cache: None,
    });
    let group = device.create_bind_group(
        None,
        &pipeline.get_bind_group_layout(0).into(),
        &[
            BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(&cube),
            },
            BindGroupEntry {
                binding: 2,
                resource: queries.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 3,
                resource: output.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 4,
                resource: BindingResource::Sampler(&linear),
            },
        ],
    );
    let mut lines = include_str!("../../../../tests/fixtures/display_colour.txt").lines();
    let mut cases = 0;
    let mut maximum = 0f32;
    let mut raster_maximum = 0f32;
    while let Some(header) = lines.next() {
        let h: Vec<_> = header.split_whitespace().collect();
        assert_eq!(h.len(), 11);
        let bits = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let display: Vec<u8> = h[8]
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        let display: Vec<f32> = display
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let p = ToneMappingParameters {
            curve: crate::rendering::tone_mapping::ToneCurve::new(
                crate::rendering::tone_mapping::ToneSettings {
                    overbright: bits(h[2]),
                    black: bits(h[3]),
                    white: bits(h[4]),
                    junction: bits(h[5]),
                    toe: bits(h[6]),
                    shoulder: bits(h[7]),
                },
            ),
            gamma: display[0],
            brightness: display[1],
            exposure: display[2],
            use_curve: h[9] == "1",
            use_colour_cube: h[10] == "1",
        };
        let rows: Vec<Vec<u32>> = (0..64)
            .map(|_| {
                lines
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .map(|v| u32::from_str_radix(v, 16).unwrap())
                    .collect()
            })
            .collect();
        assert!(rows.iter().all(|v| v.len() == 8));
        let query_bytes: Vec<u8> = rows
            .iter()
            .flat_map(|v| v[..4].iter().flat_map(|x| x.to_le_bytes()))
            .collect();
        queue.write_buffer(&params, 0, &p.uniform_bytes());
        queue.write_buffer(
            &queries,
            0,
            &rows
                .iter()
                .flat_map(|v| v[..4].iter().flat_map(|x| x.to_le_bytes()))
                .collect::<Vec<_>>(),
        );
        let mut encoder = device.create_command_encoder(&default());
        queue.write_texture(
            hdr.as_image_copy(),
            &query_bytes,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(128),
                rows_per_image: Some(8),
            },
            Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 1,
            },
        );
        let engine = [1., 139.26, 0.5, 3.][cases % 4];
        queue.write_buffer(
            &adaptation,
            0,
            &[engine, 0., 0., 0., 0., 0., 0., 0.]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let mut raster_parameters = p;
        raster_parameters.exposure /= engine;
        display_pass.record(
            device,
            queue,
            &mut encoder,
            &raster_parameters,
            &hdr_view,
            &cube,
            &adaptation,
            &display_view,
        );
        encoder.copy_texture_to_buffer(
            display_output.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &raster_staging,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(8),
                },
            },
            Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 1,
            },
        );
        {
            let mut pass = encoder.begin_compute_pass(&default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, 64 * 16);
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        let (raster_tx, raster_rx) = std::sync::mpsc::channel();
        raster_staging
            .slice(..)
            .map_async(MapMode::Read, move |v| raster_tx.send(v).unwrap());
        staging
            .slice(..)
            .map_async(MapMode::Read, move |v| tx.send(v).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        raster_rx.recv().unwrap().unwrap();
        let bytes = staging.slice(..).get_mapped_range();
        let raster_bytes = raster_staging.slice(..).get_mapped_range();
        for (i, row) in rows.iter().enumerate() {
            for c in 0..4 {
                let at = i * 16 + c * 4;
                let actual = f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
                let expected = f32::from_bits(row[4 + c]);
                let error = (actual - expected).abs();
                maximum = maximum.max(error);
                assert!(
                    actual.is_finite() && expected.is_finite() && error < 0.0005,
                    "case{cases} row{i} channel{c}: {actual} != {expected}"
                );
                let at = (i / 8) * 256 + (i % 8) * 16 + c * 4;
                let raster = f32::from_le_bytes(raster_bytes[at..at + 4].try_into().unwrap());
                let error = (raster - expected).abs();
                raster_maximum = raster_maximum.max(error);
                assert!(
                    raster.is_finite() && error < 0.0005,
                    "raster case{cases} row{i} channel{c}: {raster} != {expected}, engine={engine}"
                );
            }
        }
        drop(bytes);
        staging.unmap();
        drop(raster_bytes);
        raster_staging.unmap();
        cases += 1;
    }
    assert_eq!(cases, 96);
    eprintln!("96 display frames / 24576 channels, max absolute error {maximum}");
    eprintln!("96 raster display frames / 24576 channels, max absolute error {raster_maximum}");
}
