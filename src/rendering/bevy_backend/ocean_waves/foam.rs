//! 1402fb7d0 foam dispatch and 1402fa110's one-time zero initialization.
use super::*;

#[derive(Clone, Copy, Debug)]
pub enum FoamMode {
    Height,
    Jacobian,
}
impl FoamMode {
    /// 2fb7d0: zero selects height; every other integer selects Jacobian.
    pub fn from_type(foam_type: i32) -> Self {
        if foam_type == 0 {
            Self::Height
        } else {
            Self::Jacobian
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FoamSettings {
    pub threshold: f32,
    pub spawn_rate: f32,
    pub decay_rate: f32,
    pub delta_seconds: f32,
}
impl FoamSettings {
    pub fn from_parameters(
        parameters: crate::rendering::ocean::WaveFoamParameters,
        delta_seconds: f32,
    ) -> Self {
        Self {
            threshold: parameters.threshold,
            spawn_rate: parameters.spawn_rate,
            decay_rate: parameters.decay_rate,
            delta_seconds,
        }
    }
}

pub(super) struct WaveFoam {
    texture: Texture,
    view: TextureView,
    uniform: Buffer,
    pipelines: [ComputePipeline; 2],
    height_bindings: [BindGroup; 2],
    jacobian_binding: BindGroup,
}
impl WaveFoam {
    pub(super) fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        displacement: [&TextureView; 2],
        gradients: &TextureView,
    ) -> Self {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("ocean persistent foam"),
            size: Extent3d {
                width: WAVE_RESOLUTION,
                height: WAVE_RESOLUTION,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::R32Float,
            usage: TextureUsages::STORAGE_BINDING
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&default());
        // Native ClearUnorderedAccessViewFloat is a single zero clear during
        // creation. Use a render clear for the same R32F contents on this backend.
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("initialize ocean foam"),
        });
        {
            let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("clear ocean foam once"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(default()),
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
        }
        queue.submit([encoder.finish()]);
        let uniform = device.create_buffer(&BufferDescriptor {
            label: Some("ocean foam parameters"),
            size: 16,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("ocean foam"),
            source: ShaderSource::Wgsl(
                include_str!("../../../../assets/shaders/dh2_ocean_foam.wgsl").into(),
            ),
        });
        let pipelines = ["foam_from_height", "foam_from_jacobian"].map(|entry| {
            device.create_compute_pipeline(&RawComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: default(),
                cache: None,
            })
        });
        let binding = |pipeline: &ComputePipeline, source: &TextureView| {
            let layout = pipeline.get_bind_group_layout(0).into();
            device.create_bind_group(
                "ocean foam inputs",
                &layout,
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(source),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::TextureView(&view),
                    },
                ],
            )
        };
        let height_bindings = displacement.map(|source| binding(&pipelines[0], source));
        let jacobian_binding = binding(&pipelines[1], gradients);
        Self {
            texture,
            view,
            uniform,
            pipelines,
            height_bindings,
            jacobian_binding,
        }
    }

    pub(super) fn prepare(&self, queue: &RenderQueue, settings: FoamSettings) {
        queue.write_buffer(
            &self.uniform,
            0,
            &words(
                [
                    settings.threshold,
                    settings.spawn_rate,
                    settings.decay_rate,
                    settings.delta_seconds,
                ]
                .map(f32::to_bits),
            ),
        );
    }
    pub(super) fn record(&self, encoder: &mut CommandEncoder, index: usize, mode: FoamMode) {
        let (pipeline, binding) = match mode {
            FoamMode::Height => (&self.pipelines[0], &self.height_bindings[index]),
            FoamMode::Jacobian => (&self.pipelines[1], &self.jacobian_binding),
        };
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("ocean foam"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, binding, &[]);
        pass.dispatch_workgroups(
            WAVE_RESOLUTION.div_ceil(16),
            WAVE_RESOLUTION.div_ceil(16),
            1,
        );
    }
    pub(super) fn texture(&self) -> &Texture {
        &self.texture
    }
    pub(super) fn view(&self) -> &TextureView {
        &self.view
    }
    pub(super) fn uniform_bytes(&self) -> u64 {
        self.uniform.size()
    }
}
