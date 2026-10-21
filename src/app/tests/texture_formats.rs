//! Numeric upload/sampling checks for authored texture formats.
use super::*;

#[test]
#[ignore = "requires GPU; uploads and samples R16Float authored mip payloads"]
fn half_float_texture_upload_and_sampling_gpu() {
    let values = [0.0009765625_f32, 0.25, 3., 4096., -2., 0.33325195, 12.];
    scalar_16_texture_upload_and_sampling(
        texture::ImageFormat::R16Float,
        values.map(|v| half::f16::from_f32(v).to_bits()),
        values,
        0.0,
    );
}

#[test]
#[ignore = "requires GPU; uploads and filters authored R16Unorm mip payloads"]
fn normalized_16_texture_upload_and_sampling_gpu() {
    let words = [0u16, 1, 32768, 65535, 12345, 54321, 40000];
    scalar_16_texture_upload_and_sampling(
        texture::ImageFormat::R16Unorm,
        words,
        words.map(|v| f32::from(v) / 65535.0),
        0.0000002,
    );
}

fn scalar_16_texture_upload_and_sampling(
    source_format: texture::ImageFormat,
    words: [u16; 7],
    values: [f32; 7],
    tolerance: f32,
) {
    use bevy::render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    };
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
    let format = gpu_image_format(source_format);
    let handle = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(empty_streamed_image(4, 1, 3, format));
    for _ in 0..8 {
        app.update();
    }
    let render = app.sub_app(RenderApp).world();
    let device = render.resource::<RenderDevice>();
    let queue = render.resource::<RenderQueue>();
    let images = render.resource::<RenderAssets<GpuImage>>();
    let image = images.get(handle.id()).unwrap();
    let uploads = [
        (0, 4, &words[..4]),
        (1, 2, &words[4..6]),
        (2, 1, &words[6..]),
    ]
    .map(|(mip_level, width, values)| CompressedMipUpload {
        mip_level,
        width,
        height: 1,
        payload: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
    });
    upload_compressed_mips(device, queue, image, None, GpuMipCopy::None, &uploads).unwrap();
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("half float lookup sampling"),
        source: ShaderSource::Wgsl(
            r#"
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var<storage,read_write> output: array<vec4<f32>>;
@compute @workgroup_size(8) fn main(@builtin(global_invocation_id) gid:vec3<u32>) {
    let id=gid.x;
    if id<4u {output[id]=textureLoad(image,vec2<i32>(i32(id),0),0);}
    else if id<6u {output[id]=textureLoad(image,vec2<i32>(i32(id-4u),0),1);}
    else if id==6u {output[id]=textureLoad(image,vec2<i32>(0),2);}
    else {output[id]=textureSampleLevel(image,linear_sampler,vec2<f32>(.5),0.0);}
}
"#
            .into(),
        ),
    });
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
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 128,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 128,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let sampler = device.create_sampler(&SamplerDescriptor {
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        ..default()
    });
    let group = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&image.texture_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&sampler),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: output.as_entire_binding(),
                },
            ],
        });
    let mut encoder = device.create_command_encoder(&default());
    {
        let mut pass = encoder.begin_compute_pass(&default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 128);
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
        .unwrap();
    let mapped = readback.slice(..).get_mapped_range();
    let actual = mapped
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect::<Vec<_>>();
    for id in 0..8 {
        let red = if id < 7 {
            values[id]
        } else {
            (values[1] + values[2]) * 0.5
        };
        let expected = [red, 0., 0., 1.];
        for channel in 0..4 {
            assert!(
                actual[id * 4 + channel].is_finite()
                    && (actual[id * 4 + channel] - expected[channel]).abs() <= tolerance,
                "{source_format:?} mip/sample{id} channel{channel}: {} != {}",
                actual[id * 4 + channel],
                expected[channel]
            );
        }
    }
    drop(mapped);
    readback.unmap();
}
