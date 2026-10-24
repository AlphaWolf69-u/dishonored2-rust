//! Live screen-shadow owner. The source-backed material Z branch establishes
//! opaque/alpha depth before screen visibility and forward lighting.
use crate::{
    screen_shadow_pipeline::*, shadow_pass::*, shadow_sampling::ParallelSamplingBuffer,
};
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::{
    core_pipeline::{core_3d::main_opaque_pass_3d, Core3d, Core3dSystems},
    prelude::*,
    render::{
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderContext, RenderDevice, RenderQueue},
        storage::GpuShaderBuffer,
        texture::GpuImage,
        view::{ExtractedView, ViewDepthTexture},
        RenderApp,
    },
};

#[derive(Resource, Clone, ExtractResource)]
pub struct ScreenShadowFrame {
    pub output: Handle<Image>,
    pub cloud: Handle<Image>,
    pub depth_default: Option<Handle<Image>>,
    pub current: Option<ScreenShadowView>,
    pub ambient_occlusion: Option<AmbientOcclusionView>,
    /// r_playerDepthHack: the unhack factor's source (FUN_1402fcff0).
    pub player_depth_hack: f32,
    /// Ambient visibility submissions can finish before the directional atlas.
    pub produced: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub directional_produced: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

#[derive(Clone)]
pub struct ScreenShadowView {
    pub revision: u64,
    pub parameters: ScreenShadowUniform,
    pub producer_state: [u32; 9],
}

#[derive(Clone)]
pub struct AmbientOcclusionView {
    pub parameters: crate::rendering::ambient_occlusion::AmbientOcclusionParameters,
    pub extent: [u32; 2],
    pub unproject: Vec4,
    pub render_scale: [f32; 2],
    pub blur_state: [u32; 9],
}

pub fn visibility_image(width: u32, height: u32) -> Image {
    let mut image = Image::new_uninit(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba8Unorm,
        Default::default(),
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC;
    image
}

pub struct ScreenShadowScenePlugin;
impl Plugin for ScreenShadowScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractResourcePlugin::<ScreenShadowFrame>::default())
            .add_systems(
                Update,
                (ensure_screen_depth_source, capture_screen_visibility),
            );
        let Some(render) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render.init_resource::<ScreenGpu>().add_systems(
            Core3d,
            (
                clear_view_stencil.before(Core3dSystems::Prepass),
                render_screen_shadow
                    .in_set(Core3dSystems::MainPass)
                    .before(main_opaque_pass_3d),
            ),
        );
    }
}

fn ensure_screen_depth_source(
    mut commands: Commands,
    frame: Option<Res<ScreenShadowFrame>>,
    cameras: Query<
        Entity,
        (
            With<Camera3d>,
            Without<bevy::core_pipeline::prepass::DepthPrepass>,
        ),
    >,
) {
    if frame.is_none() {
        return;
    }
    for camera in &cameras {
        commands
            .entity(camera)
            .insert(bevy::core_pipeline::prepass::DepthPrepass);
    }
}

#[derive(Resource, Default)]
struct ScreenGpu {
    unhack: Option<super::depth_buffer_hack::DepthBufferHack>,
    cached: Option<ScreenTargets>,
    ambient: Option<AmbientTargets>,
    reported: bool,
}
struct AmbientTargets {
    depth_default_id: Option<TextureId>,
    depth_id: TextureId,
    output_id: TextureId,
    mode: u8,
    blur_state: [u32; 9],
    producer: super::ambient_occlusion::AmbientOcclusionPass,
    filter: super::screen_visibility::ScreenVisibilityFilter,
}
struct ScreenTargets {
    extent: [u32; 2],
    depth_id: TextureId,
    sampled_id: TextureId,
    output_id: TextureId,
    cloud_id: TextureId,
    atlas_id: TextureId,
    state: [u32; 9],
    producer: [ScreenShadowPipelines; 2],
    parameters: [Buffer; 2],
    bindings: [BindGroup; 2],
}

fn clear_view_stencil(
    frame: Option<Res<ScreenShadowFrame>>,
    views: Query<&ViewDepthTexture, With<ExtractedView>>,
    mut context: RenderContext,
) {
    if frame.is_none() {
        return;
    }
    let Ok(depth) = views.single() else {
        return;
    };
    if !depth.texture.format().has_stencil_aspect() {
        return;
    }
    // Native view depth begins with stencil zero; modelDepthHack owners mark
    // bit0 when their actual geometry is submitted in the depth prepass (the
    // first-person arms). Never synthesize a stencil-1 silhouette.
    let view = depth.texture.create_view(&TextureViewDescriptor::default());
    let _pass = context
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("view stencil initialization"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &view,
                depth_ops: None,
                stencil_ops: Some(Operations {
                    load: LoadOp::Clear(0),
                    store: StoreOp::Store,
                }),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
}

fn typed_buffer(
    device: &RenderDevice,
    value: impl Into<bevy::render::storage::ShaderBuffer>,
) -> Buffer {
    let bytes = value.into().data.expect("typed native screen transport");
    device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("screen owner parameters"),
        contents: &bytes,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    })
}

impl ScreenTargets {
    fn new(
        device: &RenderDevice,
        depth: &ViewDepthTexture,
        sampled: &Texture,
        output: &GpuImage,
        cloud: &GpuImage,
        atlas: &GpuImage,
        parallel: &Buffer,
        view: &ScreenShadowView,
        unused_source: &TextureView,
    ) -> Result<Self, String> {
        let extent = [output.texture.width(), output.texture.height()];
        if depth.texture.sample_count() != 1
            || extent != [depth.texture.width(), depth.texture.height()]
            || output.texture.format() != TextureFormat::Rgba8Unorm
        {
            return Err("screen shadows require matching single-sample current-view depth and RGBA8 output".into());
        }
        let states = screen_producer_states(
            view.producer_state,
            TextureFormat::Rgba8Unorm,
            depth.texture.format(),
        )?;
        let producer = states.map(|state| {
            ScreenShadowPipelines::new(
                device,
                state.color,
                Some(state.depth),
                ColorTargetState {
                    format: TextureFormat::Rgba16Float,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                },
            )
        });
        // screenShadows/DepthBuffer is the unhacked copy (0x21181874).
        let depth_view = sampled.create_view(&TextureViewDescriptor {
            aspect: TextureAspect::DepthOnly,
            ..Default::default()
        });
        let point = device.create_sampler(&SamplerDescriptor {
            label: Some("pointClamp"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            ..Default::default()
        });
        let comparison = crate::shadow_pipeline::shadow_comparison_sampler(device);
        let cloud_sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("cloud bilinearWrap"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });
        let parameters = std::array::from_fn(|_| typed_buffer(device, view.parameters.clone()));
        let bind = |index: usize, source: &TextureView| {
            producer[0].bind(
                device,
                parallel,
                &parameters[index],
                &depth_view,
                &point,
                &atlas.texture_view,
                &comparison,
                &cloud.texture_view,
                &cloud_sampler,
                source,
            )
        };
        // The visibility entry does not read binding8. Reuse the filter's
        // real intermediate to avoid an attachment/read alias.
        let bindings = std::array::from_fn(|i| bind(i, unused_source));
        Ok(Self {
            extent,
            depth_id: depth.texture.id(),
            sampled_id: sampled.id(),
            output_id: output.texture.id(),
            cloud_id: cloud.texture.id(),
            atlas_id: atlas.texture.id(),
            state: view.producer_state,
            producer,
            parameters,
            bindings,
        })
    }
}

fn render_screen_shadow(
    frame: Option<Res<ScreenShadowFrame>>,
    atlas_frame: Res<ShadowSceneFrame>,
    atlas_gpu: Res<ShadowSceneGpu>,
    sampling: Option<Res<ParallelSamplingBuffer>>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    images: Res<RenderAssets<GpuImage>>,
    views: Query<(
        &ViewDepthTexture,
        &ExtractedView,
        &bevy::render::camera::ExtractedCamera,
    )>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    pipelines: Res<crate::tiled_lighting_gpu::TlfPipelines>,
    mut gpu: ResMut<ScreenGpu>,
    mut context: RenderContext,
    mut last_error: Local<Option<String>>,
    mut last_ao_error: Local<Option<String>>,
    profile: Option<Res<crate::frame_profile::CpuFrameProfile>>,
) {
    let _span = profile.as_deref().map(|profile| profile.measure(42));
    let Some(frame) = frame else {
        return;
    };
    let diagnostics = context.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let span = diagnostics.time_span(context.command_encoder(), "screen_shadow_pipeline");
    let unhacked = (|| -> Result<Texture, String> {
        let (depth, _, _) = views.single().map_err(|e| e.to_string())?;
        if gpu.unhack.as_ref().is_none_or(|old| old.source_id() != depth.texture.id()) {
            gpu.unhack = None;
            gpu.unhack = Some(super::depth_buffer_hack::DepthBufferHack::new(
                &device,
                &pipelines,
                &depth.texture,
            )?);
        }
        let unhack = gpu.unhack.as_ref().unwrap();
        unhack.record(&queue, context.command_encoder(), &depth.texture, frame.player_depth_hack);
        Ok(unhack.texture.clone())
    })();
    let ambient_result = (|| -> Result<(), String> {
        let view = frame.ambient_occlusion.as_ref().ok_or("AO current view pending")?;
        let unhacked = unhacked.as_ref().map_err(Clone::clone)?;
        let (_, _, camera) = views.single().map_err(|e| e.to_string())?;
        if camera.physical_viewport_size != camera.physical_target_size
            || camera.viewport.as_ref().is_some_and(|v| v.physical_position != UVec2::ZERO) {
            return Err("AO sub-view owner is unresolved".into());
        }
        let output = images.get(frame.output.id()).ok_or("AO target pending GPU upload")?;
        if output.texture.format() != TextureFormat::Rgba8Unorm
            || view.extent != [output.texture.width(), output.texture.height()] {
            return Err("AO target does not match the current view".into());
        }
        let depth_default = frame.depth_default.as_ref().map(|image| images.get(image.id()).ok_or("AO depth initializer pending GPU upload")).transpose()?;
        let depth_default_id = depth_default.map(|image|image.texture.id());
        if gpu.ambient.as_ref().is_none_or(|old| old.depth_id != unhacked.id()
            || old.output_id != output.texture.id() || old.mode != view.parameters.mode()
            || old.blur_state != view.blur_state || old.depth_default_id != depth_default_id) {
            gpu.ambient = Some(AmbientTargets {
                depth_default_id,
                depth_id: unhacked.id(), output_id: output.texture.id(),
                mode: view.parameters.mode(), blur_state: view.blur_state,
                producer: super::ambient_occlusion::AmbientOcclusionPass::new(
                    &device, &pipelines, unhacked, &view.parameters)?,
                filter: super::screen_visibility::ScreenVisibilityFilter::new(
                    &device, unhacked, &output.texture_view, view.extent, view.blur_state, depth_default.map(|image| &image.texture_view))?,
            });
        }
        let cached = gpu.ambient.as_mut().unwrap();
        let encoder = context.command_encoder();
        cached.filter.pack(&queue, encoder, &output.texture_view, view.unproject, view.render_scale);
        cached.producer.record(&device, &queue, encoder, &pipelines, &cached.filter.depth,
            &output.texture_view, &view.parameters)?;
        Ok(())
    })();
    let result = (|| -> Result<(), String> {
        ambient_result.as_ref().map_err(Clone::clone)?;
        let view = frame
            .current
            .as_ref()
            .ok_or("screen CPU frame pending")?;
        if view.revision != atlas_frame.revision || !atlas_gpu.completed_revision(view.revision) {
            return Err("screen pass awaits the same-frame atlas submission".into());
        }
        let ShadowSceneState::Ready { target_image, .. } = &atlas_frame.state else {
            return Err("atlas is unavailable".into());
        };
        let (depth, _, camera) = views.single().map_err(|error| error.to_string())?;
        if camera.physical_viewport_size != camera.physical_target_size
            || camera
                .viewport
                .as_ref()
                .is_some_and(|v| v.physical_position != UVec2::ZERO)
        {
            return Err("screen sub-view owner is unresolved".into());
        }
        let output = images
            .get(frame.output.id())
            .ok_or("screen target pending GPU upload")?;
        let cloud = images
            .get(frame.cloud.id())
            .ok_or("cloud input pending GPU upload")?;
        let atlas = images
            .get(target_image.id())
            .ok_or("atlas pending GPU upload")?;
        let parallel = sampling
            .as_ref()
            .and_then(|s| buffers.get(s.buffer.id()))
            .ok_or("parallel parameters pending GPU upload")?;
        let unhacked = unhacked.as_ref().map_err(Clone::clone)?;
        if gpu.cached.as_ref().is_none_or(|old| {
            old.depth_id != depth.texture.id()
                || old.sampled_id != unhacked.id()
                || old.output_id != output.texture.id()
                || old.cloud_id != cloud.texture.id()
                || old.atlas_id != atlas.texture.id()
                || old.state != view.producer_state
        }) {
            gpu.cached = Some(ScreenTargets::new(
                &device,
                depth,
                unhacked,
                output,
                cloud,
                atlas,
                &parallel.buffer,
                view,
                gpu.ambient.as_ref().unwrap().filter.scratch(),
            )?);
        }
        let cached = gpu.cached.as_mut().unwrap();
        view.parameters.validate()?;
        let mut cloud = view.parameters.cloud_density;
        cloud.w = view.parameters.options.z as f32;
        // This trailing shared transport extends the atlas prefix without
        // duplicating cloud parameters per material. Queue writes precede this
        // frame's GPU submission, including both opaque geometry submissions.
        let tail = bevy::render::storage::ShaderBuffer::from([
            view.parameters.cloud_motion,
            cloud,
            view.parameters.camera_time,
        ]);
        queue.write_buffer(&parallel.buffer, 432, &tail.data.unwrap());
        for (i, buffer) in cached.parameters.iter().enumerate() {
            let mut value = view.parameters.clone();
            // Native second stencil draw restores modelDepthHack=1 and step=.025.
            if i == 1 {
                value.depth_scale_step_hack.w = 1.0;
                value.depth_scale_step_hack.z = 0.025;
            }
            queue.write_buffer(
                buffer,
                0,
                &bevy::render::storage::ShaderBuffer::from(value)
                    .data
                    .unwrap(),
            );
        }
        let encoder = context.command_encoder();
        let attachment = depth.texture.create_view(&TextureViewDescriptor::default());
        let viewport = [
            0.0,
            0.0,
            cached.extent[0] as f32,
            cached.extent[1] as f32,
            0.0,
            1.0,
        ];
        let load = Operations {
            load: LoadOp::Load,
            store: StoreOp::Store,
        };
        for i in 0..2 {
            cached.producer[i].draw(
                encoder,
                false,
                &cached.bindings[i],
                &output.texture_view,
                Some(&attachment),
                1,
                load,
                viewport,
            );
        }
        Ok(())
    })();
    if ambient_result.is_ok() {
        if let (Some(cached), Some(output)) = (gpu.ambient.as_ref(), images.get(frame.output.id())) {
            cached.filter.filter(context.command_encoder(), &output.texture_view);
            frame.produced.fetch_add(1, std::sync::atomic::Ordering::Release);
        }
        *last_ao_error = None;
    } else if last_ao_error.as_ref() != ambient_result.as_ref().err() {
        warn!("Ambient occlusion pending: {}", ambient_result.as_ref().unwrap_err());
        *last_ao_error = ambient_result.as_ref().err().cloned();
    }
    span.end(context.command_encoder());
    match result {
        Ok(()) => {
            frame.directional_produced.fetch_add(1, std::sync::atomic::Ordering::Release);
            if !gpu.reported {
                println!("Screen shadow producer and X/Y blur online");
                gpu.reported = true;
            }
            *last_error = None;
        }
        Err(error) => {
            // A failed screen pass must not let final lighting sample an old view.
            if let Some(buffer) = sampling.as_ref().and_then(|s| buffers.get(s.buffer.id())) {
                let neutral = bevy::render::storage::ShaderBuffer::from(
                    crate::shadow_sampling::ParallelSamplingUniform::default(),
                );
                queue.write_buffer(&buffer.buffer, 0, &neutral.data.unwrap());
            }
            if last_error.as_ref() != Some(&error) {
                warn!("Screen shadows pending: {error}");
                *last_error = Some(error);
            }
        }
    }
    if let Some(buffer) = sampling.as_ref().and_then(|s| buffers.get(s.buffer.id())) {
        queue.write_buffer(&buffer.buffer, 424, &u32::from(ambient_result.is_ok()).to_le_bytes());
    }

}

fn capture_screen_visibility(
    mut commands: Commands,
    frame: Option<Res<ScreenShadowFrame>>,
    images: Res<Assets<Image>>,
    mut requested: Local<bool>,
) {
    if *requested {
        return;
    }
    let Ok(path) = std::env::var("DH2_SHADOW_SCREEN_SNAPSHOT") else {
        return;
    };
    let Some(frame) = frame else {
        return;
    };
    if frame.produced.load(std::sync::atomic::Ordering::Acquire) < 30
        || frame.directional_produced.load(std::sync::atomic::Ordering::Acquire) < 30
    {
        return;
    }
    let Some(image) = images.get(&frame.output) else {
        return;
    };
    let extent = image.texture_descriptor.size;
    if let Some(view) = &frame.current {
        if let Err(error) = std::fs::write(
            format!("{path}.view.txt"),
            format!("{:#?}", view.parameters),
        ) {
            error!("screen diagnostic parameters: {error}");
        }
    }
    *requested = true;
    commands.spawn(bevy::render::gpu_readback::Readback::texture(frame.output.clone())).observe(
        move |event:On<bevy::render::gpu_readback::ReadbackComplete>,mut commands:Commands| {
            let pitch=(extent.width*4).div_ceil(256)*256;
            let mut values=Vec::with_capacity((extent.width*extent.height) as usize);
            for row in event.data.chunks(pitch as usize).take(extent.height as usize) {
                values.extend(row[..extent.width as usize*4].chunks_exact(4).map(|pixel|pixel[1]));
            }
            let min=values.iter().min();let max=values.iter().max();
            let shadowed=values.iter().filter(|&&v|v<255).count();
            println!("Screen visibility readback: {}x{}, range={min:?}..{max:?}, shadowed={shadowed}/{}",extent.width,extent.height,values.len());
            let result=(||->Result<(),Box<dyn std::error::Error>> {
                std::fs::write(format!("{path}.rgba.bin"),&event.data)?;
                let mut encoder=png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(&path)?),extent.width,extent.height);
                encoder.set_color(png::ColorType::Grayscale);encoder.set_depth(png::BitDepth::Eight);
                encoder.write_header()?.write_image_data(&values)?;Ok(())
            })();
            if let Err(error)=result {error!("screen diagnostic capture failed: {error}");}
            commands.entity(event.entity).despawn();
        });
}
