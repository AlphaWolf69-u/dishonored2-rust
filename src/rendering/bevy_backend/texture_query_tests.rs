//! Exercise the shader translator's texture query output fields on real devices.
use bevy::prelude::*;
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
    RenderApp,
};

#[test]
#[ignore = "requires GPU; validates texture query lowering on each backend"]
fn texture_query_dimensions_layers_levels_and_samples_gpu() {
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
    let world = app.sub_app_mut(RenderApp).world_mut();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let specs = [
        (
            TextureDimension::D1,
            TextureViewDimension::D1,
            16,
            1,
            1,
            1,
            1,
            TextureFormat::Rgba8Unorm,
            TextureUsages::TEXTURE_BINDING,
        ),
        (
            TextureDimension::D2,
            TextureViewDimension::D2Array,
            16,
            8,
            3,
            4,
            1,
            TextureFormat::Rgba8Unorm,
            TextureUsages::TEXTURE_BINDING,
        ),
        (
            TextureDimension::D2,
            TextureViewDimension::D2Array,
            16,
            8,
            2,
            1,
            1,
            TextureFormat::Depth32Float,
            TextureUsages::TEXTURE_BINDING,
        ),
        (
            TextureDimension::D2,
            TextureViewDimension::D2Array,
            16,
            8,
            3,
            1,
            1,
            TextureFormat::R32Float,
            TextureUsages::STORAGE_BINDING,
        ),
        (
            TextureDimension::D2,
            TextureViewDimension::CubeArray,
            16,
            16,
            12,
            3,
            1,
            TextureFormat::Rgba8Unorm,
            TextureUsages::TEXTURE_BINDING,
        ),
        (
            TextureDimension::D2,
            TextureViewDimension::D2,
            16,
            8,
            1,
            1,
            4,
            TextureFormat::Rgba8Unorm,
            TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
        ),
    ];
    let textures: Vec<_> = specs
        .iter()
        .map(
            |&(
                dimension,
                _,
                width,
                height,
                depth_or_array_layers,
                mip_level_count,
                sample_count,
                format,
                usage,
            )| {
                device.create_texture(&TextureDescriptor {
                    label: Some("query regression"),
                    size: Extent3d {
                        width,
                        height,
                        depth_or_array_layers,
                    },
                    dimension,
                    mip_level_count,
                    sample_count,
                    format,
                    usage,
                    view_formats: &[],
                })
            },
        )
        .collect();
    let views: Vec<_> = textures
        .iter()
        .zip(specs)
        .map(|(texture, (_, dimension, ..))| {
            texture.create_view(&TextureViewDescriptor {
                dimension: Some(dimension),
                ..default()
            })
        })
        .collect();
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 96,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 96,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("all texture query fields"), source: ShaderSource::Wgsl(r#"
        @group(0) @binding(0) var a: texture_1d<f32>;
        @group(0) @binding(1) var b: texture_2d_array<f32>;
        @group(0) @binding(2) var c: texture_depth_2d_array;
        @group(0) @binding(3) var d: texture_storage_2d_array<r32float, read>;
        @group(0) @binding(4) var e: texture_cube_array<f32>;
        @group(0) @binding(5) var f: texture_multisampled_2d<f32>;
        @group(0) @binding(6) var<storage, read_write> result: array<u32>;
        @compute @workgroup_size(1) fn main() {
            result[0]=textureDimensions(a); result[1]=textureNumLevels(a); result[2]=textureDimensions(a,0);
            result[3]=textureDimensions(b).x; result[4]=textureDimensions(b).y;
            result[5]=textureNumLayers(b); result[6]=textureNumLevels(b); result[7]=textureDimensions(b,2).x;
            result[8]=textureDimensions(c).x; result[9]=textureDimensions(c).y;
            result[10]=textureNumLayers(c); result[11]=textureNumLevels(c);
            result[12]=textureDimensions(d).x; result[13]=textureDimensions(d).y; result[14]=textureNumLayers(d);
            result[15]=textureDimensions(e).x; result[16]=textureDimensions(e).y;
            result[17]=textureNumLayers(e); result[18]=textureNumLevels(e);
            result[19]=textureDimensions(f).x; result[20]=textureDimensions(f).y; result[21]=textureNumSamples(f);
            result[22]=textureDimensions(b,2).y; result[23]=textureDimensions(e,1).x;
        }"#.into()) });
    let pipeline = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: default(),
            cache: None,
        });
    let mut entries: Vec<_> = views
        .iter()
        .enumerate()
        .map(|(i, view)| BindGroupEntry {
            binding: i as u32,
            resource: BindingResource::TextureView(view),
        })
        .collect();
    entries.push(BindGroupEntry {
        binding: 6,
        resource: output.as_entire_binding(),
    });
    let bindings = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 96);
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
    device.poll(PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    let actual: Vec<_> = bytes
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(
        actual,
        [16, 1, 16, 16, 8, 3, 4, 4, 16, 8, 2, 1, 16, 8, 3, 16, 16, 2, 3, 16, 8, 4, 2, 8]
    );
    drop(bytes);
    readback.unmap();
}
