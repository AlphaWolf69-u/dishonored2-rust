//! Numeric depth-attachment gather and each intermediate R16F store, no images.
use super::*;
use bevy::render::RenderApp;

#[test]
#[ignore = "requires GPU; compares installed water foam depth and four blur shaders"]
fn water_foam_depth_matches_installed_shaders() {
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
    check_point_addresses(
        device,
        queue,
        "MIRROR",
        "point_mirror_texel",
        include_str!("../../../../tests/fixtures/point_mirror.txt"),
    );
    check_point_addresses(
        device,
        queue,
        "CLAMP",
        "point_clamp_texel",
        include_str!("../../../../tests/fixtures/point_clamp.txt"),
    );
    check_water_lighting(device, queue);
    let seed = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("synthetic depth attachment seeding"),
        source: ShaderSource::Wgsl(
            r#"
@group(0) @binding(0) var source:texture_2d<f32>;
@vertex fn vertex(@builtin(vertex_index) index:u32)->@builtin(position) vec4<f32> {
    let uv=vec2<f32>(f32((index<<1u)&2u),f32(index&2u));
    return vec4(uv*2.0-1.0,0.0,1.0);
}
@fragment fn fragment(@builtin(position) position:vec4<f32>)->@builtin(frag_depth) f32 {
    return textureLoad(source,vec2<i32>(position.xy),0).r;
}
"#
            .into(),
        ),
    });
    let seed = device.create_render_pipeline(&RawRenderPipelineDescriptor {
        label: Some("synthetic depth attachment seeding"),
        layout: None,
        vertex: RawVertexState {
            module: &seed,
            entry_point: Some("vertex"),
            compilation_options: default(),
            buffers: &[],
        },
        fragment: Some(RawFragmentState {
            module: &seed,
            entry_point: Some("fragment"),
            compilation_options: default(),
            targets: &[],
        }),
        primitive: PrimitiveState {
            cull_mode: None,
            ..default()
        },
        depth_stencil: Some(DepthStencilState {
            format: TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(CompareFunction::Always),
            stencil: default(),
            bias: default(),
        }),
        multisample: default(),
        multiview_mask: None,
        cache: None,
    });
    let mut input = include_str!("../../../../tests/fixtures/water_foam_depth.txt")
        .lines()
        .peekable();
    let mut cases = 0;
    while let Some(header) = input.next() {
        let header = header.split_whitespace().collect::<Vec<_>>();
        assert_eq!(header[0], "case");
        let full = UVec2::new(header[1].parse().unwrap(), header[2].parse().unwrap());
        let controls = header[3..]
            .iter()
            .map(|v| f32::from_bits(u32::from_str_radix(v, 16).unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(controls.len(), 32);
        let mut pixels = [Vec::new(), Vec::new()];
        for _ in 0..full.x * full.y {
            let row = input.next().unwrap().split_whitespace().collect::<Vec<_>>();
            assert_eq!(row[0], "input");
            for i in 0..2 {
                pixels[i].extend(u32::from_str_radix(row[i + 1], 16).unwrap().to_le_bytes());
            }
        }
        let texture = |format, usage| TextureDescriptor {
            label: Some("water foam oracle input"),
            size: Extent3d {
                width: full.x,
                height: full.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        };
        let raw = pixels.each_ref().map(|p| {
            device.create_texture_with_data(
                queue,
                &texture(TextureFormat::R32Float, TextureUsages::TEXTURE_BINDING),
                TextureDataOrder::LayerMajor,
                p,
            )
        });
        let depths: [Texture; 2] = std::array::from_fn(|_| {
            device.create_texture(&texture(
                TextureFormat::Depth32Float,
                TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            ))
        });
        let views = depths.each_ref().map(|t| t.create_view(&default()));
        let mut encoder = device.create_command_encoder(&default());
        for i in 0..2 {
            let source = raw[i].create_view(&default());
            let binding = device.create_bind_group(
                "synthetic depth input",
                &seed.get_bind_group_layout(0).into(),
                &[BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&source),
                }],
            );
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("seed water foam depths"),
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &views[i],
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..default()
            });
            pass.set_pipeline(&seed);
            pass.set_bind_group(0, &binding, &[]);
            pass.draw(0..3, 0..1);
        }
        let mut foam = WaterFoamDepth::new(device, queue, full).unwrap();
        let extent = foam.extent();
        assert_eq!(
            extent,
            ((full + UVec2::splat(3)) >> 2u32).max(UVec2::splat(2))
        );
        let region = WaterFoamRegion {
            viewport: [0., 0., extent.x as f32, extent.y as f32],
            scissor: [0, 0, extent.x, extent.y],
        };
        let view = WaterFoamView {
            inverse_view_projection: Mat4::from_cols_array(controls[..16].try_into().unwrap()),
            camera: Vec4::from_slice(&controls[16..20]),
            linear_depth: Vec4::from_slice(&controls[20..24]),
            near_far: Vec4::from_slice(&controls[24..28]),
        };
        let pixel_scale = Vec4::from_slice(&controls[28..]);
        foam.prepare(
            device,
            queue,
            &views[0],
            &views[1],
            view,
            pixel_scale,
            region,
        )
        .unwrap();
        let row_bytes = 256u32;
        let layer_bytes = u64::from(row_bytes * extent.y);
        let staging = device.create_buffer(&BufferDescriptor {
            label: Some("water foam numeric readback"),
            size: layer_bytes * 5,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        for step in 0..5 {
            foam.record_step(&mut encoder, step, region);
            let output = if step == 0 { 0 } else { step % 2 };
            encoder.copy_texture_to_buffer(
                foam.textures[output].as_image_copy(),
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset: layer_bytes * step as u64,
                        bytes_per_row: Some(row_bytes),
                        rows_per_image: Some(extent.y),
                    },
                },
                Extent3d {
                    width: extent.x,
                    height: extent.y,
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
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let mapped = staging.slice(..).get_mapped_range();
        if std::env::var_os("DH2_WATER_FOAM_DIAGNOSTICS").is_some() {
            std::fs::write(
                format!("analysis/water-foam-{}x{}.bin", full.x, full.y),
                &*mapped,
            )
            .unwrap();
        }
        let mut maximum_error = [0f32; 5];
        for step in 0..5 {
            let expected = input.next().unwrap().split_whitespace().collect::<Vec<_>>();
            assert_eq!(expected[0], "pass");
            assert_eq!(expected[1].parse::<usize>().unwrap(), step);
            assert_eq!(expected.len(), (extent.x * extent.y) as usize + 2);
            for y in 0..extent.y {
                for x in 0..extent.x {
                    let offset =
                        (step as u64 * layer_bytes + u64::from(y * row_bytes + x * 2)) as usize;
                    let actual = half::f16::from_bits(u16::from_le_bytes(
                        mapped[offset..offset + 2].try_into().unwrap(),
                    ))
                    .to_f32();
                    let reference = half::f16::from_bits(
                        u16::from_str_radix(expected[(y * extent.x + x) as usize + 2], 16).unwrap(),
                    )
                    .to_f32();
                    let error = (actual - reference).abs();
                    maximum_error[step] = maximum_error[step].max(error);
                    assert!(
                        actual.is_finite() && error <= 0.0015,
                        "{full:?} step{step} pixel{x},{y}: {actual} != {reference}, error {error}"
                    );
                }
            }
        }
        eprintln!("water foam {full:?} all five stores max errors={maximum_error:?}");
        drop(mapped);
        staging.unmap();
        let ids = foam.textures.each_ref().map(Texture::id);
        let binding = foam.depth_binding.as_ref().unwrap().1.id();
        foam.prepare(
            device,
            queue,
            &views[0],
            &views[1],
            view,
            pixel_scale,
            region,
        )
        .unwrap();
        assert_eq!(ids, foam.textures.each_ref().map(Texture::id));
        assert_eq!(binding, foam.depth_binding.as_ref().unwrap().1.id());
        let mut encoder = device.create_command_encoder(&default());
        foam.record(
            device,
            queue,
            &mut encoder,
            &views[0],
            &views[1],
            view,
            pixel_scale,
            region,
        )
        .unwrap();
        let submission = queue.submit([encoder.finish()]);
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        cases += 1;
    }
    assert_eq!(cases, 5);
    drop(seed);
    device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
}

fn check_point_addresses(
    device: &RenderDevice,
    queue: &RenderQueue,
    mode: &str,
    function: &str,
    fixture: &str,
) {
    let source = format!(
        "{}\n{}",
        include_str!("../../../../assets/shaders/dh2_point_sampling.wgsl"),
        r#"
@group(0) @binding(0) var<storage,read> positions:array<vec2<f32>>;
@group(0) @binding(1) var<storage,read_write> output:array<u32>;
@group(0) @binding(2) var<uniform> extent_count:vec4<u32>;
@group(0) @binding(3) var source_texture:texture_2d<f32>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=extent_count.z {return;}
    let point=point_mirror_texel(positions[id.x],textureDimensions(source_texture));
    output[id.x]=u32(point.y)*extent_count.x+u32(point.x)+1u;
}
"#
        .replace("point_mirror_texel", function)
    );
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("point sampler address proof"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: Some("point sampler address proof"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: default(),
        cache: None,
    });
    let mut rows = fixture.lines().peekable();
    let mut samples = 0;
    while let Some(header) = rows.next() {
        let header = header.split_whitespace().collect::<Vec<_>>();
        assert_eq!(header[0], "case");
        let width = header[1].parse::<u32>().unwrap();
        let height = header[2].parse::<u32>().unwrap();
        let mut positions = Vec::new();
        let mut expected = Vec::new();
        while rows.peek().is_some_and(|line| !line.starts_with("case ")) {
            let words = rows
                .next()
                .unwrap()
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 3);
            positions.extend(words[0].to_le_bytes());
            positions.extend(words[1].to_le_bytes());
            expected.push(f32::from_bits(words[2]) as u32);
        }
        let count = expected.len() as u32;
        let input = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &positions,
            usage: BufferUsages::STORAGE,
        });
        let controls = [width, height, count, 0]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        let controls = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &controls,
            usage: BufferUsages::UNIFORM,
        });
        let source_texture = device.create_texture(&TextureDescriptor {
            label: Some("point sampler resource dimensions"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::R16Float,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let source_view = source_texture.create_view(&default());
        let output = device.create_buffer(&BufferDescriptor {
            label: None,
            size: u64::from(count) * 4,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = device.create_buffer(&BufferDescriptor {
            label: None,
            size: output.size(),
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let binding = device.create_bind_group(
            None,
            &pipeline.get_bind_group_layout(0).into(),
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: input.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: controls.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::TextureView(&source_view),
                },
            ],
        );
        let mut encoder = device.create_command_encoder(&default());
        {
            let mut pass = encoder.begin_compute_pass(&default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &binding, &[]);
            pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
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
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let mapped = staging.slice(..).get_mapped_range();
        for (i, (bytes, wanted)) in mapped.chunks_exact(4).zip(expected).enumerate() {
            assert_eq!(
                u32::from_le_bytes(bytes.try_into().unwrap()),
                wanted,
                "point MIRROR {width}x{height} sample{i}"
            );
            samples += 1;
        }
        drop(mapped);
        staging.unmap();
    }
    assert_eq!(samples, 20480);
    eprintln!("point {mode}: all {samples} original-API address results exact");
}

fn check_water_lighting(device: &RenderDevice, queue: &RenderQueue) {
    let source = format!(
        "{}\n{}",
        include_str!("../../../../assets/shaders/dh2_water_lighting.wgsl"),
        r#"
@group(0) @binding(0) var<storage,read> inputs:array<vec4<f32>>;
@group(0) @binding(1) var<storage,read_write> outputs:array<vec4<f32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let a=inputs[id.x*2u];let b=inputs[id.x*2u+1u];let f=water_fresnel(a.w,b.w);
    outputs[id.x*2u]=vec4(water_inscattering(a.x,a.y,a.z,b.xyz),f.value);
    outputs[id.x*2u+1u]=vec4(select(0.0,1.0,f.total_internal_reflection),0.0,0.0,0.0);
}
"#
    );
    let mut input = Vec::new();
    let mut expected = Vec::new();
    for line in include_str!("../../../../tests/fixtures/water_lighting.txt").lines() {
        let words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(words.len(), 16);
        input.extend(words[..8].iter().flat_map(|v| v.to_le_bytes()));
        expected.extend(words[8..].iter().map(|v| f32::from_bits(*v)));
    }
    assert_eq!(input.len(), 256 * 32);
    assert_eq!(expected.len(), 256 * 8);
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("water scattering and Fresnel proof"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: default(),
        cache: None,
    });
    let input = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &input,
        usage: BufferUsages::STORAGE,
    });
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 8192,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 8192,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let binding = device.create_bind_group(
        None,
        &pipeline.get_bind_group_layout(0).into(),
        &[
            BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    );
    let mut encoder = device.create_command_encoder(&default());
    {
        let mut pass = encoder.begin_compute_pass(&default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &binding, &[]);
        pass.dispatch_workgroups(4, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, 8192);
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    staging
        .slice(..)
        .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    let mapped = staging.slice(..).get_mapped_range();
    let mut maximum = 0f32;
    let mut nans = 0;
    for (i, (bytes, wanted)) in mapped.chunks_exact(4).zip(expected).enumerate() {
        let actual = f32::from_le_bytes(bytes.try_into().unwrap());
        if wanted.is_nan() {
            assert!(
                actual.is_nan(),
                "water lighting component{i}: {actual} should be NaN"
            );
            nans += 1;
        } else {
            let error = (actual - wanted).abs();
            maximum = maximum.max(error);
            assert!(
                actual.is_finite() && error <= 0.000003 * (1. + wanted.abs()),
                "water lighting component{i}: {actual} != {wanted}"
            );
        }
    }
    assert_eq!(nans, 15);
    eprintln!("water lighting256 cases: max error={maximum}, matched NaNs={nans}");
    drop(mapped);
    staging.unmap();
}
