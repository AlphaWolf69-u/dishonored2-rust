//! 1402fb5e0's displacement and gradient passes; collision/foam are separate.
use super::*;

pub(super) struct WaveSurfaces {
    displacement: [Texture; 2],
    displacement_views: [TextureView; 2],
    gradients: Texture,
    gradient_view: TextureView,
    displacement_uniform: Buffer,
    gradient_uniform: Buffer,
    displacement_pipeline: RenderPipeline,
    gradient_pipeline: RenderPipeline,
    displacement_binding: BindGroup,
    gradient_bindings: [BindGroup; 2],
}

impl WaveSurfaces {
    pub(super) fn new(device: &RenderDevice, transformed: &Buffer) -> Self {
        let texture = |label| {
            device.create_texture(&TextureDescriptor {
                label: Some(label),
                size: Extent3d {
                    width: WAVE_RESOLUTION,
                    height: WAVE_RESOLUTION,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                // Source packed 0x5cb maps to DXGI10/RGBA16_FLOAT.
                format: TextureFormat::Rgba16Float,
                usage: TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::TEXTURE_BINDING
                    | TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let displacement = [
            texture("ocean displacement 0"),
            texture("ocean displacement 1"),
        ];
        let displacement_views = displacement
            .each_ref()
            .map(|image| image.create_view(&default()));
        let gradients = texture("ocean gradients and fold");
        let gradient_view = gradients.create_view(&default());
        let uniform = |label, size| {
            device.create_buffer(&BufferDescriptor {
                label: Some(label),
                size,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let displacement_uniform = uniform("ocean displacement parameters", 32);
        let gradient_uniform = uniform("ocean gradient parameters", 16);
        let pipeline = |source: &'static str, entry| {
            let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
                label: Some(entry),
                source: ShaderSource::Wgsl(source.into()),
            });
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some(entry),
                layout: None,
                vertex: RawVertexState {
                    module: &module,
                    entry_point: Some("fullscreen_vertex"),
                    compilation_options: default(),
                    buffers: &[],
                },
                fragment: Some(RawFragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: default(),
                    targets: &[Some(ColorTargetState {
                        format: TextureFormat::Rgba16Float,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                primitive: PrimitiveState {
                    cull_mode: None,
                    ..default()
                },
                depth_stencil: None,
                multisample: default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let displacement_pipeline = pipeline(
            include_str!("../../../../assets/shaders/dh2_ocean_displacement.wgsl"),
            "displacement_fragment",
        );
        let gradient_pipeline = pipeline(
            include_str!("../../../../assets/shaders/dh2_ocean_gradients.wgsl"),
            "gradients_fragment",
        );
        let displacement_layout = displacement_pipeline.get_bind_group_layout(0).into();
        let displacement_binding = device.create_bind_group(
            "ocean displacement input",
            &displacement_layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: displacement_uniform.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: transformed.as_entire_binding(),
                },
            ],
        );
        let gradient_layout = gradient_pipeline.get_bind_group_layout(0).into();
        let gradient_bindings = displacement_views.each_ref().map(|view| {
            device.create_bind_group(
                "ocean gradient input",
                &gradient_layout,
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: gradient_uniform.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(view),
                    },
                ],
            )
        });
        Self {
            displacement,
            displacement_views,
            gradients,
            gradient_view,
            displacement_uniform,
            gradient_uniform,
            displacement_pipeline,
            gradient_pipeline,
            displacement_binding,
            gradient_bindings,
        }
    }

    pub(super) fn prepare(&self, queue: &RenderQueue, choppiness: f32) {
        let square = WAVE_RESOLUTION * WAVE_RESOLUTION;
        queue.write_buffer(
            &self.displacement_uniform,
            0,
            &words([
                square,
                square * 2,
                WAVE_RESOLUTION,
                0,
                choppiness.to_bits(),
                0,
                0,
                0,
            ]),
        );
        queue.write_buffer(
            &self.gradient_uniform,
            0,
            &words([choppiness.to_bits(), 0, 0, 0]),
        );
    }

    pub(super) fn record(&self, encoder: &mut CommandEncoder, current: usize) {
        for (target, pipeline, binding) in [
            (
                &self.displacement_views[current],
                &self.displacement_pipeline,
                &self.displacement_binding,
            ),
            (
                &self.gradient_view,
                &self.gradient_pipeline,
                &self.gradient_bindings[current],
            ),
        ] {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("ocean wave surface"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, binding, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    pub(super) fn displacement(&self, index: usize) -> &Texture {
        &self.displacement[index]
    }
    pub(super) fn displacement_view(&self, index: usize) -> &TextureView {
        &self.displacement_views[index]
    }
    pub(super) fn gradients(&self) -> &Texture {
        &self.gradients
    }
    pub(super) fn gradient_view(&self) -> &TextureView {
        &self.gradient_view
    }
    pub(super) fn uniform_bytes(&self) -> u64 {
        self.displacement_uniform.size() + self.gradient_uniform.size()
    }
}
