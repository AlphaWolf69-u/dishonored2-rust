//! 303ae0's R16F foam-depth producer and four authored blur draws.
//! The water-depth attachment must include actual water geometry. View/scene
//! admission, quarter viewport/scissor and effective matrices belong to caller.
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

#[derive(Clone, Copy, Default, ShaderType)]
pub struct WaterFoamView {
    pub inverse_view_projection: Mat4,
    pub camera: Vec4,
    /// -inverseProjection[2].w, inverseProjection[3][2], [3][3], cameraForward.z.
    pub linear_depth: Vec4,
    /// globalViewInfos.zw in xy.
    pub near_far: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct BlurParameters {
    /// positionToViewTexture.zw, resolutionScale.xy.
    pixel_scale: Vec4,
    direction_boost: Vec4,
}

#[derive(Clone, Copy)]
pub struct WaterFoamRegion {
    pub viewport: [f32; 4],
    pub scissor: [u32; 4],
}

pub struct WaterFoamDepth {
    extent: UVec2,
    textures: [Texture; 2],
    views: [TextureView; 2],
    depth_pipeline: RenderPipeline,
    blur_pipeline: RenderPipeline,
    view: UniformBuffer<WaterFoamView>,
    blur: [UniformBuffer<BlurParameters>; 4],
    clamp: Sampler,
    depth_binding: Option<([TextureViewId; 2], BindGroup)>,
    blur_bindings: [BindGroup; 4],
}

impl WaterFoamDepth {
    pub fn new(device: &RenderDevice, queue: &RenderQueue, full: UVec2) -> Result<Self, String> {
        if full.min_element() == 0 || full.max_element() > i32::MAX as u32 - 3 {
            return Err(format!("invalid water foam extent {full:?}"));
        }
        // 303ae0 uses ceiling quarter dimensions, with a minimum of two.
        let extent = ((full + UVec2::splat(3)) >> 2u32).max(UVec2::splat(2));
        let textures = std::array::from_fn(|_| {
            device.create_texture(&TextureDescriptor {
                label: Some("water foam depth"),
                size: Extent3d {
                    width: extent.x,
                    height: extent.y,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::R16Float,
                usage: TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::TEXTURE_BINDING
                    | TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        });
        let views = textures.each_ref().map(|t| t.create_view(&default()));
        let pipeline = |label, source: &'static str| {
            let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
                label: Some(label),
                source: ShaderSource::Wgsl(source.into()),
            });
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some(label),
                layout: None,
                vertex: RawVertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: default(),
                    buffers: &[],
                },
                fragment: Some(RawFragmentState {
                    module: &shader,
                    entry_point: Some("fragment"),
                    compilation_options: default(),
                    targets: &[Some(ColorTargetState {
                        format: TextureFormat::R16Float,
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
        let depth_pipeline = pipeline(
            "water/FoamDepth",
            include_str!("../../../assets/shaders/dh2_water_foam_depth.wgsl"),
        );
        let blur_pipeline = pipeline(
            "PostFX separable Gaussian blur",
            concat!(
                include_str!("../../../assets/shaders/dh2_point_sampling.wgsl"),
                "\n",
                include_str!("../../../assets/shaders/dh2_separable_gaussian_blur.wgsl")
            ),
        );
        let mut view = UniformBuffer::<WaterFoamView>::default();
        view.write_buffer(device, queue);
        let mut blur: [UniformBuffer<BlurParameters>; 4] =
            std::array::from_fn(|_| UniformBuffer::default());
        for buffer in &mut blur {
            buffer.write_buffer(device, queue);
        }
        let sampler = |address| {
            device.create_sampler(&SamplerDescriptor {
                address_mode_u: address,
                address_mode_v: address,
                address_mode_w: address,
                min_filter: FilterMode::Nearest,
                mag_filter: FilterMode::Nearest,
                mipmap_filter: MipmapFilterMode::Nearest,
                ..default()
            })
        };
        let clamp = sampler(AddressMode::ClampToEdge);
        let blur_bindings = std::array::from_fn(|i| {
            device.create_bind_group(
                "water foam blur inputs",
                &blur_pipeline.get_bind_group_layout(0).into(),
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: blur[i].binding().unwrap(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(&views[i % 2]),
                    },
                ],
            )
        });
        Ok(Self {
            extent,
            textures,
            views,
            depth_pipeline,
            blur_pipeline,
            view,
            blur,
            clamp,
            depth_binding: None,
            blur_bindings,
        })
    }

    pub fn extent(&self) -> UVec2 {
        self.extent
    }
    pub fn texture(&self) -> &Texture {
        &self.textures[0]
    }
    pub fn view(&self) -> &TextureView {
        &self.views[0]
    }

    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        opaque: &TextureView,
        water: &TextureView,
        view: WaterFoamView,
        pixel_scale: Vec4,
        region: WaterFoamRegion,
    ) -> Result<(), String> {
        self.prepare(device, queue, opaque, water, view, pixel_scale, region)?;
        for step in 0..5 {
            self.record_step(encoder, step, region);
        }
        Ok(())
    }

    fn prepare(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        opaque: &TextureView,
        water: &TextureView,
        view: WaterFoamView,
        pixel_scale: Vec4,
        region: WaterFoamRegion,
    ) -> Result<(), String> {
        let [x, y, width, height] = region.viewport;
        let [sx, sy, sw, sh] = region.scissor;
        if ![x, y, width, height].into_iter().all(f32::is_finite)
            || x < 0.
            || y < 0.
            || width <= 0.
            || height <= 0.
            || x + width > self.extent.x as f32
            || y + height > self.extent.y as f32
            || sw == 0
            || sh == 0
            || sx.checked_add(sw).is_none_or(|v| v > self.extent.x)
            || sy.checked_add(sh).is_none_or(|v| v > self.extent.y)
        {
            return Err("water foam quarter viewport/scissor is outside its target".into());
        }
        self.view.set(view);
        self.view.write_buffer(device, queue);
        // Source303ae0, globals141ac8fd8/141ac8fdc and literal3f8ccccd.
        for (buffer, direction_boost) in self.blur.iter_mut().zip([
            Vec4::new(0., 1.5, 2., 0.),
            Vec4::new(4., 0., 3., 0.),
            Vec4::new(1.1, 0., 1., 0.),
            Vec4::new(0., 1.1, -1., 0.),
        ]) {
            buffer.set(BlurParameters {
                pixel_scale,
                direction_boost,
            });
            buffer.write_buffer(device, queue);
        }
        let ids = [opaque.id(), water.id()];
        if self
            .depth_binding
            .as_ref()
            .is_none_or(|cached| cached.0 != ids)
        {
            self.depth_binding = Some((
                ids,
                device.create_bind_group(
                    "water foam depths",
                    &self.depth_pipeline.get_bind_group_layout(0).into(),
                    &[
                        BindGroupEntry {
                            binding: 0,
                            resource: self.view.binding().unwrap(),
                        },
                        BindGroupEntry {
                            binding: 1,
                            resource: BindingResource::TextureView(opaque),
                        },
                        BindGroupEntry {
                            binding: 2,
                            resource: BindingResource::TextureView(water),
                        },
                        BindGroupEntry {
                            binding: 3,
                            resource: BindingResource::Sampler(&self.clamp),
                        },
                    ],
                ),
            ));
        }
        Ok(())
    }

    fn record_step(&self, encoder: &mut CommandEncoder, step: usize, region: WaterFoamRegion) {
        let [x, y, width, height] = region.viewport;
        let [sx, sy, sw, sh] = region.scissor;
        let output = if step == 0 { 0 } else { step % 2 };
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("water foam depth/blur"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &self.views[output],
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
        if step == 0 {
            pass.set_pipeline(&self.depth_pipeline);
            pass.set_bind_group(0, &self.depth_binding.as_ref().unwrap().1, &[]);
        } else {
            pass.set_pipeline(&self.blur_pipeline);
            pass.set_bind_group(0, &self.blur_bindings[step - 1], &[]);
            pass.set_viewport(x, y, width, height, 0., 1.);
            pass.set_scissor_rect(sx, sy, sw, sh);
        }
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
#[path = "water_foam_depth/tests.rs"]
mod tests;
