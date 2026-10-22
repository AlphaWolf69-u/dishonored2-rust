//! Retained 4096x1 fog-opacity curves from 3110e0's two installed programs.
//! Special environment image-track publication remains the scene owner's job.
use crate::rendering::fog::lookup::{FogLookupHistory, FogLookupPlan};
use bevy::{
    color::LinearRgba,
    math::Vec4,
    prelude::default,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

pub struct FogLookupOverrides<'a> {
    pub views: [&'a TextureView; 4],
    pub weights: Vec4,
}
struct Curve {
    texture: Texture,
    view: TextureView,
    layer: u32,
}
pub struct FogLookupPass {
    curves: [Curve; 3],
    array_view: TextureView,
    history: FogLookupHistory,
    final_image: usize,
    layout: BindGroupLayout,
    pipelines: [RenderPipeline; 2],
    uniforms: [UniformBuffer<Vec4>; 2],
    bindings: [Option<([TextureViewId; 5], BindGroup)>; 2],
}
impl FogLookupPass {
    /// Source36d0c0 fills all three R16F banks with half-float one. The caller
    /// supplies its actual previous raw blend weight, separately from pixels.
    pub fn new(device: &RenderDevice, queue: &RenderQueue, previous_weight: f32) -> Self {
        let ones = [0x00_u8, 0x3c].repeat(4096 * 3);
        let texture = device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("fog curve history"),
                size: Extent3d {
                    width: 4096,
                    height: 1,
                    depth_or_array_layers: 3,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::R16Float,
                usage: TextureUsages::TEXTURE_BINDING
                    | TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::COPY_SRC,
                view_formats: &[],
            },
            TextureDataOrder::MipMajor,
            &ones,
        );
        Self::from_texture(device, texture, previous_weight)
    }
    /// Reuse the same resident allocation bound by material receivers. Layers
    /// hold the three independent banks; no copy into another texture is needed.
    pub fn shared(
        device: &RenderDevice,
        texture: &Texture,
        previous_weight: f32,
    ) -> Result<Self, String> {
        if texture.size()
            != (Extent3d {
                width: 4096,
                height: 1,
                depth_or_array_layers: 3,
            })
            || texture.format() != TextureFormat::R16Float
            || texture.mip_level_count() != 1
            || texture.sample_count() != 1
            || !texture
                .usage()
                .contains(TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT)
        {
            return Err("shared fog lookup requires three renderable 4096x1 R16F layers".into());
        }
        Ok(Self::from_texture(device, texture.clone(), previous_weight))
    }
    fn from_texture(device: &RenderDevice, texture: Texture, previous_weight: f32) -> Self {
        let array_view = texture.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..default()
        });
        let curves = std::array::from_fn(|index| {
            let view = texture.create_view(&TextureViewDescriptor {
                dimension: Some(TextureViewDimension::D2),
                base_array_layer: index as u32,
                array_layer_count: Some(1),
                ..default()
            });
            Curve {
                texture: texture.clone(),
                view,
                layer: index as u32,
            }
        });
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("fog curve blending"),
            source: ShaderSource::Wgsl(
                concat!(
                    include_str!("../../../assets/shaders/dh2_point_sampling.wgsl"),
                    "\n",
                    include_str!("../../../assets/shaders/dh2_fog_lookup.wgsl")
                )
                .into(),
            ),
        });
        let mut entries = vec![BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::FRAGMENT,
            count: None,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: BufferSize::new(16),
            },
        }];
        entries.extend((1..=5).map(|binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            count: None,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: true },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
        }));
        let layout = device.create_bind_group_layout(Some("fog curve inputs"), &entries);
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("fog curve program"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipelines = std::array::from_fn(|i| {
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some("fog curve blend or override"),
                layout: Some(&pipeline_layout),
                vertex: RawVertexState {
                    module: &module,
                    entry_point: Some("vertex"),
                    compilation_options: default(),
                    buffers: &[],
                },
                fragment: Some(RawFragmentState {
                    module: &module,
                    entry_point: Some(if i == 0 {
                        "blend_fog_lookup"
                    } else {
                        "override_fog_lookup"
                    }),
                    compilation_options: default(),
                    targets: &[Some(ColorTargetState {
                        format: TextureFormat::R16Float,
                        blend: None,
                        write_mask: ColorWrites::RED,
                    })],
                }),
                primitive: default(),
                depth_stencil: None,
                multisample: default(),
                multiview_mask: None,
                cache: None,
            })
        });
        Self {
            curves,
            array_view,
            history: FogLookupHistory::new(previous_weight),
            final_image: 1,
            layout,
            pipelines,
            uniforms: std::array::from_fn(|_| UniformBuffer::default()),
            bindings: std::array::from_fn(|_| None),
        }
    }
    pub fn view(&self) -> &TextureView {
        &self.curves[self.final_image].view
    }
    pub fn array_view(&self) -> &TextureView {
        &self.array_view
    }
    pub fn texture_id(&self) -> TextureId {
        self.curves[0].texture.id()
    }

    fn bind(&mut self, device: &RenderDevice, index: usize, views: [&TextureView; 5]) {
        let ids = views.map(TextureView::id);
        if self.bindings[index]
            .as_ref()
            .is_some_and(|old| old.0 == ids)
        {
            return;
        }
        let mut entries = vec![BindGroupEntry {
            binding: 0,
            resource: self.uniforms[index].binding().unwrap(),
        }];
        entries.extend(
            views
                .into_iter()
                .enumerate()
                .map(|(i, view)| BindGroupEntry {
                    binding: i as u32 + 1,
                    resource: BindingResource::TextureView(view),
                }),
        );
        self.bindings[index] = Some((
            ids,
            device.create_bind_group(Some("fog curve images"), &self.layout, &entries),
        ));
    }
    fn draw(&self, encoder: &mut CommandEncoder, mode: usize, output: usize) {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("fog curve blend"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &self.curves[output].view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(LinearRgba::BLACK.into()),
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
        pass.set_pipeline(&self.pipelines[mode]);
        pass.set_bind_group(0, &self.bindings[mode].as_ref().unwrap().1, &[]);
        pass.draw(0..3, 0..1);
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        weight: f32,
        overrides: Option<FogLookupOverrides<'_>>,
    ) -> FogLookupPlan {
        let weights = overrides.as_ref().map_or(Vec4::ZERO, |v| v.weights);
        let plan = self.history.prepare(weight, weights.to_array());
        self.uniforms[0].set(Vec4::splat(plan.weight));
        self.uniforms[0].write_buffer(device, queue);
        let source = self.curves[plan.source].view.clone();
        // Base variant reads only the first two images; retain one explicit
        // layout without allocating dummy textures for unused shader slots.
        self.bind(device, 0, [&source, target, &source, &source, &source]);
        self.draw(encoder, 0, plan.output);
        if plan.apply_overrides {
            let overrides = overrides.unwrap();
            self.uniforms[1].set(weights);
            self.uniforms[1].write_buffer(device, queue);
            let output = self.curves[plan.output].view.clone();
            self.bind(
                device,
                1,
                [
                    &output,
                    overrides.views[0],
                    overrides.views[1],
                    overrides.views[2],
                    overrides.views[3],
                ],
            );
            self.draw(encoder, 1, 2);
        }
        self.final_image = plan.final_image;
        plan
    }
}

#[cfg(test)]
#[path = "fog_lookup/tests.rs"]
mod tests;
