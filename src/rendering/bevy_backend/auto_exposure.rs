//! Original PostFX/Luminance histogram, scrolling-window selection and adaptation.
use bevy::{
    core_pipeline::{tonemapping::tonemapping, Core3d, Core3dSystems},
    prelude::*,
    render::{
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        globals::{GlobalsBuffer, GlobalsUniform},
        render_resource::*,
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        view::{ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms},
        Render, RenderApp, RenderStartup, RenderSystems,
    },
};
use std::collections::HashMap;
#[derive(Component, Clone, Copy, ExtractComponent, ShaderType, Debug)]
#[require(bevy::camera::Hdr)]
pub struct NativeExposure {
    pub range: Vec4,      // min/max world luminance, clip shadows/highlights
    pub adaptation: Vec4, // EV, white level, speed up/down
    pub camera: Vec4,     // reference camera FPS, F-stop bias, custom EV
}
impl NativeExposure {
    pub fn from_parameters(
        p: &crate::entities::EnvironmentRenderParameters,
    ) -> Result<Self, String> {
        let scalar =
            |name: &str, default: f32| p.scalar_parameter(name).map(|v| v.unwrap_or(default));
        let settings = Self {
            range: Vec4::new(
                scalar("env/autoexp/adapt_min_luminance", 0.03)?,
                scalar("env/autoexp/adapt_max_luminance", 2000.)?,
                scalar("env/autoexp/clip_shadows", 0.)?,
                scalar("env/autoexp/clip_highlights", 1.)?,
            ),
            adaptation: Vec4::new(
                scalar("env/autoexp/ev", 0.)?,
                scalar("postfx/luminance/white_level", 1.)?,
                scalar("env/autoexp/adapt_speed_up", 0.99)?,
                scalar("env/autoexp/adapt_speed_down", 0.99)?,
            ),
            camera: Vec4::new(
                scalar("env/autoexp/reference_camera_fps", 30.)?,
                scalar("env/autoexp/fstop_bias", 0.)?,
                scalar("postfx/luminance/customevbias", 0.)?,
                0.,
            ),
        };
        if !settings.range.is_finite()
            || !settings.adaptation.is_finite()
            || !settings.camera.is_finite()
            || settings.range.x <= 0.
            || settings.range.y <= 0.
            || settings.adaptation.y <= 0.
            || settings.camera.x <= 0.
        {
            return Err("invalid authored exposure parameters".into());
        }
        Ok(settings)
    }
}
pub struct NativeExposurePlugin;
#[derive(Resource)]
struct ShaderHandle(Handle<Shader>);
#[derive(Resource)]
struct Pipelines {
    layout: BindGroupLayoutDescriptor,
    histogram: CachedComputePipelineId,
    adapt: CachedComputePipelineId,
}
#[derive(Resource, Default)]
struct Views(HashMap<Entity, GpuView>);
struct GpuView {
    settings: UniformBuffer<NativeExposure>,
    histogram: Buffer,
    state: Buffer,
    tick: u64,
    start: std::time::Instant,
}
#[derive(Resource, Default)]
struct TraceCopies(Vec<(Buffer, u64, f64, std::path::PathBuf)>);
impl Plugin for NativeExposurePlugin {
    fn build(&self, app: &mut App) {
        let shader = app
            .world_mut()
            .resource_mut::<Assets<Shader>>()
            .add(Shader::from_wgsl(
                format!(
                    "{}\n{}",
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/assets/shaders/dh2_exposure_math.wgsl"
                    )),
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/assets/shaders/dh2_exposure.wgsl"
                    ))
                ),
                "shaders/dh2_exposure.wgsl",
            ));
        app.add_plugins(ExtractComponentPlugin::<NativeExposure>::default());
        let Some(render) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render
            .insert_resource(ShaderHandle(shader))
            .init_resource::<Views>()
            .init_resource::<TraceCopies>()
            .add_systems(RenderStartup, initialize)
            .add_systems(Render, map_trace.in_set(RenderSystems::Cleanup))
            .add_systems(
                Core3d,
                expose
                    .in_set(Core3dSystems::PostProcess)
                    .before(tonemapping),
            );
    }
}
fn initialize(mut commands: Commands, shader: Res<ShaderHandle>, cache: Res<PipelineCache>) {
    use binding_types::*;
    let layout = BindGroupLayoutDescriptor::new(
        "native exposure",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<GlobalsUniform>(false),
                uniform_buffer::<NativeExposure>(false),
                texture_2d(TextureSampleType::Float { filterable: false }),
                storage_buffer_sized(false, std::num::NonZeroU64::new(512)),
                storage_buffer_sized(false, std::num::NonZeroU64::new(32)),
                storage_buffer::<ViewUniform>(true),
            ),
        ),
    );
    let make = |entry: &str| {
        cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(format!("native exposure {entry}").into()),
            layout: vec![layout.clone()],
            shader: shader.0.clone(),
            entry_point: Some(entry.to_owned().into()),
            ..default()
        })
    };
    commands.insert_resource(Pipelines {
        histogram: make("histogram"),
        adapt: make("adapt"),
        layout,
    });
}
fn expose(
    view: ViewQuery<(
        &ViewUniformOffset,
        &ViewTarget,
        &NativeExposure,
        &bevy::render::camera::ExtractedCamera,
    )>,
    mut views: ResMut<Views>,
    pipelines: Res<Pipelines>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    uniforms: Res<ViewUniforms>,
    globals: Res<GlobalsBuffer>,
    mut copies: ResMut<TraceCopies>,
    mut context: RenderContext,
) {
    let entity = view.entity();
    let (offset, target, settings, camera) = view.into_inner();
    assert!(
        camera.hdr,
        "native exposure requires an HDR camera before display tonemapping"
    );
    let (Some(histogram), Some(adapt)) = (
        cache.get_compute_pipeline(pipelines.histogram),
        cache.get_compute_pipeline(pipelines.adapt),
    ) else {
        return;
    };
    let gpu = views.0.entry(entity).or_insert_with(|| {
        eprintln!(
            "Native exposure HDR pipeline online: {:?}",
            target.main_texture().format()
        );
        GpuView {
            settings: UniformBuffer::from(*settings),
            histogram: device.create_buffer(&BufferDescriptor {
                label: Some("native exposure histogram"),
                size: 512,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            // FUN_140370dc0, flags0xb: engine and luminance factors1, other words0.
            state: device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("native exposure state"),
                contents: &[1.0_f32, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
                    .collect::<Vec<_>>(),
                usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            }),
            tick: 0,
            start: std::time::Instant::now(),
        }
    });
    gpu.settings.set(*settings);
    gpu.settings.write_buffer(&device, &queue);
    let Some(view_buffer) = uniforms.uniforms.buffer() else {
        return;
    };
    let group = device.create_bind_group(
        None,
        &cache.get_bind_group_layout(&pipelines.layout),
        &BindGroupEntries::sequential((
            &globals.buffer,
            &gpu.settings,
            target.main_texture_view(),
            gpu.histogram.as_entire_buffer_binding(),
            gpu.state.as_entire_buffer_binding(),
            BufferBinding {
                buffer: view_buffer,
                size: Some(ViewUniform::min_size()),
                offset: 0,
            },
        )),
    );
    let width = target.main_texture().width().div_ceil(2).div_ceil(2);
    let height = target.main_texture().height().div_ceil(2).div_ceil(2);
    {
        let mut pass = context
            .command_encoder()
            .begin_compute_pass(&ComputePassDescriptor {
                label: Some("native exposure"),
                ..default()
            });
        pass.set_bind_group(0, &group, &[offset.offset]);
        pass.set_pipeline(histogram);
        pass.dispatch_workgroups(width.div_ceil(16), height.div_ceil(16), 1);
        pass.set_pipeline(adapt);
        pass.dispatch_workgroups(1, 1, 1);
    }
    gpu.tick += 1;
    if let Some(path) = std::env::var_os("DH2_TRACE_EXPOSURE") {
        if gpu.tick <= 30 || gpu.tick % 60 == 0 {
            let staging = device.create_buffer(&BufferDescriptor {
                label: Some("native exposure numerical trace"),
                size: 32,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            context
                .command_encoder()
                .copy_buffer_to_buffer(&gpu.state, 0, &staging, 0, 32);
            copies.0.push((
                staging,
                gpu.tick,
                gpu.start.elapsed().as_secs_f64(),
                path.into(),
            ));
        }
    }
}
fn map_trace(mut copies: ResMut<TraceCopies>) {
    for (buffer, tick, seconds, path) in copies.0.drain(..) {
        let callback = buffer.clone();
        buffer.slice(..).map_async(MapMode::Read, move |result| {
            if result.is_err() {
                return;
            }
            let data = callback.slice(..).get_mapped_range();
            let words = data
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect::<Vec<_>>();
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(file, "{tick} {seconds:.6} {words:?}");
            }
            drop(data);
            callback.unmap();
        });
    }
}

#[cfg(test)]
pub fn verify_gpu_math(app: &mut App) {
    let render = app.sub_app(RenderApp).world();
    let device = render.resource::<RenderDevice>();
    let queue = render.resource::<RenderQueue>();
    let source = format!(
        "{}\n{}",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/shaders/dh2_exposure_math.wgsl"
        )),
        r#"
 @group(0) @binding(0) var<storage,read_write> output:array<vec4<f32>>;
 @compute @workgroup_size(1) fn main(@builtin(global_invocation_id) tid:vec3<u32>) {
 let id=tid.x;var h:array<u32,128>;for(var b=0u;b<128u;b++) {h[b]=select(select((b*13u+id*7u)%31u,1000u,b==(id*17u)%128u),0u,id==0u);}
 let p=Settings(vec4(select(0.03,1.0,(id&1u)!=0u),select(2000.0,500.0,(id&1u)!=0u),f32(id%4u)*0.1,1.0-f32(id%3u)*0.1),
 vec4(f32(i32(id%7u)-3)*0.5,f32(id%3u)+1.0,0.99,0.8),vec4(30.0,0.0,0.0,0.0));
 let r=native_adapt(h,p,f32(id%6u)*0.5,f32(id%5u+1u)*0.016);
 output[id*2u]=vec4(r.engine,r.luminance,r.min_ldr,r.max_ldr);output[id*2u+1u]=vec4(r.middle,r.ev,r.fstop,f32(r.peak));
 }"#
    );
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("native exposure parity"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
    let size = 64 * 32;
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[BindGroupEntry {
                binding: 0,
                resource: output.as_entire_binding(),
            }],
        });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(64, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(20)),
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    let expected = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/exposure_cases.txt"
    ))
    .split_whitespace()
    .map(|s| f32::from_bits(u32::from_str_radix(s, 16).unwrap()))
    .collect::<Vec<_>>();
    for (i, (chunk, e)) in bytes.chunks_exact(4).zip(expected).enumerate() {
        let actual = f32::from_le_bytes(chunk.try_into().unwrap());
        assert!(
            (actual - e).abs() < 0.00003 * e.abs().max(1.0),
            "native exposure case{} field{}: {actual} != {e}",
            i / 8,
            i % 8
        );
    }
    eprintln!("GPU native exposure:64 installed-HLSL histogram/window/adaptation cases passed");
}
