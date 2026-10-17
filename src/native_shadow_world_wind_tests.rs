use super::*;
use bevy::prelude::*;
use bevy::render::{render_resource::*, renderer::RenderQueue, RenderApp};

#[test]
#[ignore = "requires graphics adapter and original installed world-wind stream-output fixtures"]
fn native_world_wind_matches_installed_vertex_shader() {
    run_world_wind_fixture(include_str!("../analysis/original_world_shadow_wind_cases.txt"),false);
}

#[test]
#[ignore = "requires graphics adapter and installed shadow-gather zero-wind stream-output fixture"]
fn native_world_shadow_zero_wind_matches_installed_vertex_shader() {
    run_world_wind_fixture(include_str!("../analysis/original_world_shadow_zero_wind_cases.txt"),false);
}

#[test]
#[ignore = "requires graphics adapter and installed 51-slot world VS stream-output fixtures"]
fn native_world_fixed_wind_matches_installed_vertex_shader() {
    run_world_wind_fixture(include_str!("../analysis/original_world_shadow_fixed_wind_cases.txt"),true);
}

fn run_world_wind_fixture(fixture: &str,fixed: bool) {
    let mut rows = fixture.lines();
    let meta: Vec<usize> = rows.next().unwrap().split_whitespace().skip(1)
        .map(|s|s.parse().unwrap()).collect();
    let [count,matrix_count,offset,wind_offset] = meta.as_slice() else { panic!("invalid fixture header") };
    let words = |line: &str, tag: &str| -> Vec<u32> {
        let mut fields = line.split_whitespace();
        assert_eq!(fields.next(),Some(tag));
        fields.map(|s|u32::from_str_radix(s,16).unwrap()).collect()
    };
    let bytes = |words: &[u32]| -> Vec<u8> { words.iter().flat_map(|w|w.to_le_bytes()).collect() };
    let vp = words(rows.next().unwrap(),"VP");
    let native = words(rows.next().unwrap(),"PARAMS");
    assert_eq!(native.len(),36);
    let mut view = Vec::new();
    for row in (0..4).rev() { view.extend_from_slice(&vp[row*4..row*4+4]); }
    view.extend(native);
    assert_eq!(view.len()*4,208);
    let matrices: Vec<_> = (0..*matrix_count).flat_map(|_|words(rows.next().unwrap(),"MATRIX")).collect();
    let indices: Vec<_> = (0..offset+count).flat_map(|_|words(rows.next().unwrap(),"INDEX")).collect();
    let winds: Vec<_> = (0..wind_offset+count).flat_map(|_|words(rows.next().unwrap(),"WIND")).collect();
    let mut vertices = Vec::new();
    let mut expected = Vec::new();
    for _ in 0..*count {
        let c = words(rows.next().unwrap(),"CASE");
        assert_eq!(c.len(),14);
        // Scene uses NATIVE_VEGETATION_POSITION: original game axes.
        vertices.extend([c[0],c[1],c[2],1f32.to_bits(),
            c[5],c[6],c[7],0,c[8],c[9],0,0]);
        expected.extend_from_slice(&c[10..14]);
    }
    assert!(rows.next().is_none());
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(bevy::window::WindowPlugin {
        primary_window:None,exit_condition:bevy::window::ExitCondition::DontExit,..default()
    }).disable::<bevy::winit::WinitPlugin>()
      .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>());
    app.finish(); app.cleanup();
    let world = app.sub_app_mut(RenderApp).world_mut();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let buffer = |label,words: &[u32],usage|device.create_buffer_with_data(&BufferInitDescriptor {
        label:Some(label),contents:&bytes(words),usage });
    let state = buffer("native world wind view/material/time/offsets",&view,BufferUsages::UNIFORM);
    let ids = buffer("original remapped IDs",&indices,BufferUsages::STORAGE);
    let matrices = buffer("native model rows",&matrices,BufferUsages::STORAGE);
    let winds = buffer("distinct previous/current wind records",&winds,BufferUsages::STORAGE);
    let vertices = buffer("host position/raw color/normal",&vertices,BufferUsages::STORAGE);
    let source = format!("{}\n{}\n{}\n{}",
        include_str!("../assets/shaders/dh2_shadow_wind.wgsl"),
        include_str!("../assets/shaders/dh2_shadow_world_project.wgsl"),
        include_str!("../assets/shaders/dh2_shadow_world_vegetation.wgsl"),r#"
        struct TestVertex { position: vec4<f32>, color: vec4<f32>, normal: vec4<f32> }
        @group(2) @binding(0) var<storage,read> vertices: array<TestVertex>;
        @group(2) @binding(1) var<storage,read_write> results: array<vec4<f32>>;
        @compute @workgroup_size(1) fn parity(@builtin(global_invocation_id) id: vec3<u32>) {
            let v=vertices[id.x];
            results[id.x]=world_vegetation_shadow_position(v.position.xyz,v.normal.xy,v.color.xyz,id.x);
        }
    "#);
    let source=if fixed {
        source.replace("results[id.x]=world_vegetation_shadow_position(",
            "results[id.x]=world_fixed_vegetation_shadow_position(")
    } else { source };
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label:Some("installed world wind VS parity"),source:ShaderSource::Wgsl(source.into()) });
    let pipeline = device.wgpu_device().create_compute_pipeline(&RawComputePipelineDescriptor {
        label:None,layout:None,module:&module,entry_point:Some("parity"),compilation_options:default(),cache:None });
    let size = (*count*16) as u64;
    let output = device.create_buffer(&BufferDescriptor { label:None,size,
        usage:BufferUsages::STORAGE|BufferUsages::COPY_SRC,mapped_at_creation:false });
    let readback = device.create_buffer(&BufferDescriptor { label:None,size,
        usage:BufferUsages::COPY_DST|BufferUsages::MAP_READ,mapped_at_creation:false });
    let binding = |group, entries: &[(u32,&Buffer)]|device.wgpu_device().create_bind_group(&BindGroupDescriptor {
        label:None,layout:&pipeline.get_bind_group_layout(group),entries:&entries.iter().map(|(slot,b)|
            BindGroupEntry { binding:*slot,resource:b.as_entire_binding() }).collect::<Vec<_>>() });
    let mut resources=vec![(0,&ids),(1,&matrices)];
    if !fixed { resources.push((4,&winds)); }
    let groups = [binding(0,&[(0,&state)]),binding(1,&resources),
        binding(2,&[(0,&vertices),(1,&output)])];
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        for (i,g) in groups.iter().enumerate() { pass.set_bind_group(i as u32,g,&[]); }
        pass.dispatch_workgroups(*count as u32,1,1);
    }
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,size);
    queue.submit([encoder.finish()]);
    let (tx,rx)=std::sync::mpsc::channel();
    readback.slice(..).map_async(MapMode::Read,move |r|tx.send(r).unwrap());
    device.wgpu_device().poll(PollType::Wait { submission_index:None,timeout:None }).unwrap();
    rx.recv().unwrap().unwrap();
    let mapped=readback.slice(..).get_mapped_range();
    let mut maximum_error=0.0f32;
    for (i,(actual,expected)) in mapped.chunks_exact(4).zip(expected).enumerate() {
        let actual=f32::from_le_bytes(actual.try_into().unwrap());
        let expected=f32::from_bits(expected);
        let error=(actual-expected).abs();
        maximum_error=maximum_error.max(error);
        // Same relative float comparison as the rigid wind suite, scaled for
        // this test's nonidentity model/projection. Not a production bias.
        assert!(error.is_finite() && error<=0.000001*(1.0+expected.abs()),
            "world-wind case {} component {}: {actual} vs {expected}; error={error}",i/4,i%4);
    }
    println!("installed world wind (fixed={fixed}): {count} cases, max clip error {maximum_error}");
    drop(mapped); readback.unmap();
}
