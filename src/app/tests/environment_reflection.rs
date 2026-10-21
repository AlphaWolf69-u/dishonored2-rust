use super::*;

#[test]
#[ignore = "requires installed assets and a graphics adapter"]
fn installed_cube_upload_preserves_faces_mips_and_hdr() {
    use bevy::render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    };
    let catalog =
        archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let probes = probe_lighting::load_for_world(&catalog, "dust_p_lowchaos")
        .unwrap()
        .unwrap();
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
    let loaded = app
        .world_mut()
        .resource_scope(|world, mut images: Mut<Assets<Image>>| {
            let mut buffers = world.resource_mut::<Assets<ShaderBuffer>>();
            super::super::environment_reflection::load(
                Some(&catalog),
                Some(&probes),
                &mut images,
                &mut buffers,
            )
            .unwrap()
        });
    let lighting = {
        let mut buffers = app.world_mut().resource_mut::<Assets<ShaderBuffer>>();
        // PNW contents are covered by the separate world-vertex test. This
        // test verifies the reflection publisher and its GPU record layouts.
        let references = buffers.add(ShaderBuffer::from(vec![0u32; 8]));
        loaded
            .water_probes(Some(&probes), references, &mut buffers)
            .unwrap()
            .unwrap()
    };
    assert_eq!(lighting.cube_count as usize, probes.reflection_probes.len());
    let handle = loaded.image;
    let lookup = loaded.brdf_lookup;
    let cube_indices = lighting.cube_indices;
    for _ in 0..8 {
        app.update();
    }
    let render = app.sub_app(RenderApp).world();
    let device = render.resource::<RenderDevice>();
    let queue = render.resource::<RenderQueue>();
    let image = render
        .resource::<RenderAssets<GpuImage>>()
        .get(handle.id())
        .unwrap();
    let indices = [
        0,
        1,
        probes.reflection_probes.len() / 2,
        probes.reflection_probes.len() - 1,
    ];
    let mips = image.texture.mip_level_count();
    let count = 4 * 6 * mips;
    let total = count + 64 + lighting.primary_count;
    let size = u64::from(total) * 16;
    let mut expected = vec![[0f32; 3]; total as usize];
    for i in 0..lighting.primary_count as usize {
        let (index, luminance) = probes.reflection_info(i);
        expected[count as usize + 64 + i] = [index as f32, luminance, index as f32];
    }
    for (slot, &index) in indices.iter().enumerate() {
        let path =
            material::compiled_image_stem(&catalog, &probes.reflection_probes[index].cubemap_name)
                .unwrap();
        let cube = texture::compressed_inline_cube(&catalog.read(&path).unwrap().unwrap()).unwrap();
        let mut offset = 0;
        for level in 0..mips {
            let width = (cube.info.width >> level).max(1);
            let p = width / 2;
            let face_size = cube.info.format.mip_bytes(width, width);
            for face in 0..6 {
                let at =
                    offset + face * face_size + ((p / 4 * width.div_ceil(4) + p / 4) * 16) as usize;
                let mut pixels = [0.; 48];
                bcdec_rs::bc6h_float(&cube.payload[at..at + 16], &mut pixels, 12, false);
                let pixel = ((p % 4) * 4 + p % 4) as usize * 3;
                expected[level as usize * 24 + slot * 6 + face]
                    .copy_from_slice(&pixels[pixel..pixel + 3]);
            }
            offset += 6 * face_size;
        }
    }
    let lookup = lookup.unwrap();
    let lookup = render
        .resource::<RenderAssets<GpuImage>>()
        .get(lookup.id())
        .unwrap();
    let raw = catalog
        .read("generated/image/rendering/brdf/preintegratedbrdf_ward.bimage7")
        .unwrap()
        .unwrap();
    let chain = texture::compressed_inline_chain(&raw).unwrap();
    assert_eq!((chain.info.width, chain.info.height), (64, 64));
    for i in 0..64 {
        let x = i % 8 * 8 + 1;
        let y = i / 8 * 8 + 5;
        for channel in 0..2 {
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let at = ((y + dy) * 64 + x + dx) * 8 + channel * 4;
                expected[count as usize + i][channel] +=
                    0.25 * f32::from_le_bytes(chain.payload[at..at + 4].try_into().unwrap());
            }
        }
    }
    let source = r#"
@group(0) @binding(0)var image:texture_2d_array<f32>;
@group(0) @binding(1)var<storage,read_write> output:array<vec4<f32>>;
@group(0) @binding(2)var lookup:texture_2d<f32>;
@group(0) @binding(3)var lookup_sampler:sampler;
@group(0) @binding(4)var<storage,read> cube_indices:array<u32>;
@group(0) @binding(5)var<storage,read> probe_info:array<vec2<u32>>;
@compute @workgroup_size(1)fn main(@builtin(global_invocation_id) id:vec3<u32>){
if id.x>=COUNT+64u {let info=probe_info[id.x-COUNT-64u];output[id.x]=vec4(f32(info.x),bitcast<f32>(info.y),f32(cube_indices[info.x]),0.);return;}
if id.x>=COUNT {let i=id.x-COUNT;let uv=(vec2<f32>(f32(i%8u),f32(i/8u))+vec2(0.25,0.75))/8.0;output[id.x]=textureSampleLevel(lookup,lookup_sampler,uv,0.0);return;}
let indices=CUBES;let level=id.x/24u;let slot=(id.x/6u)%4u;let face=id.x%6u;
let width=textureDimensions(image,i32(level)).x;
output[id.x]=textureLoad(image,vec2<i32>(i32(width/2u)),i32(cube_indices[indices[slot]]*6u+face),i32(level));
}
"#.replace("COUNT",&format!("{count}u"))
    .replace(
        "CUBES",
        &format!(
            "array<u32,4>({}u,{}u,{}u,{}u)",
            indices[0], indices[1], indices[2], indices[3]
        ),
    );
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("installed reflection BC6H upload"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: default(),
            cache: None,
        });
    let view = image.texture.create_view(&TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let cube_indices = render
        .resource::<RenderAssets<bevy::render::storage::GpuShaderBuffer>>()
        .get(cube_indices.id())
        .unwrap();
    let probe_info = render
        .resource::<RenderAssets<bevy::render::storage::GpuShaderBuffer>>()
        .get(lighting.probe_info.id())
        .unwrap();
    let bind = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(&lookup.texture_view),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::Sampler(&lookup.sampler),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: cube_indices.buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: probe_info.buffer.as_entire_binding(),
                },
            ],
        });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(total, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback.slice(..).map_async(MapMode::Read, move |r| {
        tx.send(r).unwrap();
    });
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
        .unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    let actual = bytes
        .chunks_exact(16)
        .map(|b| {
            std::array::from_fn::<_, 3, _>(|c| {
                f32::from_le_bytes(b[c * 4..c * 4 + 4].try_into().unwrap())
            })
        })
        .collect::<Vec<_>>();
    drop(bytes);
    readback.unmap();
    for (i, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        for c in 0..3 {
            assert!(
                actual[c].is_finite()
                    && (actual[c] - expected[c]).abs() <= 2e-5 + 2e-3 * expected[c].abs(),
                "cube sample{i} channel{c}: {} != {}",
                actual[c],
                expected[c]
            );
        }
    }
    println!(
        "{count} BC6H face/mip samples, 64 Ward samples and {} primary probe records/remaps passed",
        lighting.primary_count
    );
}

#[test]
#[ignore = "requires a graphics adapter"]
fn reflection_math_matches_installed_shader() {
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
    shader_matches(&mut app);
}

pub(super) fn shader_matches(app: &mut App) {
    use bevy::render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    };
    let world = app.sub_app_mut(RenderApp).world_mut();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let fixture: Vec<Vec<f32>> = include_str!("../../../tests/fixtures/environment_reflection.txt")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|line| {
            line.split_whitespace()
                .map(|w| f32::from_bits(u32::from_str_radix(w, 16).unwrap()))
                .collect()
        })
        .collect();
    assert_eq!(fixture.len(), 64);
    let indirect = crate::rendering::bevy_backend::probe_weights::indirect_source(include_str!(
        "../../../assets/shaders/dh2_indirect.wgsl"
    ))
    .replace("#define_import_path dh2_indirect", "")
    .replace("#{MATERIAL_BIND_GROUP}", "0");
    let reflection = include_str!("../../../assets/shaders/dh2_reflection.wgsl")
        .lines()
        .filter(|line| {
            !line.starts_with('#')
                && !line.contains("binding(156)")
                && !line.contains("binding(157)")
                && !line.contains("binding(159)")
                && !line.contains("binding(160)")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .replace("#{MATERIAL_BIND_GROUP}", "0")
        .replace(
            "textureSampleLevel(reflection_cubes,reflection_sampler,direction,i32(index),mip).rgb",
            "reflection_sample(direction,index,mip)",
        ).replace("textureSampleLevel(reflection_brdf_lookup,reflection_brdf_sampler,vec2(clamp(dot(V,N),0.0,1.0),roughness),0.0).rg","integrated_ward(dot(V,N),roughness)");
    let source = format!(
        "{indirect}\n{reflection}\n{}",
        r#"
fn reflection_sample(d:vec3<f32>,index:u32,mip:f32)->vec3<f32>{return abs(d)*(1.0+f32(index)*0.1)+mip*0.2;}
@group(0) @binding(159)var<storage,read_write> result:array<vec4<f32>>;
@group(0) @binding(160)var<storage,read> inputs:array<vec4<f32>>;
fn view_vector(p:vec3<f32>)->vec3<f32>{return vec3(p.x,p.z,-p.y);}
@compute @workgroup_size(1) fn main(@builtin(global_invocation_id) tid:vec3<u32>){
let id=tid.x;let at=id*7u;let p=inputs[at];let n=inputs[at+1u];let v=inputs[at+2u];let f0=inputs[at+3u].xyz;
let r=n.w*n.w;
result[id*3u]=vec4(integrated_ward(dot(v.xyz,n.xyz),r),0.0,0.0);
result[id*3u+1u]=vec4(reflected_light(id,view_vector(p.xyz),view_vector(n.xyz),view_vector(v.xyz),n.w,f0,false),0.0);
result[id*3u+2u]=reflection_direction(p.xyz,normalize(mix(reflect(-v.xyz,n.xyz),n.xyz,r)),u32(v.w));
}
"#
    );
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("installed HQ reflection math"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: default(),
            cache: None,
        });
    let mut records = vec![0f32; 64 * 9 * 4];
    for (id, input) in fixture.iter().enumerate() {
        records[id * 36 + 3] = input[11];
        records[id * 36 + 7] = input[3];
    }
    let bytes = records
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<_>>();
    let sh = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &bytes,
        usage: BufferUsages::STORAGE,
    });
    let mut words = vec![4u32, 0, 0, 0];
    for index in 0..4 {
        for plane in 0..6 {
            let mut n = [0.; 3];
            n[plane / 2] = if plane % 2 == 0 { -1. } else { 1. };
            words.extend(crate::rendering::environment_reflection::pack_plane(
                n,
                3. + plane as f32 * 0.5 + index as f32,
            ));
        }
        words.extend(
            [
                index as f32 * 0.25,
                -(index as f32) * 0.5,
                index as f32 * 0.125,
                0.,
            ]
            .map(f32::to_bits),
        );
    }
    let bytes = words
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
    let planes = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &bytes,
        usage: BufferUsages::STORAGE,
    });
    let bytes = fixture
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<_>>();
    let inputs = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &bytes,
        usage: BufferUsages::STORAGE,
    });
    let size = 64 * 3 * 16;
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let bind = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 100,
                    resource: sh.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 158,
                    resource: planes.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 159,
                    resource: output.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 160,
                    resource: inputs.as_entire_binding(),
                },
            ],
        });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(64, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback.slice(..).map_async(MapMode::Read, move |r| {
        tx.send(r).unwrap();
    });
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
        .unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    let actual = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect::<Vec<_>>();
    drop(bytes);
    readback.unmap();
    for (id, input) in fixture.iter().enumerate() {
        for i in (8..12).chain(0..8) {
            let expected = input[16 + i];
            let actual = actual[id * 12 + i];
            assert!(
                actual.is_finite() && (actual - expected).abs() <= 5e-4 * expected.abs().max(1.),
                "reflection case{id} lane{i}: {actual} != {expected}"
            );
        }
    }
}
