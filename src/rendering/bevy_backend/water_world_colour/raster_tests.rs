//! Actual vertex fetch/binding/raster entry against cached VS14494 stream output.
use super::*;
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::{renderer::RenderQueue, RenderApp},
};
use std::collections::HashMap;

#[test]
#[ignore = "requires graphics adapter; numeric raw-world vertex raster readback"]
fn world_water_vertex_raster_uses_raw_mesh_and_shared_probe_bindings() {
    check_vertex_raster(
        include_str!("../../../../tests/fixtures/water_world_vertex.txt"),
        true,
    );
}

#[test]
#[ignore = "requires graphics adapter; numeric world-water vertex raster without probe buffers"]
fn world_water_vertex_raster_without_probes_matches_cached_stages() {
    check_vertex_raster(
        include_str!("../../../../tests/fixtures/water_world_position.txt"),
        false,
    );
}

fn check_vertex_raster(source: &str, probes: bool) {
    let mut fixture: HashMap<&str, Vec<Vec<u32>>> = HashMap::new();
    for line in source.lines() {
        let mut v = line.split_whitespace();
        let key = v.next().unwrap();
        if key != "META" {
            fixture
                .entry(key)
                .or_default()
                .push(v.map(|x| u32::from_str_radix(x, 16).unwrap()).collect());
        }
    }
    let floats = |v: &[u32]| v.iter().map(|x| f32::from_bits(*x)).collect::<Vec<_>>();
    let bytes = |v: &[u32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>();
    let mut pnw = vec![0; 8];
    for (i, key) in ["OFFSETS", "REFS", "TETRA", "TETRA_VERTEX"]
        .into_iter()
        .enumerate()
    {
        pnw[i] = pnw.len() as u32;
        pnw.extend(fixture[key].iter().flatten().copied());
    }
    let expected = &fixture["EXPECTED"];
    let cases: Vec<_> = expected
        .iter()
        .enumerate()
        .filter(|(_, v)| {
            let p = floats(&v[8..12]);
            let [x, y, z, w]: [f32; 4] = p.try_into().unwrap();
            w > 0. && z > 0. && z < w && x.abs() < w * 0.9 && y.abs() < w * 0.9
        })
        .take(64)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(cases.len(), 64, "need actual on-screen reference vertices");
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
    let stage = if probes {
        WorldProbeVertexStage::new(device)
    } else {
        WorldProbeVertexStage::without_probes(device)
    };
    let fragment=device.create_and_validate_shader_module(ShaderModuleDescriptor { label:None,
        source:ShaderSource::Wgsl(r#"
struct Input {@builtin(position) position:vec4<f32>,@location(0) a:vec4<f32>,@location(1) b:vec4<f32>,}
struct Output {@location(0) a:vec4<f32>,@location(1) b:vec4<f32>,}
@fragment fn fragment(v:Input)->Output {return Output(v.a,v.b);}
"#.into()) });
    let positions: Vec<[f32; 3]> = fixture["VERTEX"]
        .iter()
        .map(|v| floats(&v[..3]).try_into().unwrap())
        .collect();
    let mut mesh = Mesh::new(PrimitiveTopology::PointList, RenderAssetUsages::default());
    // A wrong ATTRIBUTE_POSITION binding must fail conspicuously, even where
    // a host-baked mesh otherwise has a valid count/layout.
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[999.0, 888.0, 777.0]; positions.len()],
    );
    mesh.insert_attribute(super::super::shadow_pass::VEGETATION_POSITION, positions);
    let packed: Vec<[u8; 16]> = fixture["VERTEX"]
        .iter()
        .map(|v| {
            let mut p = [0; 16];
            p[7] = (f32::from_bits(v[3]) * 127.0) as i8 as u8;
            p
        })
        .collect();
    retain_normal_w(&mut mesh, &packed).unwrap();
    let mesh_layout =
        mesh.get_mesh_vertex_buffer_layout(&mut bevy::mesh::MeshVertexBufferLayouts::default());
    // Validate linkage of the complete colour pipeline as well as the vertex
    // readback below. Controlled solid, clipped, unblended state; this does
    // not claim gameplay material/state admission or final colour parity.
    let mut colour_state = [0; 9];
    colour_state[5] = 1 << 28;
    let _colour_program = WorldProbeColourProgram::new(
        device,
        &mesh_layout,
        TextureFormat::Depth32Float,
        TextureFormat::Rgba16Float,
        colour_state,
    )
    .unwrap();
    let _view_exposure_program = WorldProbeColourProgram::new_with_view_exposure(
        device,
        &mesh_layout,
        TextureFormat::Depth32Float,
        TextureFormat::Rgba16Float,
        colour_state,
    )
    .unwrap();
    let _world_program = WorldProbeColourProgram::new_for_world_surface(
        device,
        &mesh_layout,
        TextureFormat::Depth32Float,
        TextureFormat::Rgba16Float,
        colour_state,
        false,
        if probes {
            crate::rendering::water::program::WorldWaterColourClass::ProbeHorizontal
        } else {
            crate::rendering::water::program::WorldWaterColourClass::Horizontal
        },
    )
    .unwrap();
    let vertex_layout = vertex_buffer_layout(&mesh_layout).unwrap();
    assert_eq!(vertex_layout.attributes[0].format, VertexFormat::Float32x3);
    assert_eq!(vertex_layout.attributes[1].format, VertexFormat::Float32);
    let mut vertices = vec![0; 137 * vertex_layout.array_stride as usize];
    vertices.extend(mesh.create_packed_vertex_buffer_data());
    let vertex_buffer = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        usage: BufferUsages::VERTEX,
        contents: &vertices,
    });
    let storage: Vec<_> = [
        bytes(&fixture["REMAP"][0]),
        bytes(&fixture["MATRIX"].concat()),
        bytes(&pnw),
        bytes(&fixture["INFO"].concat()),
        bytes(&fixture["CUBE_REMAP"][0]),
    ]
    .into_iter()
    .map(|v| {
        device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            usage: BufferUsages::STORAGE,
            contents: &v,
        })
    })
    .collect();
    let inputs = if !probes {
        stage.bind_world_inputs(device, &storage[0], &storage[1])
    } else {
        stage.bind_inputs(
            device,
            WorldProbeVertexBuffers {
                instances: &storage[0],
                matrices: &storage[1],
                references: &storage[2],
                probe_info: &storage[3],
                cube_indices: &storage[4],
            },
        )
    };
    let stride = 80u64.next_multiple_of(u64::from(
        device.limits().min_uniform_buffer_offset_alignment,
    ));
    let mut uniforms = vec![0; stride as usize * cases.len()];
    for (n, &i) in cases.iter().enumerate() {
        let p = WaterWorldColourParameters {
            clip_from_world: std::array::from_fn(|r| {
                std::array::from_fn(|c| f32::from_bits(fixture["VP"][0][r * 4 + c]))
            }),
            instance_buffer_offset: 3 + (i / 16) as u32,
            first_vertex: 137,
            first_instance: 2 + (i / 16) as u32,
        };
        uniforms[n * stride as usize..n * stride as usize + 80].copy_from_slice(&p.uniform_bytes());
    }
    let uniforms = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        usage: BufferUsages::UNIFORM,
        contents: &uniforms,
    });
    let parameters = stage.bind_parameters(device, &uniforms);
    let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&stage.parameters), Some(&stage.inputs)],
        immediate_size: 0,
    });
    let targets = [0, 1].map(|_| {
        Some(ColorTargetState {
            format: TextureFormat::Rgba32Float,
            blend: None,
            write_mask: ColorWrites::ALL,
        })
    });
    let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
        label: None,
        layout: Some(&layout),
        vertex: RawVertexState {
            module: &stage.module,
            entry_point: Some(if probes {
                "water_world_colour_vertex"
            } else {
                "water_world_colour_vertex_without_probes"
            }),
            compilation_options: default(),
            buffers: &[RawVertexBufferLayout {
                array_stride: vertex_layout.array_stride,
                step_mode: vertex_layout.step_mode,
                attributes: &vertex_layout.attributes,
            }],
        },
        fragment: Some(RawFragmentState {
            module: &fragment,
            entry_point: Some("fragment"),
            compilation_options: default(),
            targets: &targets,
        }),
        primitive: PrimitiveState {
            topology: PrimitiveTopology::PointList,
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
    let texture = |format| {
        device.create_texture(&TextureDescriptor {
            label: None,
            size: Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };
    let images = [
        texture(TextureFormat::Rgba32Float),
        texture(TextureFormat::Rgba32Float),
        texture(TextureFormat::Depth32Float),
    ];
    let views = images.each_ref().map(|v| v.create_view(&default()));
    let plane = 64 * 1024u64;
    let case_bytes = plane * 2 + 64 * 256;
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: case_bytes * cases.len() as u64,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
    for (n, &i) in cases.iter().enumerate() {
        {
            let colors = [0, 1].map(|k| {
                Some(RenderPassColorAttachment {
                    view: &views[k],
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(default()),
                        store: StoreOp::Store,
                    },
                })
            });
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                color_attachments: &colors,
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &views[2],
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &parameters, &[(n as u64 * stride) as u32]);
            pass.set_bind_group(1, &inputs, &[]);
            pass.set_vertex_buffer(0, *vertex_buffer.slice(..));
            let v = 137 + (i % 16) as u32;
            let instance = 2 + (i / 16) as u32;
            pass.draw(v..v + 1, instance..instance + 1);
        }
        for k in 0..3 {
            encoder.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: &images[k],
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: if k == 2 {
                        TextureAspect::DepthOnly
                    } else {
                        TextureAspect::All
                    },
                },
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset: n as u64 * case_bytes + k as u64 * plane,
                        bytes_per_row: Some(if k == 2 { 256 } else { 1024 }),
                        rows_per_image: Some(64),
                    },
                },
                Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
            );
        }
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
    let read = staging.slice(..).get_mapped_range();
    let mut maximum = 0f32;
    let f = |offset: usize| f32::from_le_bytes(read[offset..offset + 4].try_into().unwrap());
    for (n, &i) in cases.iter().enumerate() {
        let start = n * case_bytes as usize;
        let pixels: Vec<_> = (0..4096)
            .filter(|&p| f(start + 2 * plane as usize + p * 4) > 0.)
            .collect();
        assert_eq!(pixels.len(), 1, "case{n} source{i}");
        let pixel = pixels[0];
        for word in 0..8 {
            let actual = f(start + (word / 4) * plane as usize + pixel * 16 + (word % 4) * 4);
            let expected = f32::from_bits(expected[i][word]);
            let error = (actual - expected).abs();
            maximum = maximum.max(error);
            assert!(
                error <= 2e-5 * (1.0 + expected.abs()),
                "case{n} word{word} {actual} vs {expected}"
            );
        }
        let clip = floats(&expected[i][8..12]);
        let z = f(start + 2 * plane as usize + pixel * 4);
        assert!(
            (z - clip[2] / clip[3]).abs() < 2e-6,
            "case{n} projected depth"
        );
        let xy = [
            (clip[0] / clip[3] + 1.) * 32.,
            (1. - clip[1] / clip[3]) * 32.,
        ];
        assert_eq!(
            [pixel % 64, pixel / 64],
            xy.map(|v| v.floor() as usize),
            "case{n} projected position"
        );
    }
    println!("64 actual world-water vertex raster cases max varying error={maximum}");
}
