//! The single-group focus update. The view owner supplies its real linear
//! water-depth source and distinct previous/current focus images.
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};
#[cfg(test)]
#[path = "autofocus_tests.rs"]
mod tests;
pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_point_sampling.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_autofocus.wgsl")
);
#[derive(Clone, Copy, Debug)]
pub struct AutofocusParameters {
    pub centre: [f32; 2],
    pub resolution_scale: [f32; 2],
    /// Original PostFX/DOF/targetSize.xy, including view scaling.
    pub texel: [f32; 2],
    pub radius: f32,
    pub far_clip: f32,
    /// Focus speed, fuzzy speed, sharpness minimum and maximum.
    pub speeds: [f32; 4],
    pub local_delta: f32,
}
impl AutofocusParameters {
    pub fn uniform_bytes(self) -> [u8; 64] {
        let mut bytes = [0; 64];
        for (i, value) in self
            .centre
            .into_iter()
            .chain(self.resolution_scale)
            .chain(self.texel)
            .chain([self.radius, self.far_clip])
            .chain(self.speeds)
            .chain([0., self.local_delta, 0., 0.])
            .enumerate()
        {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}
pub struct AutofocusPass {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
    parameters: Buffer,
    linear: Sampler,
    binding: Option<([TextureViewId; 3], BindGroup)>,
}
impl AutofocusPass {
    pub fn new(device: &RenderDevice) -> Self {
        let entries = [
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(64),
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: false },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 3,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 4,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::StorageTexture {
                    access: StorageTextureAccess::WriteOnly,
                    format: TextureFormat::Rg32Float,
                    view_dimension: TextureViewDimension::D2,
                },
                count: None,
            },
        ];
        let layout = device.create_bind_group_layout(Some("autofocus inputs"), &entries);
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("autofocus"),
            source: ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("autofocus"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
            label: Some("autofocus"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("focus_update"),
            compilation_options: Default::default(),
            cache: None,
        });
        let parameters = device.create_buffer(&BufferDescriptor {
            label: Some("autofocus parameters"),
            size: 64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let linear = device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            ..Default::default()
        });
        Self {
            layout,
            pipeline,
            parameters,
            linear,
            binding: None,
        }
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: AutofocusParameters,
        water_depth: &TextureView,
        previous: &TextureView,
        current: &TextureView,
    ) -> Result<(), String> {
        if current.id() == previous.id() || current.id() == water_depth.id() {
            return Err("autofocus input aliases its output".into());
        }
        queue.write_buffer(&self.parameters, 0, &parameters.uniform_bytes());
        let key = [water_depth.id(), previous.id(), current.id()];
        if self.binding.as_ref().is_none_or(|(old, _)| *old != key) {
            self.binding = Some((
                key,
                device.create_bind_group(
                    Some("autofocus view"),
                    &self.layout,
                    &[
                        BindGroupEntry {
                            binding: 0,
                            resource: self.parameters.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 1,
                            resource: BindingResource::TextureView(water_depth),
                        },
                        BindGroupEntry {
                            binding: 2,
                            resource: BindingResource::TextureView(previous),
                        },
                        BindGroupEntry {
                            binding: 3,
                            resource: BindingResource::Sampler(&self.linear),
                        },
                        BindGroupEntry {
                            binding: 4,
                            resource: BindingResource::TextureView(current),
                        },
                    ],
                ),
            ));
        }
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("autofocus"),
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding.as_ref().unwrap().1, &[]);
        pass.dispatch_workgroups(1, 1, 1);
        Ok(())
    }
}
