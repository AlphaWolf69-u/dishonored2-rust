//! arkSSSBlur, the skin program's sssProgram. FUN_140311aa0 (the forward
//! dynamic pass) draws the sssSurf list (0x1c320d84) into hdrBuffer and
//! sssBuffer together, then redraws the same list with each material's
//! MPR_SSS program reading sssBuffer as textureMap (blend ONE ONE, depth
//! EQUAL), before the FOG pass. sssBuffer is the renderer's _HDRBuffer2
//! (FUN_14036d0c0 +0x90), which the CLEAR pass does not clear.
//!
//! Host integration: the main opaque pass binds the other Bevy main texture
//! as sssBuffer and a per-pixel sssSurface target (see bevy_core_pipeline
//! `ViewSssSurfaceTexture`); this screen pass blurs exactly the pixels a
//! skin program wrote, which are the pixels the game's EQUAL redraw touches.
use bevy::{
    core_pipeline::{
        core_3d::{main_opaque_pass_3d, ViewSssSurfaceTexture},
        Core3d, Core3dSystems,
    },
    prelude::*,
    render::{
        camera::ExtractedCamera,
        render_resource::*,
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        view::{ExtractedView, ViewTarget},
        RenderApp,
    },
};

/// Registered SSS/BlurQuality default (renderParm `{ uniform uint 0 }`): the
/// original 92-tap point-sampled kernel. r_SSSBlurQuality also defaults to 0.
pub const BLUR_QUALITY: u32 = 0;

/// resolutionScale.xy / globalViewInfos.xy. A captured PerViewCB holds
/// globalViewInfos = (1/P00, 1/P11, near, far); the rewrite renders at full
/// target resolution (resolutionScale 1).
pub fn kernel_scale(clip_from_view: Mat4) -> Vec2 {
    Vec2::new(clip_from_view.x_axis.x, clip_from_view.y_axis.y)
}

#[derive(Resource, Default)]
struct SssBlurGpu {
    pipeline: Option<(TextureFormat, RenderPipeline)>,
    parameters: Option<Buffer>,
    samplers: Option<(Sampler, Sampler)>,
}

pub(super) fn register(app: &mut App) {
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.init_resource::<SssBlurGpu>().add_systems(
            Core3d,
            render_sss_blur
                .in_set(Core3dSystems::MainPass)
                .after(main_opaque_pass_3d)
                .before(super::fog_scene::AtmospherePass),
        );
    }
}

fn create_pipeline(device: &RenderDevice, format: TextureFormat) -> RenderPipeline {
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("arkSSSBlur"),
        source: ShaderSource::Wgsl(
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/shaders/dh2_sss_blur.wgsl"))
                .into(),
        ),
    });
    let add = BlendComponent {
        src_factor: BlendFactor::One,
        dst_factor: BlendFactor::One,
        operation: BlendOperation::Add,
    };
    device.create_render_pipeline(&RawRenderPipelineDescriptor {
        label: Some("arkSSSBlur"),
        layout: None,
        vertex: RawVertexState {
            module: &module,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(RawFragmentState {
            module: &module,
            entry_point: Some("blur"),
            compilation_options: Default::default(),
            // hdrBuffer is R11G11B10 in the game: blending touches colour only.
            targets: &[Some(ColorTargetState {
                format,
                blend: Some(BlendState { color: add, alpha: add }),
                write_mask: ColorWrites::COLOR,
            })],
        }),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn render_sss_blur(
    view: ViewQuery<(
        &ViewTarget,
        &ExtractedView,
        &ExtractedCamera,
        Option<&ViewSssSurfaceTexture>,
    )>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut gpu: ResMut<SssBlurGpu>,
    mut ctx: RenderContext,
) {
    let (target, extracted, camera, surface) = view.into_inner();
    let Some(surface) = surface else { return };
    let Some(size) = camera.physical_target_size else { return };
    let format = target.main_texture_format();
    if gpu.pipeline.as_ref().is_none_or(|(f, _)| *f != format) {
        gpu.pipeline = Some((format, create_pipeline(&device, format)));
    }
    let parameters = gpu.parameters.get_or_insert_with(|| {
        device.create_buffer(&BufferDescriptor {
            label: Some("arkSSSBlur parameters"),
            size: 32,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    });
    let scale = kernel_scale(extracted.clip_from_view);
    let mut bytes = Vec::with_capacity(32);
    for value in [scale.x, scale.y, 1.0 / size.x as f32, 1.0 / size.y as f32] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in [BLUR_QUALITY, 0, 0, 0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    queue.write_buffer(parameters, 0, &bytes);
    let parameters = parameters.clone();
    let (point, bilinear) = gpu.samplers.get_or_insert_with(|| {
        let sampler = |filter, label| {
            device.create_sampler(&SamplerDescriptor {
                label: Some(label),
                address_mode_u: AddressMode::ClampToEdge,
                address_mode_v: AddressMode::ClampToEdge,
                address_mode_w: AddressMode::ClampToEdge,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: MipmapFilterMode::Nearest,
                ..Default::default()
            })
        };
        (sampler(FilterMode::Nearest, "pointSampler"), sampler(FilterMode::Linear, "bilinearClamp"))
    });
    let (point, bilinear) = (point.clone(), bilinear.clone());
    let pipeline = &gpu.pipeline.as_ref().unwrap().1;
    let binding: BindGroup = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: Some("arkSSSBlur inputs"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(target.main_texture_other_view()),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&surface.texture.default_view),
                },
                BindGroupEntry { binding: 2, resource: BindingResource::Sampler(&point) },
                BindGroupEntry { binding: 3, resource: BindingResource::Sampler(&bilinear) },
                BindGroupEntry { binding: 4, resource: parameters.as_entire_binding() },
            ],
        })
        .into();
    let encoder = ctx.command_encoder();
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("arkSSSBlur"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: target.main_texture_view(),
            depth_slice: None,
            resolve_target: None,
            ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &binding, &[]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    #[test]
    fn kernel_scale_inverts_captured_global_view_infos() {
        // Captured PerViewCB: projection P00/P11 and globalViewInfos.xy.
        let projection = bevy::prelude::Mat4::from_cols_array(&[
            0.6998658, 0.0, 0.0, 0.0, 0.0, 1.2448134, 0.0, 0.0, 0.0, 0.0, 1.4285733e-6, -1.0, 0.0,
            0.0, 0.010000014, 0.0,
        ]);
        let scale = super::kernel_scale(projection);
        assert!((1.0 / scale.x - 1.4288454).abs() < 1e-5);
        assert!((1.0 / scale.y - 0.8033332).abs() < 1e-5);
    }
}
