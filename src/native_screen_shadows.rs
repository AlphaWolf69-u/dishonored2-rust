//! Original screenShadows producer and bilateral X/Y programs.
//!
//! Evidence: installed renderprog/screenshadows.decl and its bilateralBlur
//! declarations, shadowCommon::GetFastCloudShadowing, helper::B8, and executable
//! callback FUN_14032c7f0. This submits the owner's passes; it does not guess the
//! output alias, channel masks, jitter variant, original depth convention or
//! blur parameters. All targets and inputs stay on the GPU.

use bevy::{
    color::LinearRgba,
    math::{UVec4, Vec4},
    render::{render_resource::*, renderer::RenderDevice},
};

/// Rewrite-authored transport of original named render parameters, not an
/// assertion about the executable's constant-buffer binary layout.
#[derive(Clone, Debug, ShaderType)]
pub struct NativeScreenShadowUniform {
    pub inverse_view_projection: [Vec4; 4],
    /// -inverseProjection[2].w, inverseProjection[3].z, [3].w, unused.
    pub unproject: Vec4,
    pub camera_time: Vec4,
    /// resolutionScale.xy, screenShadows/stepSize, modelDepthHack.x.
    pub depth_scale_step_hack: Vec4,
    /// target width/height, integer blur radius, axis (0=X, 1=Y).
    pub target: UVec4,
    /// Original bilateral falloff and sharpness.
    pub blur: Vec4,
    /// cloudAltitude, cloudRotation, cloudSpeedX, cloudSpeedY.
    pub cloud_motion: Vec4,
    /// cloudTilingX, cloudTilingY, cloudDensity, unused.
    pub cloud_density: Vec4,
    /// useJitter, parallelLightHasShadow, original cloud compile branch, unused.
    pub options: UVec4,
}

impl NativeScreenShadowUniform {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .inverse_view_projection
            .iter()
            .chain([
                &self.unproject,
                &self.camera_time,
                &self.depth_scale_step_hack,
                &self.blur,
                &self.cloud_motion,
                &self.cloud_density,
            ])
            .any(|v| v.to_array().iter().any(|x| !x.is_finite()))
        {
            return Err("nonfinite original screen-shadow parameter".into());
        }
        if self.target.x == 0
            || self.target.y == 0
            || self.target.w > 1
            || self.target.x > i32::MAX as u32
            || self.target.y > i32::MAX as u32
            || self.target.z > i32::MAX as u32 / 2
            || self.depth_scale_step_hack.x <= 0.0
            || self.depth_scale_step_hack.y <= 0.0
            || self.options.x > 1
            || self.options.y > 1
            || self.options.z > 1
        {
            return Err("invalid original screen-shadow target or compile choices".into());
        }
        Ok(())
    }
}

/// Resolved state of one of FUN_14032c7f0's stencil-partitioned draws.
/// The native factories are verified against the executable in render_state.rs.
pub struct NativeScreenProducerState {
    pub color: ColorTargetState,
    pub depth: DepthStencilState,
    pub stencil_reference: u32,
}

pub fn screen_producer_states(
    source: crate::render_state::NativeRenderState,
    color_format: TextureFormat,
    depth_format: TextureFormat,
) -> Result<[NativeScreenProducerState; 2], String> {
    use crate::render_state::{native_blend_state, screen_shadow_pass_states};
    if !depth_format.has_stencil_aspect() {
        return Err("native screen-shadow producer requires the original stencil partition".into());
    }
    let make = |state: crate::render_state::NativeRenderState| -> Result<NativeScreenProducerState, String> {
        let blend = native_blend_state(&state).targets[0].unwrap();
        Ok(NativeScreenProducerState {
            color: ColorTargetState { format: color_format, blend: None,
                write_mask: ColorWrites::from_bits_truncate(u32::from(blend.write_mask)) },
            depth: crate::native_shadow_gpu::native_depth_stencil_gpu(&state, depth_format)?,
            stencil_reference: 1,
        })
    };
    let [first, second] = screen_shadow_pass_states(source);
    Ok([make(first)?, make(second)?])
}

/// Persistent pipelines specialized by the actual owner target/depth state.
/// The native callback writes only G, using stencil == 1 then != 1; AO and
/// packed depth survive both draws. No depth/stencil-free live substitution.
pub struct NativeScreenShadowPipelines {
    pub layout: BindGroupLayout,
    pub visibility: RenderPipeline,
    pub bilateral: RenderPipeline,
    module: ShaderModule,
}

impl NativeScreenShadowPipelines {
    pub fn new(
        device: &RenderDevice,
        visibility_target: ColorTargetState,
        visibility_depth: Option<DepthStencilState>,
        bilateral_target: ColorTargetState,
    ) -> Self {
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("original screen shadow reconstruction and bilateral programs"),
            source: ShaderSource::Wgsl(
                format!(
                    "{}\n{}\n{}",
                    include_str!("../assets/shaders/dh2_shadow.wgsl"),
                    include_str!("../assets/shaders/dh2_screen_shadows.wgsl"),
                    include_str!("../assets/shaders/dh2_dwao_shadow_blur.wgsl")
                )
                .into(),
            ),
        });
        let stage = ShaderStages::FRAGMENT;
        let texture = |binding, sample_type, view_dimension| BindGroupLayoutEntry {
            binding,
            visibility: stage,
            ty: BindingType::Texture {
                sample_type,
                view_dimension,
                multisampled: false,
            },
            count: None,
        };
        let sampler = |binding, kind| BindGroupLayoutEntry {
            binding,
            visibility: stage,
            ty: BindingType::Sampler(kind),
            count: None,
        };
        let layout = device.create_bind_group_layout(
            "original screen shadow inputs",
            &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: stage,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: BufferSize::new(432),
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::VERTEX_FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: BufferSize::new(192),
                    },
                    count: None,
                },
                texture(2, TextureSampleType::Depth, TextureViewDimension::D2),
                sampler(3, SamplerBindingType::NonFiltering),
                texture(4, TextureSampleType::Depth, TextureViewDimension::D2Array),
                sampler(5, SamplerBindingType::Comparison),
                texture(
                    6,
                    TextureSampleType::Float { filterable: true },
                    TextureViewDimension::D2,
                ),
                sampler(7, SamplerBindingType::Filtering),
                texture(
                    8,
                    TextureSampleType::Float { filterable: false },
                    TextureViewDimension::D2,
                ),
            ],
        );
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("original screen shadow pass layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry, target, depth| {
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: RawVertexState {
                    module: &module,
                    entry_point: Some("screen_vertex"),
                    compilation_options: PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: PrimitiveState {
                    topology: PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: depth,
                multisample: MultisampleState::default(),
                fragment: Some(RawFragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: PipelineCompilationOptions::default(),
                    targets: &[Some(target)],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let visibility = pipeline("screen_visibility", visibility_target, visibility_depth);
        let bilateral = pipeline("screen_bilateral", bilateral_target, None);
        Self {
            layout,
            visibility,
            bilateral,
            module,
        }
    }

    /// Bind the original owner's existing views/buffers, never a CPU depth readback.
    pub fn bind(
        &self,
        device: &RenderDevice,
        parallel: &Buffer,
        parameters: &Buffer,
        depth: &TextureView,
        point: &Sampler,
        atlas: &TextureView,
        comparison: &Sampler,
        cloud: &TextureView,
        cloud_sampler: &Sampler,
        source: &TextureView,
    ) -> BindGroup {
        device.create_bind_group(
            "original screen shadow pass resources",
            &self.layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: parallel.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: parameters.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(depth),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::Sampler(point),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: BindingResource::TextureView(atlas),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: BindingResource::Sampler(comparison),
                },
                BindGroupEntry {
                    binding: 6,
                    resource: BindingResource::TextureView(cloud),
                },
                BindGroupEntry {
                    binding: 7,
                    resource: BindingResource::Sampler(cloud_sampler),
                },
                BindGroupEntry {
                    binding: 8,
                    resource: BindingResource::TextureView(source),
                },
            ],
        )
    }

    /// Load/store/clear and viewport belong to the native cache/render-task owner.
    /// Bindings are reusable while the views remain unchanged.
    pub fn draw(
        &self,
        encoder: &mut CommandEncoder,
        bilateral: bool,
        bindings: &BindGroup,
        destination: &TextureView,
        depth: Option<&TextureView>,
        stencil_reference: u32,
        operations: Operations<LinearRgba>,
        viewport: [f32; 6],
    ) {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some(if bilateral {
                "native screen shadow bilateral"
            } else {
                "native screen shadow visibility"
            }),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: match operations.load {
                        LoadOp::Clear(color) => LoadOp::Clear(color.into()),
                        LoadOp::Load => LoadOp::Load,
                        LoadOp::DontCare(token) => LoadOp::DontCare(token),
                    },
                    store: operations.store,
                },
            })],
            depth_stencil_attachment: depth.map(|view| RenderPassDepthStencilAttachment {
                view,
                depth_ops: None,
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(if bilateral {
            &self.bilateral
        } else {
            &self.visibility
        });
        pass.set_bind_group(0, bindings, &[]);
        pass.set_stencil_reference(stencil_reference);
        pass.set_viewport(
            viewport[0],
            viewport[1],
            viewport[2],
            viewport[3],
            viewport[4],
            viewport[5],
        );
        pass.draw(0..3, 0..1);
    }
}

/// Original FUN_140334e40 offset publication for the actual PostFX_DWAOBlur
/// program. The shader's Gaussian weights are fixed in its declaration;
/// DWAO/BlurWeights and centre-weight parameters are unused there.
#[derive(Clone, Debug, ShaderType)]
pub struct NativeDwaoBlurUniform {
    pub offset0: Vec4,
    pub offset1: Vec4,
    pub parameters: Vec4,
}

impl NativeDwaoBlurUniform {
    pub fn from_owner(
        extent: [f32; 2],
        direction: [f32; 2],
        radius: f32,
        depth_threshold: f32,
        center_weight: f32,
    ) -> Result<Self, String> {
        if extent
            .into_iter()
            .chain(direction)
            .chain([radius, depth_threshold, center_weight])
            .any(|v| !v.is_finite())
            || extent.iter().any(|v| *v <= 0.0)
        {
            return Err("invalid original DWAO blur owner parameters".into());
        }
        let x = direction[0] / extent[0];
        let y = direction[1] / extent[1];
        Ok(Self {
            offset0: Vec4::new(
                x * -2.0 * radius,
                y * -2.0 * radius,
                x * -1.0 * radius,
                y * -1.0 * radius,
            ),
            offset1: Vec4::new(x * radius, y * radius, x * 2.0 * radius, y * 2.0 * radius),
            parameters: Vec4::new(depth_threshold, center_weight, 0.0, 0.0),
        })
    }
}

/// Default screen-shadow callback's depth-rejecting AO/shadow Gaussian blur.
/// This is DISTINCT from screenshadows/bilateralblur. The original program
/// enables depth testing; its resolved depth state/view are mandatory inputs.
pub struct NativeDwaoBlurPipeline {
    layout: BindGroupLayout,
    pipeline: RenderPipeline,
}

impl NativeDwaoBlurPipeline {
    pub fn new(
        device: &RenderDevice,
        shared: &NativeScreenShadowPipelines,
        target: ColorTargetState,
        depth: DepthStencilState,
    ) -> Self {
        let layout = device.create_bind_group_layout(
            "native DWAO blur offsets",
            &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX_FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(48),
                },
                count: None,
            }],
        );
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("native depth-rejecting screen shadow blur"),
            bind_group_layouts: &[Some(&shared.layout), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("PostFX_DWAOBlur original program"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &shared.module,
                entry_point: Some("screen_dwao_vertex"),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth),
            multisample: MultisampleState::default(),
            fragment: Some(RawFragmentState {
                module: &shared.module,
                entry_point: Some("screen_dwao_blur"),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(target)],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { layout, pipeline }
    }

    pub fn bind(&self, device: &RenderDevice, parameters: &Buffer) -> BindGroup {
        device.create_bind_group(
            "original DWAO offset publication",
            &self.layout,
            &[BindGroupEntry {
                binding: 0,
                resource: parameters.as_entire_binding(),
            }],
        )
    }

    pub fn draw(
        &self,
        encoder: &mut CommandEncoder,
        common: &BindGroup,
        parameters: &BindGroup,
        destination: &TextureView,
        depth: &TextureView,
        operations: Operations<LinearRgba>,
        viewport: [f32; 6],
    ) {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("original depth-rejecting AO and shadow blur"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: match operations.load {
                        LoadOp::Clear(color) => LoadOp::Clear(color.into()),
                        LoadOp::Load => LoadOp::Load,
                        LoadOp::DontCare(token) => LoadOp::DontCare(token),
                    },
                    store: operations.store,
                },
            })],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: None,
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, common, &[]);
        pass.set_bind_group(1, parameters, &[]);
        pass.set_viewport(
            viewport[0],
            viewport[1],
            viewport[2],
            viewport[3],
            viewport[4],
            viewport[5],
        );
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
#[path = "native_screen_shadows_tests.rs"]
mod tests;
