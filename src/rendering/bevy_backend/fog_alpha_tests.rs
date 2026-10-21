//! Alpha helpers share the production fog math and installed numeric oracle.
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};

pub(super) fn check_alpha(device: &RenderDevice, queue: &RenderQueue) {
    let source = format!(
        "{}\n{}\n{}",
        include_str!("../../../assets/shaders/dh2_fog_math.wgsl"),
        include_str!("../../../assets/shaders/dh2_fog_alpha.wgsl"),
        r#"
@group(0) @binding(0) var<storage,read_write> output:array<vec4<f32>>;
@compute @workgroup_size(1) fn main(@builtin(global_invocation_id) tid:vec3<u32>) {
    let id=tid.x;
    let p=WorldFogParameters(
        vec4(10.0,1.0/90.0,3.5,0.0),vec4(.1,.3,.8,.2),vec4(1.1,.8,.2,.7),
        vec4(2.0,1.0/70.0,.4,0.0),vec4(.2,.7,1.1,.3),vec4(1.8,.4,.1,.8),
        vec4(-2.0,14.0,.03,0.0),vec4(.6,0.0,.8,.75),vec4(10.0,9.8,7.2,.3),
        vec4(300.0,30.0,.25,0.0),vec4(5.0,100.0,0.0,0.0),vec4(.4,1.0,.7,0.0),
        vec4<u32>(select(0u,128u,(id&8u)!=0u),0u,0u,0u));
    let flags=vec3<f32>(f32(id&1u),f32((id>>1u)&1u),f32((id>>2u)&1u));
    let v=AlphaVolumeParameters(vec4(3.0,1.0/40.0,2.5,0.0),vec4(.4,.2,.6,1.0),vec4(.8,.9,.3,1.0),
        vec4(2.0,select(.03,0.0,(id&16u)!=0u),0.0,0.0),vec4(.6,0.0,.8,.75),vec4(10.0,9.8,7.2,.3),vec4(flags,0.0));
    let ray=vec3((f32(id%5u)-2.0)*.3,(f32(id%7u)-3.0)*.2,1.0);
    let distance=f32(id%13u)*17.0;
    let camera=f32(id%11u)-5.0;
    output[id*2u]=fog_alpha_world(p,vec4(flags,select(0.0,2.0,(id&32u)!=0u)),
        AlphaWorldSample(ray,distance,camera,f32(id%9u)/8.0,f32(id%5u)/4.0));
    output[id*2u+1u]=fog_alpha_volume(v,ray,distance,camera,vec2((f32(id%5u)-2.0)*3.0,f32(id%7u)*8.0));
}
"#
    );
    check_shader(
        device,
        queue,
        &source,
        include_str!("../../../tests/fixtures/fog_alpha.txt"),
        0,
    );
}

fn check_shader(
    device: &RenderDevice,
    queue: &RenderQueue,
    source: &str,
    expected: &str,
    binding: u32,
) {
    check_shader_seeded(device, queue, source, expected, binding, None);
}

fn check_shader_seeded(
    device: &RenderDevice,
    queue: &RenderQueue,
    source: &str,
    expected: &str,
    binding: u32,
    seed: Option<&[u8]>,
) {
    let expected = expected
        .split_whitespace()
        .map(|word| f32::from_bits(u32::from_str_radix(word, 16).unwrap()))
        .collect::<Vec<_>>();
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("alpha fog helper comparison"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
    let size = (expected.len() * 4) as u64;
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if let Some(seed) = seed {
        assert_eq!(seed.len() as u64, size);
        queue.write_buffer(&output, 0, seed);
    }
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
            entries: &[BindGroupEntry {
                binding,
                resource: output.as_entire_binding(),
            }],
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
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(MapMode::Read, move |result| tx.send(result).unwrap());
    device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    for (index, (raw, expected)) in bytes.chunks_exact(4).zip(expected).enumerate() {
        let actual = f32::from_le_bytes(raw.try_into().unwrap());
        assert!(
            actual == expected
                || actual.is_nan() && expected.is_nan()
                || (actual - expected).abs() < 0.0003,
            "alpha fog helper case={} part={} channel={}: {actual} != {expected}",
            index / 8,
            index / 4 % 2,
            index % 4
        );
    }
    drop(bytes);
    readback.unmap();
}

pub(super) fn check_scattering(device: &RenderDevice, queue: &RenderQueue) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // This ignored numerical test reads the production shaders/fixture at
    // runtime so oracle captures do not require relinking the whole client.
    let shadow = std::fs::read_to_string(root.join("assets/shaders/dh2_shadow.wgsl")).unwrap();
    let scattering = std::fs::read_to_string(root.join("assets/shaders/dh2_light_scattering.wgsl")).unwrap();
    let source = format!(
        "{}\n{}\n{}",
        shadow,
        scattering,
        r#"
@group(0) @binding(0) var<storage,read_write> data:array<vec4<f32>>;
@compute @workgroup_size(1) fn main(@builtin(global_invocation_id) tid:vec3<u32>) {
    let base=tid.x*12u;
    let a=data[base];let b=data[base+1u];let c=data[base+2u];let d=data[base+3u];
    let e=data[base+4u];let f=data[base+5u];let g=data[base+6u];let h=data[base+7u];let i=data[base+8u];
    let record=TiledLightRecord(c.xyz,d.w,f.xyz,c.w,d.xyz,0.0,
        0.0,f.w,g.w,h.x,g.xyz,h.y,e.xyz,e.w,
        0xffffffffu,(u32(b.w)<<4u)|(u32(h.z)<<2u)|u32(h.w),vec2(0u));
    let light=scatter_light_from_record(record);
    let kind=(record.flags>>4u)&7u;
    data[base+9u]=scatter_unshadowed(kind,a.xyz,b.xyz,a.w,light);
    let sphere=ray_sphere_interval(a.xyz,b.xyz,c.xyz,c.w);
    var shape:vec2<f32>;
    if kind==2u {
        if light.custom_shape!=0u {shape=ray_frustum_interval(a.xyz,b.xyz,light);}
        else {shape=ray_cone_interval(a.xyz,b.xyz,light);}
    } else {shape=ray_cube_interval(a.xyz,b.xyz,light);}
    data[base+10u]=vec4(sphere,shape);
    data[base+11u]=vec4(scatter_integral(a.xyz,b.xyz,c.xyz,a.w),
        scatter_attenuation(kind,a.xyz,b.xyz,i.z,a.w,light),scatter_turbulence(i.x,i.y,light),1.0);
}
"#
    );
    let fixture = std::fs::read_to_string(root.join("tests/fixtures/fog_scattering_shapes.txt")).unwrap();
    let mut words: Vec<u32> = fixture
        .split_whitespace()
        .map(|s| u32::from_str_radix(s, 16).unwrap())
        .collect();
    assert_eq!(words.len(), 64 * 12 * 4);
    let mut exercised = [[false;2];3];
    for row in words.chunks_exact(12*4) {
        if row[9*4..10*4].iter().any(|&v| f32::from_bits(v).abs()>0.000001) {
            exercised[f32::from_bits(row[7]) as usize][usize::from(f32::from_bits(row[30])!=0.)]=true;
        }
    }
    assert!(exercised.into_iter().flatten().all(|v|v), "each light/shape must produce scattering");
    // Poison output slots; an unexecuted/missing write must never pass by
    // reading the expected result we used to seed the actual input fields.
    for row in words.chunks_exact_mut(12 * 4) {
        row[9 * 4..].fill(0x7fc0_dead);
    }
    let bytes: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    check_shader_seeded(device, queue, &source, &fixture, 0, Some(&bytes));
    check_scattering_accumulation(device, queue);
}

pub(super) fn check_scattering_accumulation(device: &RenderDevice, queue: &RenderQueue) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = format!(
        "{}\n{}\n{}",
        std::fs::read_to_string(root.join("assets/shaders/dh2_shadow.wgsl")).unwrap(),
        std::fs::read_to_string(root.join("assets/shaders/dh2_light_scattering.wgsl")).unwrap(),
        r#"
@group(0) @binding(0) var<storage,read_write> data:array<vec4<f32>>;
@compute @workgroup_size(1) fn main(@builtin(global_invocation_id) tid:vec3<u32>) {
    let base=tid.x*16u;
    var sums:array<vec4<f32>,3>;
    for(var kind=0u;kind<3u;kind+=1u) {
        for(var light=0u;light<4u;light+=1u) {
            sums[kind]=scatter_add(sums[kind],data[base+kind*4u+light]);
        }
        data[base+12u+kind]=sums[kind];
    }
    data[base+15u]=scatter_resolve(sums[0],sums[1],sums[2]);
}
"#
    );
    let fixture = std::fs::read_to_string(root.join("tests/fixtures/fog_scattering_accumulation.txt")).unwrap();
    let mut words: Vec<u32> = fixture.split_whitespace()
        .map(|s| u32::from_str_radix(s,16).unwrap()).collect();
    assert_eq!(words.len(),64*16*4);
    for row in words.chunks_exact_mut(16*4) {row[12*4..].fill(0x7fc0_dead);}
    let bytes:Vec<u8>=words.into_iter().flat_map(u32::to_le_bytes).collect();
    check_shader_seeded(device,queue,&source,&fixture,0,Some(&bytes));
}

pub(super) fn check_volume(device: &RenderDevice, queue: &RenderQueue) {
    let source = format!(
        "{}\n{}\n{}",
        include_str!("../../../assets/shaders/dh2_fog_math.wgsl"),
        include_str!("../../../assets/shaders/dh2_fog_volume.wgsl"),
        r#"
@group(0) @binding(4) var<storage,read_write> output:array<vec4<f32>>;
@compute @workgroup_size(1) fn main(@builtin(global_invocation_id) tid:vec3<u32>) {
    let id=tid.x;
    let flags=1u|((id%4u)<<1u);
    let interval=vec4(f32(id%3u)*5.0,1.0-f32(id%4u)*12.0,0.0,f32(id%13u)*7.0);
    let ray=vec3((f32(id%5u)-2.0)*.3,(f32(id%7u)-3.0)*.2,1.0);
    let p=AreaFogUniform(mat4x4<f32>(),mat4x4<f32>(),mat4x4<f32>(),
        vec4(3.0,1.0/40.0,2.5,0.0),vec4(.4,.2,.6,1.0),vec4(.8,.9,.3,1.0),
        vec4(2.0,.03,f32(id%9u)-4.0,f32(flags)),vec4(.6,0.0,.8,.75),vec4(10.0,9.8,7.2,.3),vec4(1.0),vec4(1.0));
    output[id*2u]=select(area_radiance(p,ray,interval),vec4(-99.0),interval.w<0.001);
    output[id*2u+1u]=area_interval_value(f32(id%11u)*3.0,f32(id%7u)*5.0,(id&1u)!=0u);
}
"#
    );
    check_shader(
        device,
        queue,
        &source,
        include_str!("../../../tests/fixtures/fog_volume.txt"),
        4,
    );
}
