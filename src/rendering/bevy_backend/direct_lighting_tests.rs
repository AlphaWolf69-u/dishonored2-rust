use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        RenderApp,
    },
};
#[test]
#[ignore = "requires GPU; compares the actual surface kernels with shipped shader execution"]
fn direct_brdf_matches_game() {
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
    let words: Vec<u32> = include_str!("../../../tests/fixtures/direct_lighting.txt")
        .lines()
        .filter(|s| !s.starts_with('#'))
        .flat_map(str::split_whitespace)
        .map(|s| u32::from_str_radix(s, 16).unwrap())
        .collect();
    let inputs: Vec<u8> = words
        .chunks_exact(36)
        .flat_map(|r| r[..28].iter().flat_map(|v| v.to_le_bytes()))
        .collect();
    let count = (words.len() / 36) as u32;
    let input = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &inputs,
        usage: BufferUsages::STORAGE,
    });
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size: u64::from(count) * 32,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: u64::from(count) * 32,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let source = String::from(
        include_str!("../../../assets/shaders/dh2_direct.wgsl")
            .replace("#define_import_path dh2_direct", "")
            .as_str(),
    ) + r#"
 @group(0) @binding(0) var<storage,read> cases:array<vec4<f32>>;
 @group(0) @binding(1) var<storage,read_write> result:array<vec4<f32>>;
 @compute @workgroup_size(64) fn main(@builtin(global_invocation_id)i:vec3<u32>){
 if i.x>=512u{return;} let b=i.x*7u;
 let s=ward_surface(cases[b].xyz,cases[b+2u].xyz,cases[b].w,cases[b+3u].xyz,cases[b+4u].xyz,cases[b+3u].w,cases[b+4u].w,cases[b+6u].x);
 let light=WardLight(cases[b+1u].xyz,cases[b+1u].w,cases[b+5u].xyz,cases[b+2u].w,cases[b+5u].w);
 let r=ward_direct(s,light);result[2u*i.x]=vec4(r.diffuse,0);result[2u*i.x+1u]=vec4(r.specular,0);
 }"#;
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("Ward parity"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: default(),
        cache: None,
    });
    let layout: BindGroupLayout = pipeline.get_bind_group_layout(0).into();
    let bindings = device.create_bind_group(
        None,
        &layout,
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
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, u64::from(count) * 32);
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    staging.slice(..).map_async(MapMode::Read, move |r| {
        tx.send(r).unwrap();
    });
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    let bytes = staging.slice(..).get_mapped_range();
    let values: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|x| f32::from_le_bytes(x.try_into().unwrap()))
        .collect();
    let mut max: f32 = 0.;
    for (i, r) in words.chunks_exact(36).enumerate() {
        for j in 0..8 {
            let expected = f32::from_bits(r[28 + j]);
            let actual = values[i * 8 + j];
            let error = (actual - expected).abs();
            max = max.max(error);
            assert!(
                actual.is_finite() && error <= 2e-5 + expected.abs() * 3e-4,
                "case {i} lane {j}: actual={actual} expected={expected}"
            );
        }
    }
    println!("{count} cases; maximum absolute error {max}");
}
