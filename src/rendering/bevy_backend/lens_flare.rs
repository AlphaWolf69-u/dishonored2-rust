//! 3263e0: threshold, ghost/halo generation and incremental H/V blur.
//! The caller supplies hrBufferDiv2 and the real colour-profile pair. This
//! owner must not substitute a luminance texture for that HDR resource.
use bevy::{
    math::Vec4,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};
use std::sync::Arc;

pub const SHADER_SOURCE: &str = include_str!("../../../assets/shaders/dh2_lens_flare.wgsl");
#[derive(Clone, Copy, Default, ShaderType)]
pub struct FlareParameters {
    /// Inverse viewport size, resolution scale.
    pub view: Vec4,
    /// Downscale/TexelSize, threshold bias and threshold scale.
    pub threshold: Vec4,
    /// Dispersal, distortion, halo width and halo weight.
    pub ghost: Vec4,
    /// Profile blend, integer blur radius, integer ghost count, BW program.
    pub controls: Vec4,
    pub direction: Vec4,
}
#[derive(Clone, Copy)]
pub enum FlareStage {
    Threshold = 0,
    Generate = 1,
    Blur = 2,
}
struct Pipelines {
    layout: BindGroupLayout,
    draws: [RenderPipeline; 3],
    clamp: Sampler,
    mirror: Sampler,
}
pub struct FlarePass {
    shared: Arc<Pipelines>,
    stage: FlareStage,
    parameters: UniformBuffer<FlareParameters>,
    binding: Option<([TextureViewId; 3], BindGroup)>,
}
impl FlarePass {
    pub fn new(device: &RenderDevice, format: TextureFormat, stage: FlareStage) -> Self {
        use binding_types::*;
        let layout = device.create_bind_group_layout(
            Some("flare inputs"),
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    uniform_buffer::<FlareParameters>(false),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        );
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("lens flare"),
            source: ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("lens flare"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let draws = ["threshold_fragment", "generate_fragment", "blur_fragment"].map(|entry| {
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: RawVertexState {
                    module: &shader,
                    entry_point: Some("flare_vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(RawFragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(ColorTargetState {
                        format,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        });
        let sampler = |address| {
            device.create_sampler(&SamplerDescriptor {
                address_mode_u: address,
                address_mode_v: address,
                address_mode_w: address,
                min_filter: FilterMode::Linear,
                mag_filter: FilterMode::Linear,
                ..Default::default()
            })
        };
        Self {
            shared: Arc::new(Pipelines {
                layout,
                draws,
                clamp: sampler(AddressMode::ClampToEdge),
                mirror: sampler(AddressMode::MirrorRepeat),
            }),
            stage,
            parameters: Default::default(),
            binding: None,
        }
    }
    fn sibling(&self, stage: FlareStage) -> Self {
        Self {
            shared: self.shared.clone(),
            stage,
            parameters: Default::default(),
            binding: None,
        }
    }
    /// Each submitted draw needs its own owner to retain its parameter upload.
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: FlareParameters,
        inputs: [&TextureView; 3],
        output: &TextureView,
        viewport: [u32; 2],
        scissor: [u32; 2],
    ) {
        self.parameters.set(parameters);
        self.parameters.write_buffer(device, queue);
        let ids = inputs.map(|v| v.id());
        if self.binding.as_ref().is_none_or(|b| b.0 != ids) {
            self.binding = Some((
                ids,
                device.create_bind_group(
                    Some("flare images"),
                    &self.shared.layout,
                    &BindGroupEntries::sequential((
                        self.parameters.binding().unwrap(),
                        inputs[0],
                        inputs[1],
                        inputs[2],
                        &self.shared.clamp,
                        &self.shared.mirror,
                    )),
                ),
            ));
        }
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("lens flare"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: output,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.shared.draws[self.stage as usize]);
        pass.set_bind_group(0, &self.binding.as_ref().unwrap().1, &[]);
        pass.set_viewport(0.0, 0.0, viewport[0] as f32, viewport[1] as f32, 0.0, 1.0);
        pass.set_scissor_rect(0, 0, scissor[0], scissor[1]);
        pass.draw(0..3, 0..1);
    }
}

/// Two retained packed targets, four independent parameter buffers, one shared
/// set of pipelines. No extra queue submission and no per-frame target growth.
pub struct FlareChain {
    temporary: Texture,
    temporary_view: TextureView,
    output: Texture,
    output_view: TextureView,
    passes: [FlarePass; 4],
    full_extent: [u32; 2],
    scaled_extent: [u32; 2],
    scale: [f32; 2],
}
impl FlareChain {
    /// `source_extent` belongs to hrBufferDiv2; `output_extent` is separately
    /// allocated by 327270 from depthAlphaDiv2. Neither is a luminance mip.
    pub fn new(
        device: &RenderDevice,
        source_extent: [u32; 2],
        output_extent: [u32; 2],
        scale: [f32; 2],
    ) -> Result<Self, String> {
        if !device.features().intersects(
            WgpuFeatures::RG11B10UFLOAT_RENDERABLE
                | WgpuFeatures::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        ) {
            return Err("lens flares require renderable R11G11B10 float targets".into());
        }
        let full_extent = source_extent.map(|v| v / 2);
        if full_extent
            .into_iter()
            .chain(output_extent)
            .any(|v| v == 0 || v > i32::MAX as u32)
        {
            return Err("invalid flare target extent".into());
        }
        let rect = crate::visibility::tome::scaled_render_viewport(
            [0, 0, full_extent[0] as i32 - 1, full_extent[1] as i32 - 1],
            scale,
        )?;
        let scaled_extent =
            crate::visibility::tome::render_viewport_dimensions(rect)?.map(|v| v as u32);
        if (0..2).any(|i| scaled_extent[i] > full_extent[i] || scaled_extent[i] > output_extent[i])
        {
            return Err("flare viewport exceeds its target".into());
        }
        let texture = |extent: [u32; 2]| {
            device.create_texture(&TextureDescriptor {
                label: Some("lens flare packed target"),
                size: Extent3d {
                    width: extent[0],
                    height: extent[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rg11b10Ufloat,
                usage: TextureUsages::TEXTURE_BINDING
                    | TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let temporary = texture(full_extent);
        let temporary_view = temporary.create_view(&Default::default());
        let output = texture(output_extent);
        let output_view = output.create_view(&Default::default());
        let first = FlarePass::new(device, TextureFormat::Rg11b10Ufloat, FlareStage::Threshold);
        let passes = [
            first.sibling(FlareStage::Threshold),
            first.sibling(FlareStage::Generate),
            first.sibling(FlareStage::Blur),
            first.sibling(FlareStage::Blur),
        ];
        Ok(Self {
            temporary,
            temporary_view,
            output,
            output_view,
            passes,
            full_extent,
            scaled_extent,
            scale,
        })
    }
    pub fn output(&self) -> &TextureView {
        &self.output_view
    }
    pub fn output_texture(&self) -> &Texture {
        &self.output
    }
    pub fn temporary_texture(&self) -> &Texture {
        &self.temporary
    }
    /// Returns false for the original disabling bit. The caller binds its black
    /// image and clears the flare-enabled flag in that case.
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        source: &TextureView,
        profiles: [&TextureView; 2],
        p: FlareParameters,
        flags: u32,
    ) -> bool {
        self.record_observed(
            device,
            queue,
            encoder,
            source,
            profiles,
            p,
            flags,
            |_, _, _| {},
        )
    }

    fn record_observed(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        source: &TextureView,
        profiles: [&TextureView; 2],
        mut p: FlareParameters,
        flags: u32,
        mut after_draw: impl FnMut(&mut CommandEncoder, usize, &Texture),
    ) -> bool {
        if flags & 0x200000 != 0 {
            return false;
        }
        p.controls.y = (p.controls.y as i32).clamp(1, 32) as f32;
        p.controls.z = (p.controls.z as i32).clamp(1, 24) as f32;
        for i in 0..4 {
            let viewport = if i == 0 {
                self.full_extent
            } else {
                self.scaled_extent
            };
            p.view = Vec4::new(
                1.0 / viewport[0] as f32,
                1.0 / viewport[1] as f32,
                self.scale[0],
                self.scale[1],
            );
            p.direction = if i == 2 {
                Vec4::X
            } else if i == 3 {
                Vec4::Y
            } else {
                Vec4::ZERO
            };
            let (input, output) = match i {
                0 => (source, &self.temporary_view),
                1 | 3 => (&self.temporary_view, &self.output_view),
                _ => (&self.output_view, &self.temporary_view),
            };
            self.passes[i].record(
                device,
                queue,
                encoder,
                p,
                [input, profiles[0], profiles[1]],
                output,
                viewport,
                self.scaled_extent,
            );
            after_draw(
                encoder,
                i,
                if i == 0 || i == 2 {
                    &self.temporary
                } else {
                    &self.output
                },
            );
        }
        true
    }
}
#[cfg(test)]
#[path = "lens_flare/tests.rs"]
mod tests;
