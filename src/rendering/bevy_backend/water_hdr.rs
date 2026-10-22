//! Native-format pre-water source and writable colour, with explicit host-unit
//! conversion. This owns storage rounding; it is not a colour downsample.
use bevy::{
    math::{UVec4, Vec4},
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

#[derive(Clone, Copy, ShaderType)]
struct Parameters {
    limits: Vec4,
    options: UVec4,
}

pub struct WaterHdr {
    extent: [u32; 2],
    compressed: bool,
    host_format: TextureFormat,
    source: Texture,
    colour: Texture,
    source_view: TextureView,
    colour_view: TextureView,
    parameters: UniformBuffer<Parameters>,
    layout: BindGroupLayout,
    encode: RenderPipeline,
    decode: RenderPipeline,
    encode_bindings: [Option<((TextureViewId, BufferId), BindGroup)>; 2],
    next_binding: usize,
    decode_binding: Option<(BufferId, BindGroup)>,
}

/// Keeps the source, working colour, constants and exposure binding together
/// until the owning water task finishes. No HDR composition is implicit.
#[must_use = "finish the water colour task into its host HDR destination"]
pub struct WaterHdrFrame<'a> {
    owner: &'a WaterHdr,
    host: &'a TextureView,
}
impl WaterHdrFrame<'_> {
    pub fn source(&self) -> &TextureView {
        &self.owner.source_view
    }
    pub fn colour(&self) -> &TextureView {
        &self.owner.colour_view
    }
    pub fn format(&self) -> TextureFormat {
        self.owner.colour.format()
    }
    pub fn finish(self, encoder: &mut CommandEncoder) {
        let binding = &self.owner.decode_binding.as_ref().unwrap().1;
        self.owner.record(
            encoder,
            &self.owner.decode,
            binding,
            &[Some(RenderPassColorAttachment {
                view: self.host,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
        );
    }
}

impl WaterHdr {
    pub fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        extent: [u32; 2],
        compressed: bool,
        host_format: TextureFormat,
    ) -> Result<Self, String> {
        if extent.contains(&0) {
            return Err("water HDR requires a nonempty full view".into());
        }
        let format = if compressed {
            TextureFormat::Rg11b10Ufloat
        } else {
            TextureFormat::Rgba16Float
        };
        let texture = |label| {
            device.create_texture(&TextureDescriptor {
                label: Some(label),
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
            })
        };
        let source = texture("pre-water HDR source");
        let colour = texture("water HDR colour");
        let source_view = source.create_view(&Default::default());
        let colour_view = colour.create_view(&Default::default());
        let mut parameters = UniformBuffer::from(Parameters {
            limits: Vec4::ZERO,
            options: UVec4::ZERO,
        });
        parameters.write_buffer(device, queue);
        let layout = device.create_bind_group_layout(
            Some("water HDR conversion"),
            &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    count: None,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: BufferSize::new(32),
                    },
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    count: None,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: false },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    count: None,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: BufferSize::new(32),
                    },
                },
            ],
        );
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("water native HDR storage bridge"),
            source: ShaderSource::Wgsl(
                concat!(
                    include_str!("../../../assets/shaders/dh2_colour_output.wgsl"),
                    "\n",
                    include_str!("../../../assets/shaders/dh2_hdr.wgsl"),
                    "\n",
                    include_str!("../../../assets/shaders/dh2_water_hdr.wgsl"),
                )
                .into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("water HDR conversion"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |entry, targets: &[Option<ColorTargetState>]| {
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: RawVertexState {
                    module: &shader,
                    entry_point: Some("water_hdr_vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(RawFragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets,
                }),
                primitive: PrimitiveState::default(),
                depth_stencil: None,
                multisample: MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let target = Some(ColorTargetState {
            format,
            blend: None,
            write_mask: ColorWrites::ALL,
        });
        let encode = make("water_hdr_encode", &[target.clone(), target]);
        let decode = make(
            "water_hdr_decode",
            &[Some(ColorTargetState {
                format: host_format,
                blend: None,
                write_mask: ColorWrites::RED | ColorWrites::GREEN | ColorWrites::BLUE,
            })],
        );
        Ok(Self {
            extent,
            compressed,
            host_format,
            source,
            colour,
            source_view,
            colour_view,
            parameters,
            layout,
            encode,
            decode,
            encode_bindings: [None, None],
            next_binding: 0,
            decode_binding: None,
        })
    }

    pub fn matches(&self, extent: [u32; 2], compressed: bool, host_format: TextureFormat) -> bool {
        self.extent == extent && self.compressed == compressed && self.host_format == host_format
    }

    pub fn prepare<'a>(
        &'a mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        host: &'a TextureView,
        host_extent: [u32; 2],
        camera_exposure: f32,
        white_level: f32,
        adaptation: &Buffer,
    ) -> Result<WaterHdrFrame<'a>, String> {
        let units = 139.26 * camera_exposure;
        if host_extent != self.extent {
            return Err("water HDR source extent differs from its full view".into());
        }
        if !units.is_finite() || units <= 0.0 {
            return Err("water HDR requires positive finite host radiance units".into());
        }
        if self.compressed && (!white_level.is_finite() || white_level <= 0.0) {
            return Err("compressed water HDR requires a positive finite white level".into());
        }
        if host.id() == self.source_view.id() || host.id() == self.colour_view.id() {
            return Err("water HDR source aliases its native target".into());
        }
        self.parameters.set(Parameters {
            limits: Vec4::new(white_level, units, 0.0, 0.0),
            options: UVec4::new(u32::from(self.compressed), 0, 0, 0),
        });
        self.parameters.write_buffer(device, queue);
        let key = (host.id(), adaptation.id());
        let index = if let Some(index) = self
            .encode_bindings
            .iter()
            .position(|v| v.as_ref().is_some_and(|v| v.0 == key))
        {
            index
        } else {
            let index = self.next_binding;
            self.encode_bindings[index] = Some((key, self.binding(device, host, adaptation)));
            self.next_binding = (index + 1) % self.encode_bindings.len();
            index
        };
        if self
            .decode_binding
            .as_ref()
            .is_none_or(|v| v.0 != adaptation.id())
        {
            self.decode_binding = Some((
                adaptation.id(),
                self.binding(device, &self.colour_view, adaptation),
            ));
        }
        let attachment = |view| {
            Some(RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Default::default()),
                    store: StoreOp::Store,
                },
            })
        };
        self.record(
            encoder,
            &self.encode,
            &self.encode_bindings[index].as_ref().unwrap().1,
            &[attachment(&self.source_view), attachment(&self.colour_view)],
        );
        Ok(WaterHdrFrame { owner: self, host })
    }

    fn binding(
        &self,
        device: &RenderDevice,
        source: &TextureView,
        adaptation: &Buffer,
    ) -> BindGroup {
        device.create_bind_group(
            Some("water HDR source and adaptation"),
            &self.layout,
            &[
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
                    resource: adaptation.as_entire_binding(),
                },
            ],
        )
    }

    fn record(
        &self,
        encoder: &mut CommandEncoder,
        pipeline: &RenderPipeline,
        binding: &BindGroup,
        colours: &[Option<RenderPassColorAttachment<'_>>],
    ) {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("water HDR conversion"),
            color_attachments: colours,
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, binding, &[]);
        pass.set_viewport(
            0.0,
            0.0,
            self.extent[0] as f32,
            self.extent[1] as f32,
            0.0,
            1.0,
        );
        pass.set_scissor_rect(0, 0, self.extent[0], self.extent[1]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
#[path = "water_hdr/tests.rs"]
mod tests;
