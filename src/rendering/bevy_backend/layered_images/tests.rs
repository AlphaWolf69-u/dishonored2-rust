use super::*;
use bevy::{prelude::*, render::RenderApp};

fn gpu_app() -> App {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(bevy::render::RenderPlugin {
                render_creation: bevy::render::settings::RenderCreation::Automatic(Box::new(
                    bevy::render::settings::WgpuSettings {
                        features: bevy::render::settings::WgpuSettings::default().features
                            | WgpuFeatures::TEXTURE_COMPRESSION_BC
                            | WgpuFeatures::FLOAT32_FILTERABLE,
                        ..default()
                    },
                )),
                ..default()
            })
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
    app
}

#[test]
#[ignore = "requires GPU and installed array; numerical uploads and source frame selection"]
fn texture_arrays_keep_mips_and_float_frame_selection() {
    let app = gpu_app();
    let world = app.sub_app(RenderApp).world();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    {
        let catalog = crate::assets::archive::Catalog::open(
            &crate::assets::installation::game_base().unwrap(),
        )
        .unwrap();
        let environment =
            crate::assets::entities::EnvironmentRenderParameters { layers: Vec::new() };
        let reader = crate::assets::environment::ParameterReader {
            catalog: &catalog,
            parameters: &environment,
        };
        let identity = reader.image_source("water/causticstex").unwrap();
        assert_eq!(identity, "rendering/water/caustics3.dds");
        assert_eq!(
            reader.image_source("water/skycubemap").unwrap(),
            "_defaultCube"
        );
        let stem = crate::material::compiled_image_stem(&catalog, &identity).unwrap();
        let bytes = catalog.read(&stem).unwrap().unwrap();
        let chain = crate::texture::compressed_inline_array(&bytes).unwrap();
        let image = LayeredImage::array(device, queue, &chain).unwrap();
        assert_eq!(image.payload_bytes(), chain.payload.len() as u64);
        assert_eq!(image.texture().depth_or_array_layers(), 32);
        let mut regions = Vec::new();
        let mut total = 0u64;
        let mut source = 0usize;
        for level in 0..chain.info.mip_count {
            let width = (chain.info.width >> level).max(1);
            let height = (chain.info.height >> level).max(1);
            let row = width.div_ceil(4) * 8;
            let rows = height.div_ceil(4);
            let pitch = row.next_multiple_of(256);
            for layer in 0..chain.layers {
                regions.push((level, layer, width, height, row, rows, pitch, total, source));
                total += u64::from(pitch) * u64::from(rows);
                source += (row * rows) as usize;
            }
        }
        assert_eq!(source, chain.payload.len());
        let staging = device.create_buffer(&BufferDescriptor {
            label: Some("array mip numeric readback"),
            size: total,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
        for &(level, layer, width, height, _, rows, pitch, offset, _) in &regions {
            let mut source = image.texture().as_image_copy();
            source.mip_level = level;
            source.origin.z = layer;
            encoder.copy_texture_to_buffer(
                source,
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(pitch),
                        rows_per_image: Some(rows),
                    },
                },
                Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let gpu = staging.slice(..).get_mapped_range();
        for &(level, layer, _, _, row, rows, pitch, offset, source) in &regions {
            for y in 0..rows as usize {
                let start = offset as usize + y * pitch as usize;
                let reference = source + y * row as usize;
                assert_eq!(
                    &gpu[start..start + row as usize],
                    &chain.payload[reference..reference + row as usize],
                    "mip{level} layer{layer} row{y}"
                );
            }
        }
        drop(gpu);
        staging.unmap();
        eprintln!(
            "All {} installed BC4 mip/layer records upload bit-exactly",
            regions.len()
        );
        let mut synthetic = vec![0u8; 62];
        synthetic[..3].copy_from_slice(b"BIM");
        for (offset, value) in [
            (12, 2u32),
            (16, 7),
            (20, 1),
            (24, 1),
            (28, 1),
            (44, 1),
            (52, 32),
        ] {
            synthetic[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        for layer in 0..32u32 {
            for value in [0, layer, 1, 1, 2] {
                synthetic.extend(value.to_le_bytes());
            }
            synthetic.extend(half::f16::from_f32(layer as f32).to_bits().to_le_bytes());
        }
        let chain = crate::texture::compressed_inline_array(&synthetic).unwrap();
        let frames = LayeredImage::array(device, queue, &chain).unwrap();
        let fixture = include_str!("../../../../tests/fixtures/texture_array_layers.txt")
            .lines()
            .map(|line| {
                line.split_whitespace()
                    .map(|v| u32::from_str_radix(v, 16).unwrap())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(fixture.len(), 320);
        let query = fixture
            .iter()
            .flat_map(|row| [row[0], 0, 0, 0].into_iter().flat_map(u32::to_le_bytes))
            .collect::<Vec<_>>();
        let input = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &query,
            usage: BufferUsages::STORAGE,
        });
        let output = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 320 * 16,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let result = device.create_buffer(&BufferDescriptor {
            label: None,
            size: output.size(),
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shader=device.create_and_validate_shader_module(ShaderModuleDescriptor {label:Some("source array frame selection"),source:ShaderSource::Wgsl(format!("{SHADER_SOURCE}\n{}",r#"
@group(0) @binding(0) var frames:texture_2d_array<f32>;
@group(0) @binding(1) var filtering:sampler;
@group(0) @binding(2) var<storage,read> queries:array<vec4<f32>>;
@group(0) @binding(3) var<storage,read_write> output:array<vec4<f32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let layer=texture_array_layer(queries[id.x].x,textureNumLayers(frames));
    output[id.x]=vec4(textureSampleLevel(frames,filtering,vec2(0.5),layer,0.0).r,f32(layer),0.0,0.0);
}

"#).into())});
        let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: default(),
            cache: None,
        });
        let sampler = device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Linear,
            ..default()
        });
        let binding = device.create_bind_group(
            None,
            &pipeline.get_bind_group_layout(0).into(),
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(frames.view()),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&sampler),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: input.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: output.as_entire_binding(),
                },
            ],
        );
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &binding, &[]);
            pass.dispatch_workgroups(5, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &result, 0, output.size());
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        result
            .slice(..)
            .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let gpu = result.slice(..).get_mapped_range();
        for (i, row) in fixture.iter().enumerate() {
            for c in 0..2 {
                let at = i * 16 + c * 4;
                let actual = f32::from_le_bytes(gpu[at..at + 4].try_into().unwrap());
                assert_eq!(actual, row[1] as f32, "frame coordinate bits{:08x}", row[0]);
            }
        }
        drop(gpu);
        result.unmap();
        eprintln!("All320 floating array-layer cases match source sampler");
    }
    device.poll(PollType::Poll).unwrap();
}

#[test]
#[ignore = "requires GPU; actual BC1 intrinsic cube against source sampler readbacks"]
fn default_cube_matches_source_upload_and_sampling() {
    let app = gpu_app();
    let world = app.sub_app(RenderApp).world();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    {
        let chain = crate::material::intrinsic_cube_image("_defaultCube").unwrap();
        let cube = LayeredImage::cube(device, queue, &chain).unwrap();
        assert_eq!(cube.payload_bytes(), 48);
        let staging = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 6 * 256,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
        for layer in 0..6 {
            let mut source = cube.texture().as_image_copy();
            source.origin.z = layer;
            encoder.copy_texture_to_buffer(
                source,
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset: u64::from(layer) * 256,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(1),
                    },
                },
                Extent3d {
                    width: 4,
                    height: 4,
                    depth_or_array_layers: 1,
                },
            );
        }
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = staging.slice(..).get_mapped_range();
        for layer in 0..6 {
            assert_eq!(
                &bytes[layer * 256..layer * 256 + 8],
                &chain.payload[layer * 8..layer * 8 + 8]
            );
        }
        drop(bytes);
        staging.unmap();
        let input = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 256 * 16,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let output = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 256 * 16,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let result = device.create_buffer(&BufferDescriptor {
            label: None,
            size: output.size(),
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("intrinsic cube numeric sampling"),
            source: ShaderSource::Wgsl(
                r#"
@group(0) @binding(0) var image:texture_cube<f32>;
@group(0) @binding(1) var filtering:sampler;
@group(0) @binding(2) var<storage,read> queries:array<vec4<f32>>;
@group(0) @binding(3) var<storage,read_write> output:array<vec4<f32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    output[id.x]=textureSampleLevel(image,filtering,queries[id.x].xyz,0.0);
}
"#
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
        let mut lines = include_str!("../../../../tests/fixtures/default_cube_samples.txt").lines();
        let mut modes = 0;
        while let Some(header) = lines.next() {
            assert_eq!(header, format!("mode {modes}"));
            let rows = (0..256)
                .map(|_| {
                    lines
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .map(|v| u32::from_str_radix(v, 16).unwrap())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            queue.write_buffer(
                &input,
                0,
                &rows
                    .iter()
                    .flat_map(|r| [r[0], r[1], r[2], 0].into_iter().flat_map(u32::to_le_bytes))
                    .collect::<Vec<_>>(),
            );
            let filter = if modes == 0 {
                FilterMode::Nearest
            } else {
                FilterMode::Linear
            };
            let sampler = device.create_sampler(&SamplerDescriptor {
                address_mode_u: AddressMode::Repeat,
                address_mode_v: AddressMode::Repeat,
                address_mode_w: AddressMode::Repeat,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: MipmapFilterMode::Nearest,
                lod_max_clamp: 13.,
                ..default()
            });
            let binding = device.create_bind_group(
                None,
                &pipeline.get_bind_group_layout(0).into(),
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(cube.view()),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Sampler(&sampler),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: input.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: output.as_entire_binding(),
                    },
                ],
            );
            let mut encoder =
                device.create_command_encoder(&CommandEncoderDescriptor { label: None });
            {
                let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &binding, &[]);
                pass.dispatch_workgroups(4, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &result, 0, output.size());
            let submission = queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            result
                .slice(..)
                .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
            device
                .poll(PollType::Wait {
                    submission_index: Some(submission),
                    timeout: None,
                })
                .unwrap();
            rx.recv().unwrap().unwrap();
            let bytes = result.slice(..).get_mapped_range();
            let mut maximum = 0f32;
            for (i, row) in rows.iter().enumerate() {
                for c in 0..4 {
                    let at = i * 16 + c * 4;
                    let actual = f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
                    let expected = f32::from_bits(row[3 + c]);
                    let error = (actual - expected).abs();
                    maximum = maximum.max(error);
                    assert!(
                        error < 2e-6,
                        "mode{modes} sample{i} channel{c}: {actual} != {expected}"
                    );
                }
            }
            eprintln!("default cube mode{modes} max error={maximum}");
            drop(bytes);
            result.unmap();
            modes += 1;
        }
        assert_eq!(modes, 2);
    }
    device.poll(PollType::Poll).unwrap();
}

#[test]
#[ignore = "requires installed float32 sky cube and graphics adapter"]
fn float32_cube_keeps_hdr_mips_and_native_filtering() {
    let app = gpu_app();
    let world = app.sub_app(RenderApp).world();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    {
        let catalog =
            crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap())
                .unwrap();
        let actual = crate::material::load_cube_image(
            &catalog,
            "maps/campaign/dust/dust_sky_reflection.dds",
        )
        .unwrap();
        assert!(!actual.defaulted);
        assert_eq!(
            actual.image.info.format,
            crate::texture::ImageFormat::Rgba32Float
        );
        let actual_gpu = LayeredImage::cube(device, queue, &actual.image).unwrap();
        assert_eq!(actual_gpu.texture().format(), TextureFormat::Rgba32Float);
        assert_eq!(
            actual_gpu.payload_bytes(),
            actual.image.payload.len() as u64
        );
        assert_float32_cube_mips(device, queue, &actual_gpu, &actual.image);
        let mut packed = 12 | (7 << 6) | (1 << 10);
        crate::texture::normalize_image_format(&mut packed).unwrap();
        let mut payload = Vec::new();
        for level in 0..4 {
            let size = 8 >> level;
            for face in 0..6 {
                for y in 0..size {
                    for x in 0..size {
                        let values = [
                            face as f32 * 4.0 + x as f32 * 0.125 - 0.5,
                            level as f32 + x as f32 * 0.03125 + y as f32 * 0.25,
                            face as f32 + level as f32 + x as f32 * 0.125 + y as f32 * 0.0625,
                            -((face + 1) as f32) * 0.125 + level as f32 * 0.25,
                        ];
                        payload.extend(values.into_iter().flat_map(f32::to_le_bytes));
                    }
                }
            }
        }
        let chain = crate::texture::InlineCubeChain {
            info: crate::texture::BimageInfo {
                width: 8,
                height: 8,
                mip_count: 4,
                packed_format: packed,
                format: crate::texture::ImageFormat::Rgba32Float,
            },
            payload,
        };
        let cube = LayeredImage::cube(device, queue, &chain).unwrap();
        let input = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 256 * 16,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let output = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 256 * 16,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let result = device.create_buffer(&BufferDescriptor {
            label: None,
            size: output.size(),
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("float32 cube numeric sampling"),
            source: ShaderSource::Wgsl(
                r#"
@group(0) @binding(0) var image:texture_cube<f32>;
@group(0) @binding(1) var filtering:sampler;
@group(0) @binding(2) var<storage,read> queries:array<vec4<f32>>;
@group(0) @binding(3) var<storage,read_write> output:array<vec4<f32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    output[id.x]=textureSampleLevel(image,filtering,queries[id.x].xyz,queries[id.x].w);
}
"#
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
        let mut lines = include_str!("../../../../tests/fixtures/float32_cube_samples.txt").lines();
        let mut modes = 0;
        while let Some(header) = lines.next() {
            assert_eq!(header, format!("mode {modes}"));
            let rows = (0..256)
                .map(|_| {
                    lines
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .map(|v| u32::from_str_radix(v, 16).unwrap())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            queue.write_buffer(
                &input,
                0,
                &rows
                    .iter()
                    .flat_map(|r| {
                        [r[0], r[1], r[2], r[3]]
                            .into_iter()
                            .flat_map(u32::to_le_bytes)
                    })
                    .collect::<Vec<_>>(),
            );
            let filter = if modes == 0 {
                FilterMode::Nearest
            } else {
                FilterMode::Linear
            };
            let sampler = device.create_sampler(&SamplerDescriptor {
                address_mode_u: AddressMode::Repeat,
                address_mode_v: AddressMode::Repeat,
                address_mode_w: AddressMode::Repeat,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: MipmapFilterMode::Nearest,
                lod_max_clamp: 13.,
                ..default()
            });
            let binding = device.create_bind_group(
                None,
                &pipeline.get_bind_group_layout(0).into(),
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(cube.view()),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Sampler(&sampler),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: input.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: output.as_entire_binding(),
                    },
                ],
            );
            let mut encoder =
                device.create_command_encoder(&CommandEncoderDescriptor { label: None });
            {
                let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &binding, &[]);
                pass.dispatch_workgroups(4, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &result, 0, output.size());
            let submission = queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            result
                .slice(..)
                .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
            device
                .poll(PollType::Wait {
                    submission_index: Some(submission),
                    timeout: None,
                })
                .unwrap();
            rx.recv().unwrap().unwrap();
            let bytes = result.slice(..).get_mapped_range();
            let mut maximum = 0f32;
            for (i, row) in rows.iter().enumerate() {
                for c in 0..4 {
                    let at = i * 16 + c * 4;
                    let actual = f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
                    let expected = f32::from_bits(row[4 + c]);
                    let error = (actual - expected).abs();
                    maximum = maximum.max(error);
                    assert!(
                        error <= 2e-6 + expected.abs() * 1e-6,
                        "mode{modes} sample{i} channel{c}: {actual} != {expected}"
                    );
                }
            }
            eprintln!("float32 cube mode{modes} max error={maximum}");
            drop(bytes);
            result.unmap();
            modes += 1;
        }
        assert_eq!(modes, 2);
    }
    device.poll(PollType::Poll).unwrap();
}

fn assert_float32_cube_mips(
    device: &RenderDevice,
    queue: &RenderQueue,
    cube: &LayeredImage,
    chain: &crate::texture::InlineCubeChain,
) {
    let mut regions = Vec::new();
    let mut total = 0u64;
    let mut source = 0;
    for level in 0..chain.info.mip_count {
        let width = (chain.info.width >> level).max(1);
        let rows = (chain.info.height >> level).max(1);
        let row = width * 16;
        let pitch = row.next_multiple_of(256);
        for face in 0..6 {
            regions.push((level, face, width, rows, row, pitch, total, source));
            total += u64::from(pitch * rows);
            source += (row * rows) as usize;
        }
    }
    assert_eq!(source, chain.payload.len());
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: total,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&default());
    for &(level, face, width, rows, _, pitch, offset, _) in &regions {
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: cube.texture(),
                mip_level: level,
                origin: Origin3d {
                    x: 0,
                    y: 0,
                    z: face,
                },
                aspect: TextureAspect::All,
            },
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset,
                    bytes_per_row: Some(pitch),
                    rows_per_image: Some(rows),
                },
            },
            Extent3d {
                width,
                height: rows,
                depth_or_array_layers: 1,
            },
        );
    }
    let submitted = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    staging.slice(..).map_async(MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(PollType::Wait {
            submission_index: Some(submitted),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    let bytes = staging.slice(..).get_mapped_range();
    for &(level, face, _, rows, row, pitch, offset, source) in &regions {
        for y in 0..rows as usize {
            let actual = offset as usize + y * pitch as usize;
            let expected = source + y * row as usize;
            assert_eq!(
                &bytes[actual..actual + row as usize],
                &chain.payload[expected..expected + row as usize],
                "float32 mip{level} face{face} row{y}"
            );
        }
    }
    eprintln!(
        "{} installed float32 cube mip/face records and {} bytes preserved exactly",
        regions.len(),
        chain.payload.len()
    );
    drop(bytes);
    staging.unmap();
}
