//! Colour downscale raster pass. Output formats and viewport/scissor come from
//! the owning render task, rather than being inferred from a luminance mip.
use crate::rendering::colour_downscale::{DownscaleInputs, DownscaleMode};
use bevy::{
    math::{UVec4, Vec4},
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_colour_output.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_colour_downscale.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_colour_downscale_source.wgsl")
);
const HOST_SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_colour_output.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_colour_downscale.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_hdr_downscale_source.wgsl")
);
#[derive(Clone, Copy, Default, ShaderType)]
pub struct DownscaleParameters {
    pub texel: Vec4,
    pub colour: Vec4,
    /// Authored adaptation maximum, engine luminance, view white level, host radiance units.
    /// Host-source draws read engine luminance from the actual GPU buffer instead.
    pub limits: Vec4,
    /// Shader TYPE and compressed HDR switch.
    pub options: UVec4,
}
impl DownscaleParameters {
    pub fn new(
        inputs: DownscaleInputs,
        mode: DownscaleMode,
        adapt_max: f32,
        engine_luminance: f32,
        white_level: f32,
        compressed: bool,
    ) -> Self {
        Self {
            texel: Vec4::from_array(inputs.texel_size),
            colour: Vec4::from_array(inputs.colour),
            limits: Vec4::new(adapt_max, engine_luminance, white_level, 0.0),
            options: UVec4::new(mode as u32, u32::from(compressed), 0, 0),
        }
    }
}
pub struct ColourDownscalePass {
    shared: std::sync::Arc<Pipeline>,
    parameters: UniformBuffer<DownscaleParameters>,
    binding: Option<((TextureViewId, Option<BufferId>), BindGroup)>,
    host_source: bool,
    geometry: bool,
}
struct Pipeline {
    layout: BindGroupLayout,
    draw: RenderPipeline,
    linear: Sampler,
    point: Sampler,
}
impl ColourDownscalePass {
    pub fn new(device: &RenderDevice, format: TextureFormat) -> Self {
        Self::create(device, format, false, false)
    }
    /// Adapt the host primary's pre-exposed physical RGB inside the downsample
    /// draw. This does not reproduce the original main-target packed quantization.
    pub fn new_with_host_source(device: &RenderDevice, format: TextureFormat) -> Self {
        Self::create(device, format, true, false)
    }
    pub fn new_with_source_geometry(
        device: &RenderDevice,
        format: TextureFormat,
        host_source: bool,
    ) -> Self {
        Self::create(device, format, host_source, true)
    }
    fn create(
        device: &RenderDevice,
        format: TextureFormat,
        host_source: bool,
        geometry: bool,
    ) -> Self {
        use binding_types::*;
        let mut entries = BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                uniform_buffer::<DownscaleParameters>(false),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                sampler(SamplerBindingType::Filtering),
            ),
        )
        .to_vec();
        if geometry {
            entries[0].visibility |= ShaderStages::VERTEX;
        }
        if host_source {
            entries.push(BindGroupLayoutEntry {
                binding: 4,
                visibility: ShaderStages::FRAGMENT,
                count: None,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(32),
                },
            });
        }
        let layout = device.create_bind_group_layout(Some("colour downscale inputs"), &entries);
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("colour downscale"),
            source: ShaderSource::Wgsl(if host_source {
                HOST_SHADER_SOURCE.into()
            } else {
                SHADER_SOURCE.into()
            }),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("colour downscale"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let draw = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("colour downscale"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &shader,
                entry_point: Some(if geometry {
                    "downscale_mesh_vertex"
                } else {
                    "downscale_vertex"
                }),
                compilation_options: Default::default(),
                buffers: if geometry {
                    &[RawVertexBufferLayout {
                        array_stride: 20,
                        step_mode: VertexStepMode::Vertex,
                        attributes: &[
                            VertexAttribute {
                                format: VertexFormat::Float32x3,
                                offset: 0,
                                shader_location: 0,
                            },
                            VertexAttribute {
                                format: VertexFormat::Float32x2,
                                offset: 12,
                                shader_location: 1,
                            },
                        ],
                    }]
                } else {
                    &[]
                },
            },
            fragment: Some(RawFragmentState {
                module: &shader,
                entry_point: Some(if geometry {
                    "downscale_mesh_fragment"
                } else {
                    "downscale_fragment"
                }),
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
        });
        let sampler = |filter| {
            device.create_sampler(&SamplerDescriptor {
                address_mode_u: AddressMode::ClampToEdge,
                address_mode_v: AddressMode::ClampToEdge,
                address_mode_w: AddressMode::ClampToEdge,
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        Self {
            shared: std::sync::Arc::new(Pipeline {
                layout,
                draw,
                linear: sampler(FilterMode::Linear),
                point: sampler(FilterMode::Nearest),
            }),
            parameters: Default::default(),
            binding: None,
            host_source,
            geometry,
        }
    }
    /// Separate uploads for different draws queued into the same command buffer.
    pub fn sibling(&self) -> Self {
        Self {
            shared: self.shared.clone(),
            parameters: Default::default(),
            binding: None,
            host_source: self.host_source,
            geometry: self.geometry,
        }
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: DownscaleParameters,
        source: &TextureView,
        destination: &TextureView,
        viewport: [u32; 4],
        scissor: [u32; 4],
    ) {
        assert!(
            !self.host_source,
            "host downscale requires the GPU adaptation buffer"
        );
        self.record_inner(
            device,
            queue,
            encoder,
            parameters,
            source,
            destination,
            viewport,
            scissor,
            None,
            None,
        );
    }
    pub fn record_host(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: DownscaleParameters,
        source: &TextureView,
        destination: &TextureView,
        viewport: [u32; 4],
        scissor: [u32; 4],
        adaptation: &Buffer,
    ) {
        assert!(
            self.host_source,
            "downscale pipeline has no host source adapter"
        );
        self.record_inner(
            device,
            queue,
            encoder,
            parameters,
            source,
            destination,
            viewport,
            scissor,
            Some(adaptation),
            None,
        );
    }
    pub fn record_with_geometry(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: DownscaleParameters,
        source: &TextureView,
        destination: &TextureView,
        viewport: [u32; 4],
        scissor: [u32; 4],
        vertices: &Buffer,
        range: std::ops::Range<u32>,
        adaptation: Option<&Buffer>,
    ) {
        assert_eq!(
            self.host_source,
            adaptation.is_some(),
            "downscale geometry source adapter differs"
        );
        self.record_inner(
            device,
            queue,
            encoder,
            parameters,
            source,
            destination,
            viewport,
            scissor,
            adaptation,
            Some((vertices, range)),
        );
    }
    fn record_inner(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: DownscaleParameters,
        source: &TextureView,
        destination: &TextureView,
        viewport: [u32; 4],
        scissor: [u32; 4],
        adaptation: Option<&Buffer>,
        geometry: Option<(&Buffer, std::ops::Range<u32>)>,
    ) {
        assert_eq!(
            self.geometry,
            geometry.is_some(),
            "downscale geometry differs from prepared pipeline"
        );
        self.parameters.set(parameters);
        self.parameters.write_buffer(device, queue);
        let key = (source.id(), adaptation.map(Buffer::id));
        if self.binding.as_ref().is_none_or(|v| v.0 != key) {
            let mut entries = vec![
                BindGroupEntry {
                    binding: 0,
                    resource: self.parameters.binding().unwrap(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(source),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(&self.shared.linear),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::Sampler(&self.shared.point),
                },
            ];
            if let Some(adaptation) = adaptation {
                entries.push(BindGroupEntry {
                    binding: 4,
                    resource: adaptation.as_entire_binding(),
                });
            }
            self.binding = Some((
                key,
                device.create_bind_group(
                    Some("colour downscale source"),
                    &self.shared.layout,
                    &entries,
                ),
            ));
        }
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("colour downscale"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.shared.draw);
        pass.set_bind_group(0, &self.binding.as_ref().unwrap().1, &[]);
        pass.set_viewport(
            viewport[0] as f32,
            viewport[1] as f32,
            viewport[2] as f32,
            viewport[3] as f32,
            0.0,
            1.0,
        );
        pass.set_scissor_rect(scissor[0], scissor[1], scissor[2], scissor[3]);
        if let Some((vertices, range)) = geometry {
            pass.set_vertex_buffer(0, *vertices.slice(..));
            pass.draw(range, 0..1);
        } else {
            pass.draw(0..3, 0..1);
        }
    }
}
/// The pre-DOF `hrBufferDiv2` colour target. This is independent of the
/// exposure/bloom luminance pyramid and inherits the main HDR storage format.
/// Only the complete, unscaled view is supported by this owner at present.
pub struct HdrDownscale {
    extent: [u32; 2],
    compressed: bool,
    texture: Texture,
    view: TextureView,
    draw: ColourDownscalePass,
}
impl HdrDownscale {
    pub fn new(device: &RenderDevice, extent: [u32; 2], compressed: bool) -> Result<Self, String> {
        if extent.contains(&0) {
            return Err("HDR downscale requires a nonempty target".into());
        }
        let format = if compressed {
            TextureFormat::Rg11b10Ufloat
        } else {
            TextureFormat::Rgba16Float
        };
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("pre-DOF HDR colour"),
            size: Extent3d {
                width: extent[0],
                height: extent[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        Ok(Self {
            extent,
            compressed,
            texture,
            view,
            draw: ColourDownscalePass::new_with_host_source(device, format),
        })
    }
    pub fn matches(&self, extent: [u32; 2], compressed: bool) -> bool {
        self.extent == extent && self.compressed == compressed
    }
    pub fn output(&self) -> &TextureView {
        &self.view
    }
    /// The caller must supply the full colour source BEFORE DOF filtering,
    /// and the current adaptation buffer. Source texel offsets use that image's
    /// dimensions; target dimensions do not replace them.
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        source: &TextureView,
        source_extent: [u32; 2],
        camera_exposure: f32,
        adapt_max: f32,
        white_level: f32,
        adaptation: &Buffer,
    ) -> Result<(), String> {
        let host_units = 139.26 * camera_exposure;
        if !host_units.is_finite() || host_units <= 0.0 {
            return Err("HDR downscale requires positive finite host radiance units".into());
        }
        if self.compressed && (!white_level.is_finite() || white_level <= 0.0) {
            return Err("compressed HDR downscale requires a positive white level".into());
        }
        // This mode does not read the clear-colour fields or CPU luminance.
        let mut parameters = DownscaleParameters::new(
            DownscaleInputs::new(source_extent, [0.0; 3])?,
            DownscaleMode::AverageRgbClamp,
            adapt_max,
            0.0,
            white_level,
            self.compressed,
        );
        parameters.limits.w = host_units;
        let viewport = [0, 0, self.extent[0], self.extent[1]];
        self.draw.record_host(
            device, queue, encoder, parameters, source, &self.view, viewport, viewport, adaptation,
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "colour_downscale/tests.rs"]
mod tests;
