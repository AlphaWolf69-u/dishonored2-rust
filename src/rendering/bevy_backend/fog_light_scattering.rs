//! Full-screen local-light fog. Inputs are supplied by the same-view TLF owner;
//! this pass neither fabricates light lists nor marks an undrawn atlas ready.
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
pub struct LocalFogFrame {
    pub camera: Option<Entity>,
    pub shadow_revision: Option<u64>,
    pub lights: std::sync::Arc<Vec<[u32; 28]>>,
    /// Authored texture identity for each packet, before the shared pool assigns slices.
    pub area_images: std::sync::Arc<Vec<Option<Handle<Image>>>>,
}

#[derive(Clone, Copy, ShaderType)]
pub struct LightScatterView {
    pub world_from_clip: Mat4,
    pub camera_far: Vec4,
    pub position_to_view_texture: Vec4,
    pub canvas_scale_jitter: Vec4,
    pub grid: UVec4,
    pub black_white: Vec4,
}

pub struct LightScatteringInputs<'a> {
    pub lights: &'a Buffer,
    pub pool: &'a Buffer,
    /// Separate scattering heads, THREE horizontal type planes; END_OF_LIST
    /// initialized before the scattering cull. The shared counter is reset once.
    pub heads: &'a Buffer,
    pub shadows: &'a Buffer,
    pub atlas: &'a TextureView,
    pub depth: &'a TextureView,
    pub first_canvas: &'a TextureView,
    pub second_canvas: &'a TextureView,
}
impl LightScatteringInputs<'_> {
    fn ids(&self) -> ([BufferId; 4], [TextureViewId; 4]) {
        (
            [
                self.lights.id(),
                self.pool.id(),
                self.heads.id(),
                self.shadows.id(),
            ],
            [
                self.atlas.id(),
                self.depth.id(),
                self.first_canvas.id(),
                self.second_canvas.id(),
            ],
        )
    }
}

pub struct LightScatteringPass {
    pipeline: RenderPipeline,
    layout: BindGroupLayout,
    view: UniformBuffer<LightScatterView>,
    bilinear: Sampler,
    comparison: Sampler,
    binding: Option<(([BufferId; 4], [TextureViewId; 4]), BindGroup)>,
}
impl LightScatteringPass {
    pub fn new(device: &RenderDevice, format: TextureFormat, initial: LightScatterView) -> Self {
        let source = format!(
            "{}\n{}\n{}\n{}",
            include_str!("../../../assets/shaders/dh2_colour_output.wgsl"),
            include_str!("../../../assets/shaders/dh2_shadow.wgsl"),
            include_str!("../../../assets/shaders/dh2_light_scattering.wgsl"),
            include_str!("../../../assets/shaders/dh2_light_scattering_pass.wgsl")
        );
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("local-light fog scattering"),
            source: ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("local-light fog accumulation"),
            layout: None,
            vertex: RawVertexState {
                module: &module,
                entry_point: Some("light_scatter_vertex"),
                compilation_options: default(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &module,
                entry_point: Some("light_scatter_fragment"),
                compilation_options: default(),
                targets: &[Some(ColorTargetState {
                    format,
                    write_mask: ColorWrites::ALL,
                    blend: Some(BlendState {
                        color: BlendComponent {
                            src_factor: BlendFactor::One,
                            dst_factor: BlendFactor::One,
                            operation: BlendOperation::Add,
                        },
                        // Installed PostFX/LightScatteing: ZERO/ONE alpha preserves
                        // the destination fog density while adding scattered RGB.
                        alpha: BlendComponent {
                            src_factor: BlendFactor::Zero,
                            dst_factor: BlendFactor::One,
                            operation: BlendOperation::Add,
                        },
                    }),
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
        });
        let layout = pipeline.get_bind_group_layout(0).into();
        let bilinear = device.create_sampler(&SamplerDescriptor {
            min_filter: FilterMode::Linear,
            mag_filter: FilterMode::Linear,
            address_mode_u: AddressMode::Repeat,
            address_mode_v: AddressMode::Repeat,
            address_mode_w: AddressMode::Repeat,
            ..default()
        });
        let comparison = super::shadow_pipeline::shadow_comparison_sampler(device);
        Self {
            pipeline,
            layout,
            view: UniformBuffer::from(initial),
            bilinear,
            comparison,
            binding: None,
        }
    }

    pub fn prepare(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        view: LightScatterView,
        inputs: LightScatteringInputs,
    ) {
        self.view.set(view);
        self.view.write_buffer(device, queue);
        let ids = inputs.ids();
        if self.binding.as_ref().is_some_and(|(old, _)| *old == ids) {
            return;
        }
        let binding = device.create_bind_group(
            Some("local-light fog inputs"),
            &self.layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: self.view.binding().unwrap(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: inputs.lights.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: inputs.pool.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: inputs.heads.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: inputs.shadows.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: BindingResource::TextureView(inputs.atlas),
                },
                BindGroupEntry {
                    binding: 6,
                    resource: BindingResource::Sampler(&self.comparison),
                },
                BindGroupEntry {
                    binding: 7,
                    resource: BindingResource::TextureView(inputs.depth),
                },
                BindGroupEntry {
                    binding: 8,
                    resource: BindingResource::TextureView(inputs.first_canvas),
                },
                BindGroupEntry {
                    binding: 9,
                    resource: BindingResource::TextureView(inputs.second_canvas),
                },
                BindGroupEntry {
                    binding: 10,
                    resource: BindingResource::Sampler(&self.bilinear),
                },
            ],
        );
        self.binding = Some((ids, binding));
    }

    pub fn record(&self, encoder: &mut CommandEncoder, target: &TextureView) {
        let Some((_, binding)) = &self.binding else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("local-light fog accumulation"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, binding, &[]);
        pass.draw(0..3, 0..1);
    }
}
