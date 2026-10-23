//! Full camera-combine draw. The caller must provide the actual near/far and
//! autofocus producers; this pass never fabricates missing blur images.
use crate::rendering::camera_effects::CombineParameters;
#[cfg(test)]
#[path = "camera_effects_tests.rs"]
mod tests;
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};
pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_point_sampling.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_camera_effects.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_camera_combine.wgsl")
);
#[derive(Clone, Debug)]
pub struct CameraCombineParameters {
    pub optics: CombineParameters,
    pub scattering_colour: [f32; 3],
    pub white_level: f32,
    pub resolution_scale: [f32; 2],
    /// Inverse projection [2].w, [3].w, [3].z, then view time.
    pub inverse_z_time: [f32; 4],
    /// None consumes/produces game HDR units; Some adapts the host primary.
    pub host_radiance_units: Option<f32>,
    pub compressed: bool,
    pub exposure_entry: u32,
}
impl CameraCombineParameters {
    pub fn uniform_bytes(&self) -> [u8; 144] {
        let p = &self.optics;
        let floats = p
            .vignette
            .into_iter()
            .chain(self.scattering_colour)
            .chain([self.white_level])
            .chain(p.far)
            .chain(p.distortion)
            .chain([0.0])
            .chain(p.centre)
            .chain(self.resolution_scale)
            .chain(self.inverse_z_time)
            .chain(p.noise)
            .chain([self.host_radiance_units.unwrap_or(1.0), 0.0, 0.0, 0.0]);
        let options = p
            .options
            .iter()
            .enumerate()
            .fold(0, |bits, (i, v)| bits | (u32::from(*v) << i));
        let mut bytes = [0; 144];
        for (i, word) in floats
            .map(f32::to_bits)
            .chain([
                options,
                u32::from(self.compressed),
                u32::from(self.host_radiance_units.is_some()),
                self.exposure_entry,
            ])
            .enumerate()
        {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        bytes
    }
}
/// Full colour, hardware depth converted to float, far colour, near alpha,
/// near colour, pre-DOF half colour, and autofocus distance, in that order.
pub struct CameraCombineInputs<'a> {
    pub images: [&'a TextureView; 7],
    pub adaptation: &'a Buffer,
}
pub struct CameraCombinePass {
    pipeline: RenderPipeline,
    layout: BindGroupLayout,
    linear: Sampler,
    point: Sampler,
    uniform: Buffer,
    binding: Option<(([TextureViewId; 7], BufferId), BindGroup)>,
}
impl CameraCombinePass {
    pub fn new(device: &RenderDevice, format: TextureFormat) -> Self {
        use binding_types::*;
        let mut entries = vec![BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::FRAGMENT | ShaderStages::VERTEX,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: BufferSize::new(144),
            },
            count: None,
        }];
        for i in 1..=7 {
            entries.push(BindGroupLayoutEntry {
                binding: i,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float {
                        filterable: i != 2 && i != 7,
                    },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        entries.extend(
            BindGroupLayoutEntries::with_indices(
                ShaderStages::FRAGMENT,
                (
                    (8, sampler(SamplerBindingType::Filtering)),
                    (9, sampler(SamplerBindingType::NonFiltering)),
                    (10, storage_buffer_read_only::<[bevy::math::Vec4; 2]>(false)),
                ),
            )
            .to_vec(),
        );
        let layout = device.create_bind_group_layout(Some("camera combine inputs"), &entries);
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("camera combine"),
            source: ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("camera combine"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("camera combine"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &shader,
                entry_point: Some("camera_vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &shader,
                entry_point: Some("camera_fragment"),
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
                mag_filter: filter,
                min_filter: filter,
                address_mode_u: AddressMode::ClampToEdge,
                address_mode_v: AddressMode::ClampToEdge,
                ..Default::default()
            })
        };
        Self {
            pipeline,
            layout,
            linear: sampler(FilterMode::Linear),
            point: sampler(FilterMode::Nearest),
            uniform: device.create_buffer(&BufferDescriptor {
                label: Some("camera combine parameters"),
                size: 144,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            binding: None,
        }
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: &CameraCombineParameters,
        inputs: CameraCombineInputs<'_>,
        destination: &TextureView,
        extent: [u32; 2],
    ) -> Result<(), String> {
        if extent.contains(&0) {
            return Err("camera combine requires a nonempty view".into());
        }
        if inputs.images.iter().any(|v| v.id() == destination.id()) {
            return Err("camera combine source aliases destination".into());
        }
        if parameters.exposure_entry > 1 {
            return Err("camera combine exposure entry must name the double buffer".into());
        }
        if inputs.adaptation.size() < u64::from(parameters.exposure_entry + 1) * 32 {
            return Err("camera combine exposure entry exceeds its actual buffer".into());
        }
        if parameters
            .host_radiance_units
            .is_some_and(|v| !v.is_finite() || v <= 0.0)
        {
            return Err("camera combine requires positive host radiance units".into());
        }
        queue.write_buffer(&self.uniform, 0, &parameters.uniform_bytes());
        let key = (inputs.images.map(TextureView::id), inputs.adaptation.id());
        if self.binding.as_ref().is_none_or(|v| v.0 != key) {
            let mut entries = vec![BindGroupEntry {
                binding: 0,
                resource: self.uniform.as_entire_binding(),
            }];
            for (i, view) in inputs.images.into_iter().enumerate() {
                entries.push(BindGroupEntry {
                    binding: i as u32 + 1,
                    resource: BindingResource::TextureView(view),
                });
            }
            entries.extend([
                BindGroupEntry {
                    binding: 8,
                    resource: BindingResource::Sampler(&self.linear),
                },
                BindGroupEntry {
                    binding: 9,
                    resource: BindingResource::Sampler(&self.point),
                },
                BindGroupEntry {
                    binding: 10,
                    resource: inputs.adaptation.as_entire_binding(),
                },
            ]);
            self.binding = Some((
                key,
                device.create_bind_group(Some("camera combine sources"), &self.layout, &entries),
            ));
        }
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("camera combine"),
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding.as_ref().unwrap().1, &[]);
        pass.set_viewport(0.0, 0.0, extent[0] as f32, extent[1] as f32, 0.0, 1.0);
        pass.set_scissor_rect(0, 0, extent[0], extent[1]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}
