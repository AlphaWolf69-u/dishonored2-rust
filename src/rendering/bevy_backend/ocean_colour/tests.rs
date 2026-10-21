use super::*;
use bevy::{
    prelude::*,
    render::{renderer::RenderQueue, RenderApp},
};

#[test]
#[ignore = "requires GPU; installed ocean colour preparation and real raster pipeline creation"]
fn ocean_colour_matches_installed_inputs() {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(bevy::render::RenderPlugin {
                render_creation: bevy::render::settings::RenderCreation::Automatic(Box::new(
                    bevy::render::settings::WgpuSettings {
                        features: bevy::render::settings::WgpuSettings::default().features
                            | bevy::render::settings::WgpuFeatures::DEPTH32FLOAT_STENCIL8,
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
    let world = app.sub_app(RenderApp).world();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    {
        let noise = super::super::noise_textures::NoiseTextures::new(device, queue);
        let base = include_bytes!("../../../../tests/fixtures/ocean_surface_inputs.bin");
        let extra = include_bytes!("../../../../tests/fixtures/ocean_colour_inputs.bin");
        let upload = |format, data: &[u8]| {
            device.create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: Some("ocean colour numerical input"),
                    size: Extent3d {
                        width: 8,
                        height: 8,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format,
                    usage: TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                TextureDataOrder::LayerMajor,
                data,
            )
        };
        let mut textures = vec![
            upload(TextureFormat::R32Float, &base[..256]),
            upload(TextureFormat::Rgba16Float, &base[256..768]),
            upload(TextureFormat::Rgba16Float, &base[768..1280]),
            upload(TextureFormat::R16Float, &base[1280..1408]),
            upload(TextureFormat::R32Float, &base[1408..]),
        ];
        for bytes in extra.chunks_exact(256) {
            textures.push(upload(TextureFormat::R32Float, bytes));
        }
        assert_eq!(textures.len(), 9);
        let mut views = vec![noise.permutation().clone(), noise.gradients().clone()];
        views.extend(textures.iter().map(|t| t.create_view(&default())));
        let wrap = device.create_sampler(&SamplerDescriptor {
            address_mode_u: AddressMode::Repeat,
            address_mode_v: AddressMode::Repeat,
            min_filter: FilterMode::Linear,
            mag_filter: FilterMode::Linear,
            lod_max_clamp: 13.,
            ..default()
        });
        let clamp = device.create_sampler(&SamplerDescriptor {
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            min_filter: FilterMode::Linear,
            mag_filter: FilterMode::Linear,
            lod_max_clamp: 13.,
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
        let uniform = buffer(272, BufferUsages::UNIFORM | BufferUsages::COPY_DST);
        let queries = buffer(128 * 48, BufferUsages::STORAGE | BufferUsages::COPY_DST);
        let output = buffer(128 * 144, BufferUsages::STORAGE | BufferUsages::COPY_SRC);
        let staging = buffer(128 * 144, BufferUsages::MAP_READ | BufferUsages::COPY_DST);
        let entries = (0..17)
            .map(|binding| BindGroupLayoutEntry {
                binding,
                visibility: ShaderStages::COMPUTE,
                count: None,
                ty: match binding {
                    1..=11 => BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    14..=16 => BindingType::Sampler(SamplerBindingType::Filtering),
                    _ => BindingType::Buffer {
                        ty: if binding == 0 {
                            BufferBindingType::Uniform
                        } else {
                            BufferBindingType::Storage {
                                read_only: binding == 12,
                            }
                        },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                },
            })
            .collect::<Vec<_>>();
        let layout = device.create_bind_group_layout(None, &entries);
        let entries = (0..17)
            .map(|binding| BindGroupEntry {
                binding,
                resource: match binding {
                    0 => uniform.as_entire_binding(),
                    1..=11 => BindingResource::TextureView(&views[(binding - 1) as usize]),
                    12 => queries.as_entire_binding(),
                    13 => output.as_entire_binding(),
                    14 => BindingResource::Sampler(noise.point_wrap()),
                    15 => BindingResource::Sampler(&wrap),
                    _ => BindingResource::Sampler(&clamp),
                },
            })
            .collect::<Vec<_>>();
        let binding = device.create_bind_group(None, &layout, &entries);
        // Drop only the production raster entry declarations, retaining every
        // production helper verbatim for this per-stage numerical comparison.
        let helpers = SHADER_SOURCE
            .strip_suffix(include_str!(
                "../../../../assets/shaders/dh2_ocean_colour_pass.wgsl"
            ))
            .unwrap();
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("ocean colour inputs"),
            source: ShaderSource::Wgsl(format!("{helpers}\n{}", include_str!("tests.wgsl")).into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: default(),
            cache: None,
        });
        let mut lines = include_str!("../../../../tests/fixtures/ocean_colour.txt").lines();
        let mut frames = 0;
        while let Some(header) = lines.next() {
            let words = header
                .split_whitespace()
                .skip(1)
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 68);
            let f = |offset: usize| std::array::from_fn(|i| f32::from_bits(words[offset + i]));
            let surface = OceanSurfaceParameters {
                spatial: f(0),
                clock_view: f(4),
                camera: f(8),
                perlin: f(12),
                perlin_foam: f(16),
                shore: f(20),
                foam: f(24),
            };
            let colour = OceanColourParameters {
                projected: f(28),
                water: f(32),
                light: f(36),
                sun: f(40),
                pixel_to_texture: f(44),
            };
            let mut bytes = surface.uniform_bytes().to_vec();
            bytes.extend(colour.uniform_bytes());
            bytes.extend(words[48..].iter().flat_map(|v| v.to_le_bytes()));
            assert_eq!(
                bytes,
                words
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>()
            );
            queue.write_buffer(&uniform, 0, &bytes);
            let rows = (0..128)
                .map(|_| {
                    lines
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .map(|v| u32::from_str_radix(v, 16).unwrap())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            assert!(rows.iter().all(|r| r.len() == 48));
            queue.write_buffer(
                &queries,
                0,
                &rows
                    .iter()
                    .flat_map(|r| r[..12].iter().flat_map(|v| v.to_le_bytes()))
                    .collect::<Vec<_>>(),
            );
            let mut encoder = device.create_command_encoder(&default());
            {
                let mut pass = encoder.begin_compute_pass(&default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &binding, &[]);
                pass.dispatch_workgroups(2, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, output.size());
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
            let data = staging.slice(..).get_mapped_range();
            let mut maximum = 0f32;
            for (i, row) in rows.iter().enumerate() {
                for c in 0..36 {
                    let at = i * 144 + c * 4;
                    let value = f32::from_le_bytes(data[at..at + 4].try_into().unwrap());
                    let expected = f32::from_bits(row[12 + c]);
                    let error = (value - expected).abs();
                    maximum = maximum.max(error);
                    assert!(
                        value.is_finite()
                            && expected.is_finite()
                            && error < 3e-5 * (1. + expected.abs()),
                        "frame{frames} row{i} field{c}: {value} != {expected}"
                    );
                }
            }
            eprintln!("ocean colour frame{frames} max input error={maximum}");
            drop(data);
            staging.unmap();
            frames += 1;
        }
        assert_eq!(frames, 3);
        let (state, reference) = crate::rendering::ocean::ocean_draw_state(
            [
                0x01e008a9, 0x01e00802, 0x01e00802, 0x01e00802, 0xe00e0000, 0x30ffff1e, 0, 0, 0,
            ],
            crate::rendering::ocean::OceanDrawInputs {
                depth_pass: false,
                eye_height: 0.,
                surface_height: 0.,
                height_extent: 1.,
                wireframe: false,
                mirrored: false,
            },
        );
        let _program = OceanColourProgram::new(
            device,
            TextureFormat::Depth32FloatStencil8,
            TextureFormat::Rgba16Float,
            state,
            reference,
        )
        .unwrap();
        eprintln!("Created both complete ocean colour raster pipelines with source prepared state");
    }
    device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
}
