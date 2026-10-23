//! Camera colour blur with installed kernel data and explicit MRT/stencil state.
use crate::rendering::depth_of_field::{BlurKernel, DofPass};
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};
pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_shader_math.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_camera_effects.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_camera_blur.wgsl")
);
#[cfg(test)]
#[path = "camera_blur_tests.rs"]
mod tests;

pub struct BlurKernelBuffer {
    buffer: Buffer,
    kind: u32,
    samples: u32,
    radius: f32,
}
impl BlurKernelBuffer {
    pub fn kind(&self) -> u32 {
        self.kind
    }
    pub fn new(device: &RenderDevice, kernel: &BlurKernel) -> Self {
        let mut data: Vec<_> = kernel
            .shader_coefficients()
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        data.extend(
            kernel
                .offsets()
                .iter()
                .flat_map(|p| [p[0], p[1], 0., 0.])
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        // The fixed-size uniform matches the largest authored (60-tap) bank.
        // Procedural filters read only the folded coefficients.
        data.resize(16 + 60 * 16, 0);
        Self {
            buffer: device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("installed camera blur kernel"),
                contents: &data,
                usage: BufferUsages::UNIFORM,
            }),
            kind: u32::from(kernel.kind()),
            samples: kernel.samples(),
            radius: kernel.radius_multiplier(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct CameraBlurParameters {
    pub target_size: [f32; 4],
    pub fringing: [f32; 3],
    pub bokeh_size: f32,
    pub vignette: [f32; 4],
    pub centre: [f32; 2],
    pub resolution_scale: [f32; 2],
    pub position_to_texture: [f32; 2],
    pub use_fringing: bool,
    pub cat_eye: bool,
}
impl CameraBlurParameters {
    fn bytes(&self, pass: DofPass, kernel: &BlurKernelBuffer) -> [u8; 96] {
        let floats = self
            .target_size
            .into_iter()
            .chain(self.fringing)
            .chain([self.bokeh_size])
            .chain(self.vignette)
            .chain(self.centre)
            .chain(self.resolution_scale)
            .chain(self.position_to_texture)
            .chain([kernel.radius, 0.]);
        let pass = match pass {
            DofPass::Alpha => 0,
            DofPass::Near => 1,
            DofPass::Far => 2,
        };
        let flags = u32::from(self.use_fringing) | (u32::from(self.cat_eye) << 1);
        let mut bytes = [0; 96];
        for (i, value) in floats
            .map(f32::to_bits)
            .chain([pass, kernel.kind, kernel.samples, flags])
            .enumerate()
        {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}
pub struct CameraBlurConfig {
    pub pass: DofPass,
    pub colour_format: TextureFormat,
    pub alpha_format: Option<TextureFormat>,
    pub depth_format: Option<TextureFormat>,
    pub state: crate::render_state::RenderState,
}
pub struct CameraBlurInputs<'a> {
    /// Refraction, high mask, low mask, high colour and low colour.
    pub images: [&'a TextureView; 5],
    pub kernel: &'a BlurKernelBuffer,
}
pub struct CameraBlurTargets<'a> {
    pub colour: &'a TextureView,
    pub alpha: Option<&'a TextureView>,
    pub depth_stencil: Option<RenderPassDepthStencilAttachment<'a>>,
    pub stencil_reference: u32,
    pub viewport: [u32; 4],
    pub scissor: [u32; 4],
}
pub struct CameraBlurPass {
    pass: DofPass,
    depth: bool,
    pipeline: RenderPipeline,
    layout: BindGroupLayout,
    uniform: Buffer,
    clamp: Sampler,
    mirror: Sampler,
    binding: Option<(([TextureViewId; 5], BufferId), BindGroup)>,
}
impl CameraBlurPass {
    pub fn new(device: &RenderDevice, config: CameraBlurConfig) -> Result<Self, String> {
        use binding_types::*;
        let mrt = config.pass != DofPass::Far;
        if mrt != config.alpha_format.is_some() {
            return Err("camera colour blur pass and MRT targets differ".into());
        }
        let depth = crate::render_state::depth_stencil_state(&config.state);
        if config.depth_format.is_none() && (depth.depth_enabled || depth.stencil_enabled) {
            return Err("camera blur depth/stencil state requires its real attachment".into());
        }
        let layout = device.create_bind_group_layout(
            Some("camera colour blur inputs"),
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                (
                    uniform_buffer_sized(false, BufferSize::new(96)),
                    uniform_buffer_sized(false, BufferSize::new(976)),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        );
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("camera colour blur"),
            source: ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("camera colour blur"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let blend = crate::render_state::blend_state(&config.state);
        let mut targets = Vec::new();
        for (index, format) in std::iter::once(config.colour_format)
            .chain(config.alpha_format)
            .enumerate()
        {
            let target = blend.targets[if blend.independent_blend { index } else { 0 }]
                .ok_or("camera blur target state missing")?;
            targets.push(Some(ColorTargetState {
                format,
                blend: super::pipeline_state::blend_target(target)?,
                write_mask: ColorWrites::from_bits_truncate(u32::from(target.write_mask)),
            }));
        }
        let (front_face, cull_mode) = super::pipeline_state::mesh_raster(&config.state, false);
        let depth_stencil = config
            .depth_format
            .map(|format| super::pipeline_state::depth_stencil_gpu(&config.state, format))
            .transpose()?;
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("camera colour blur"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &shader,
                entry_point: Some("dof_vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &shader,
                entry_point: Some(if mrt { "dof_mrt" } else { "dof_single" }),
                compilation_options: Default::default(),
                targets: &targets,
            }),
            primitive: PrimitiveState {
                front_face,
                cull_mode,
                ..Default::default()
            },
            depth_stencil,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = |mode| {
            device.create_sampler(&SamplerDescriptor {
                min_filter: FilterMode::Linear,
                mag_filter: FilterMode::Linear,
                address_mode_u: mode,
                address_mode_v: mode,
                ..Default::default()
            })
        };
        Ok(Self {
            pass: config.pass,
            depth: config.depth_format.is_some(),
            pipeline,
            layout,
            uniform: device.create_buffer(&BufferDescriptor {
                label: Some("camera colour blur parameters"),
                size: 96,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            clamp: sampler(AddressMode::ClampToEdge),
            mirror: sampler(AddressMode::MirrorRepeat),
            binding: None,
        })
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: &CameraBlurParameters,
        inputs: CameraBlurInputs<'_>,
        targets: CameraBlurTargets<'_>,
    ) -> Result<(), String> {
        if (self.pass != DofPass::Far) != targets.alpha.is_some()
            || self.depth != targets.depth_stencil.is_some()
        {
            return Err("camera blur attachments differ from prepared pipeline".into());
        }
        if inputs.images.iter().any(|image| {
            image.id() == targets.colour.id() || targets.alpha.is_some_and(|a| image.id() == a.id())
        }) {
            return Err("camera blur source aliases output".into());
        }
        queue.write_buffer(
            &self.uniform,
            0,
            &parameters.bytes(self.pass, inputs.kernel),
        );
        let key = (
            inputs.images.map(TextureView::id),
            inputs.kernel.buffer.id(),
        );
        if self.binding.as_ref().is_none_or(|b| b.0 != key) {
            self.binding = Some((
                key,
                device.create_bind_group(
                    Some("camera colour blur sources"),
                    &self.layout,
                    &BindGroupEntries::sequential((
                        self.uniform.as_entire_binding(),
                        inputs.kernel.buffer.as_entire_binding(),
                        inputs.images[0],
                        inputs.images[1],
                        inputs.images[2],
                        inputs.images[3],
                        inputs.images[4],
                        &self.clamp,
                        &self.mirror,
                    )),
                ),
            ));
        }
        let attachments = [Some(targets.colour), targets.alpha].map(|view| {
            view.map(|view| RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })
        });
        let attachment_count = if self.pass == DofPass::Far { 1 } else { 2 };
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("camera colour blur"),
            color_attachments: &attachments[..attachment_count],
            depth_stencil_attachment: targets.depth_stencil,
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding.as_ref().unwrap().1, &[]);
        pass.set_stencil_reference(targets.stencil_reference);
        let v = targets.viewport;
        let s = targets.scissor;
        pass.set_viewport(v[0] as f32, v[1] as f32, v[2] as f32, v[3] as f32, 0., 1.);
        pass.set_scissor_rect(s[0], s[1], s[2], s[3]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}
