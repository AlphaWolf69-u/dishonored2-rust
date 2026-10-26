//! Submission and persistent resources for the verified world-fog passes.
//! The scene owner supplies same-frame depth, sky masking and shadow resources.
use super::{
    fog::{FogCanvas, FogParameters, FogView},
    fog_filter::{FogFilter, FogFilterSources},
    fog_pipeline::FogPipelines,
    fog_targets::{FogAttachment, FogExtents, FogTargets},
};
use bevy::{
    color::LinearRgba,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

pub struct FogInputs<'a> {
    pub shadow_parameters: &'a super::fog_shadow::FogShadowParameters,
    pub projected_height: &'a TextureView,
    pub areas: Option<&'a super::fog_volumes::AreaFogPass>,
    pub scattering: Option<&'a super::fog_light_scattering::LightScatteringPass>,
    pub settings: crate::platform::game_settings::FogRenderSettings,
    pub ray_mask: Option<&'a TextureView>,
    pub shadowed_volumes: bool,
    /// 309430's local-light flag at volumetric parameters +0x99, consumed by
    /// 307530's blur gate independently of sunlight shadowing.
    pub shadowed_local_lights: bool,
    pub depth: &'a TextureView,
    pub sky_mask: &'a TextureView,
    pub curve: &'a TextureView,
    pub noise: &'a TextureView,
    pub atlas: &'a TextureView,
    pub parallel: &'a Buffer,
}

pub use super::position_meshes::PositionDraw as InteriorDraw;

/// Recorded fog radiance, available to water before final HDR composition.
/// Borrows the pass so its targets cannot be resized or rebound in between.
#[must_use = "record fog composition after any water colour passes"]
pub struct FogSource<'a> {
    view: &'a TextureView,
    pipeline: &'a RenderPipeline,
    binding: &'a BindGroup,
}

impl FogSource<'_> {
    pub fn view(&self) -> &TextureView {
        self.view
    }

    pub fn record_composite(self, encoder: &mut CommandEncoder, hdr: &TextureView) {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("fog HDR composition"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: hdr,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(self.pipeline);
        pass.set_bind_group(0, self.binding, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[derive(PartialEq, Eq)]
struct InputIds([TextureViewId; 6], BufferId);
impl FogInputs<'_> {
    fn ids(&self) -> InputIds {
        InputIds(
            [
                self.depth.id(),
                self.sky_mask.id(),
                self.curve.id(),
                self.noise.id(),
                self.atlas.id(),
                self.projected_height.id(),
            ],
            self.parallel.id(),
        )
    }
}
struct Bindings {
    ids: InputIds,
    interior: BindGroup,
    canvas: BindGroup,
    shadow: BindGroup,
    world: BindGroup,
    upscale: BindGroup,
    upscale_slope: BindGroup,
}

struct LightCanvas {
    parameters: UniformBuffer<FogCanvas>,
    target: FogAttachment,
    binding: Option<(TextureViewId, BindGroup)>,
}

pub struct FogPass {
    shadow_parameters: Buffer,
    pub targets: FogTargets,
    pipelines: FogPipelines,
    view: UniformBuffer<FogView>,
    parameters: UniformBuffer<FogParameters>,
    canvas: UniformBuffer<FogCanvas>,
    light_canvas: Option<LightCanvas>,
    clamp: Sampler,
    point: Sampler,
    wrap: Sampler,
    comparison: Sampler,
    bindings: Option<Bindings>,
    depth_binding: Option<(TextureViewId,BindGroup)>,
    filter: Option<FogFilter>,
    ray_visibility: Option<super::fog_filter::visibility::RayVisibilityFilter>,
}

impl FogPass {
    pub fn pipelines(&self) -> &FogPipelines {
        &self.pipelines
    }
    pub fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        extents: FogExtents,
        hdr_format: TextureFormat,
        view: FogView,
        parameters: FogParameters,
        canvas: FogCanvas,
    ) -> Self {
        Self::with_targets(
            device,
            queue,
            FogTargets::new(device, extents),
            hdr_format,
            view,
            parameters,
            canvas,
        )
    }
    pub fn with_targets(
        device: &RenderDevice,
        queue: &RenderQueue,
        targets: FogTargets,
        hdr_format: TextureFormat,
        view: FogView,
        parameters: FogParameters,
        canvas: FogCanvas,
    ) -> Self {
        let mut view = UniformBuffer::from(view);
        let mut parameters = UniformBuffer::from(parameters);
        let mut canvas = UniformBuffer::from(canvas);
        view.write_buffer(device, queue);
        parameters.write_buffer(device, queue);
        canvas.write_buffer(device, queue);
        let sampler = |filter, address, compare| {
            device.create_sampler(&SamplerDescriptor {
                min_filter: filter,
                mag_filter: filter,
                address_mode_u: address,
                address_mode_v: address,
                address_mode_w: address,
                compare,
                ..Default::default()
            })
        };
        Self {
            shadow_parameters: device.create_buffer(&BufferDescriptor {
                label: Some("fog shadow view parameters"),
                size: 64,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            targets,
            pipelines: FogPipelines::new(device, hdr_format),
            view,
            parameters,
            canvas,
            light_canvas: None,
            clamp: sampler(FilterMode::Linear, AddressMode::ClampToEdge, None),
            point: sampler(FilterMode::Nearest, AddressMode::ClampToEdge, None),
            wrap: sampler(FilterMode::Linear, AddressMode::Repeat, None),
            comparison: super::shadow_pipeline::shadow_comparison_sampler(device),
            bindings: None,
            depth_binding: None,
            filter: None,
            ray_visibility: None,
        }
    }

    pub fn update(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        extents: FogExtents,
        view: FogView,
        parameters: FogParameters,
        canvas: FogCanvas,
    ) {
        if self.targets.resize(device, extents) {
            self.bindings = None;
            self.ray_visibility = None;
        }
        self.view.set(view);
        self.parameters.set(parameters);
        self.canvas.set(canvas);
        self.view.write_buffer(device, queue);
        self.parameters.write_buffer(device, queue);
        self.canvas.write_buffer(device, queue);
    }

    pub fn update_shared(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        extents: FogExtents,
        view: FogView,
        parameters: FogParameters,
        canvas: FogCanvas,
        world: &bevy::render::texture::GpuImage,
        canvas_image: &bevy::render::texture::GpuImage,
    ) -> Result<(), String> {
        if self.targets.extents != extents
            || self.targets.world.texture.id() != world.texture.id()
            || self.targets.canvas.texture.id() != canvas_image.texture.id()
        {
            if self.targets.extents != extents {
                self.ray_visibility = None;
            }
            self.targets = FogTargets::shared(device, extents, world, canvas_image)?;
            self.bindings = None;
        }
        self.update(device, queue, extents, view, parameters, canvas);
        Ok(())
    }

    /// The second 3110e0 canvas has independent motion and environment inputs.
    /// Never substitute the world-fog canvas when its publication is absent.
    pub fn update_light_canvas(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        parameters: Option<FogCanvas>,
    ) {
        let Some(parameters) = parameters else {
            self.light_canvas = None;
            return;
        };
        let extent = self.targets.extents.canvas;
        let canvas = self.light_canvas.get_or_insert_with(|| LightCanvas {
            parameters: UniformBuffer::from(parameters),
            target: FogAttachment::new(
                device,
                "light scattering turbulence",
                extent,
                TextureFormat::R8Unorm,
            ),
            binding: None,
        });
        if canvas.target.texture.width() != extent.x || canvas.target.texture.height() != extent.y {
            canvas.target = FogAttachment::new(
                device,
                "light scattering turbulence",
                extent,
                TextureFormat::R8Unorm,
            );
        }
        canvas.parameters.set(parameters);
        canvas.parameters.write_buffer(device, queue);
    }

    pub fn light_canvas_view(&self) -> Option<&TextureView> {
        self.light_canvas.as_ref().map(|canvas| &canvas.target.view)
    }

    fn bind(&mut self, device: &RenderDevice, inputs: &FogInputs) {
        let ids = inputs.ids();
        if self.bindings.as_ref().is_some_and(|old| old.ids == ids) {
            return;
        }
        let view = self.view.buffer().unwrap();
        let parameters = self.parameters.buffer().unwrap();
        let canvas = self.canvas.buffer().unwrap();
        fn texture(binding: u32, texture: &TextureView) -> BindGroupEntry<'_> {
            BindGroupEntry {
                binding,
                resource: BindingResource::TextureView(texture),
            }
        }
        fn sampler(binding: u32, sampler: &Sampler) -> BindGroupEntry<'_> {
            BindGroupEntry {
                binding,
                resource: BindingResource::Sampler(sampler),
            }
        }
        fn buffer(binding: u32, buffer: &Buffer) -> BindGroupEntry<'_> {
            BindGroupEntry {
                binding,
                resource: buffer.as_entire_binding(),
            }
        }
        let make = |pipeline: &RenderPipeline, entries: &[BindGroupEntry]| {
            BindGroup::from(
                device
                    .wgpu_device()
                    .create_bind_group(&BindGroupDescriptor {
                        label: Some("fog pass inputs"),
                        layout: &pipeline.get_bind_group_layout(0),
                        entries,
                    }),
            )
        };
        self.bindings = Some(Bindings {
            ids,
            interior: device.create_bind_group(
                None,
                &self.pipelines.interior_view_layout,
                &[
                    buffer(0, view),
                    texture(3, inputs.depth),
                    sampler(7, &self.clamp),
                ],
            ),
            canvas: make(
                &self.pipelines.canvas,
                &[
                    buffer(0, view),
                    texture(8, inputs.noise),
                    buffer(10, canvas),
                    sampler(11, &self.wrap),
                ],
            ),
            shadow: make(
                &self.pipelines.shadow,
                &[
                    buffer(0, view),
                    texture(3, inputs.depth),
                    sampler(7, &self.clamp),
                    texture(14, inputs.atlas),
                    buffer(15, inputs.parallel),
                    sampler(16, &self.comparison),
                    texture(17, inputs.projected_height),
                    buffer(18, &self.shadow_parameters),
                ],
            ),
            world: make(
                &self.pipelines.world,
                &[
                    buffer(0, view),
                    buffer(1, parameters),
                    texture(2, &self.targets.depth.view),
                    texture(4, &self.targets.interior.view),
                    texture(5, inputs.sky_mask),
                    texture(6, inputs.curve),
                    sampler(7, &self.clamp),
                    texture(9, &self.targets.shadow.view),
                    texture(12, &self.targets.canvas.view),
                ],
            ),
            upscale: make(
                &self.pipelines.upscale,
                &[
                    buffer(0, view),
                    texture(2, &self.targets.depth.view),
                    texture(3, inputs.depth),
                    sampler(7, &self.clamp),
                    texture(12, &self.targets.world.view),
                    sampler(13, &self.point),
                ],
            ),
            upscale_slope: make(
                &self.pipelines.upscale_slope,
                &[
                    buffer(0, view),
                    texture(3, inputs.depth),
                    texture(12, &self.targets.world.view),
                    sampler(13, &self.point),
                ],
            ),
        });
    }

    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        inputs: &FogInputs,
        interiors: &[InteriorDraw],
        hdr: &TextureView,
    ) {
        self.record_sources(device, queue, encoder, inputs, interiors)
            .record_composite(encoder, hdr);
    }

    /// Record source production without touching HDR. The game's water task
    /// consumes this source before its subsequent fog composition task.
    pub fn record_sources(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        inputs: &FogInputs,
        interiors: &[InteriorDraw],
    ) -> FogSource<'_> {
        self.record_sources_with_underwater(device,queue,encoder,inputs,interiors,false,None,|_,_|{})
    }

    /// Shared downsampledWaterDepth producer, also needed when fog is disabled.
    pub fn record_linear_depth(
        &mut self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        depth: &TextureView,
    ) {
        if self
            .depth_binding
            .as_ref()
            .is_none_or(|(id, _)| *id != depth.id())
        {
            let binding = device.create_bind_group(
                Some("linear water depth inputs"),
                &self.pipelines.depth.get_bind_group_layout(0).into(),
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: self.view.binding().unwrap(),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::TextureView(depth),
                    },
                    BindGroupEntry {
                        binding: 13,
                        resource: BindingResource::Sampler(&self.point),
                    },
                ],
            );
            self.depth_binding = Some((depth.id(), binding));
        }
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("fog linear depth reduction"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &self.targets.depth.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(LinearRgba::NONE.into()),
                        store: StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipelines.depth);
            pass.set_bind_group(0, &self.depth_binding.as_ref().unwrap().1, &[]);
            pass.draw(0..3, 0..1);
        }
    }
    ///307530 inserts underwater radiance after light scattering and before
    ///the shared bilateral filter. The callback receives the real fog target.
    pub fn record_sources_with_underwater(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        inputs: &FogInputs,
        interiors: &[InteriorDraw],
        underwater_shadows: bool,
        sun_rays: Option<&super::sun_rays::SunRays>,
        underwater: impl FnOnce(&mut CommandEncoder,&TextureView),
    ) -> FogSource<'_> {
        self.bind(device, inputs);
        queue.write_buffer(
            &self.shadow_parameters,
            0,
            &inputs.shadow_parameters.uniform_bytes(),
        );
        if let Some(canvas) = &mut self.light_canvas {
            if canvas
                .binding
                .as_ref()
                .is_none_or(|(id, _)| *id != inputs.noise.id())
            {
                let binding = device.create_bind_group(
                    Some("light scattering canvas inputs"),
                    &self.pipelines.canvas.get_bind_group_layout(0).into(),
                    &[
                        BindGroupEntry {
                            binding: 0,
                            resource: self.view.binding().unwrap(),
                        },
                        BindGroupEntry {
                            binding: 8,
                            resource: BindingResource::TextureView(inputs.noise),
                        },
                        BindGroupEntry {
                            binding: 10,
                            resource: canvas.parameters.binding().unwrap(),
                        },
                        BindGroupEntry {
                            binding: 11,
                            resource: BindingResource::Sampler(&self.wrap),
                        },
                    ],
                );
                canvas.binding = Some((inputs.noise.id(), binding));
            }
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("light scattering turbulence"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &canvas.target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(LinearRgba::NONE.into()),
                        store: StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipelines.canvas);
            pass.set_bind_group(0, &canvas.binding.as_ref().unwrap().1, &[]);
            pass.draw(0..3, 0..1);
        }
        self.record_linear_depth(device,encoder,inputs.depth);
        let bindings = self.bindings.as_ref().unwrap();
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("fog interior signed depth"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &self.targets.interior.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(LinearRgba::NONE.into()),
                        store: StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_bind_group(0, &bindings.interior, &[]);
            for draw in interiors {
                pass.set_pipeline(draw.pipeline);
                pass.set_bind_group(1, draw.models, &[]);
                pass.set_vertex_buffer(0, *draw.positions.slice(..));
                if let Some((indices, format, range, base)) = &draw.indices {
                    pass.set_index_buffer(*indices.slice(..), *format);
                    pass.draw_indexed(range.clone(), *base, draw.instances.clone());
                } else {
                    pass.draw(draw.vertices.clone(), draw.instances.clone());
                }
            }
        }
        // 307530 calls 309a20 only when world fog or a shadowed volume
        // requests sunlight visibility. Keep clearing the retained attachment
        // so disabled frames cannot expose an earlier frame through readback.
        let shadow_enabled = self.parameters.get().flags.x & 64 != 0 || inputs.shadowed_volumes || underwater_shadows;
        for (label, pipeline, bind, target, enabled) in [
            (
                "fog turbulence",
                &self.pipelines.canvas,
                &bindings.canvas,
                &self.targets.canvas.view,
                true,
            ),
            (
                "fog sunlight visibility",
                &self.pipelines.shadow,
                &bindings.shadow,
                &self.targets.shadow.view,
                shadow_enabled,
            ),
            (
                "world fog radiance",
                &self.pipelines.world,
                &bindings.world,
                &self.targets.world.view,
                true,
            ),
        ] {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(LinearRgba::NONE.into()),
                        store: StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if enabled {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, bind, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        if let Some(rays) = sun_rays {rays.record(encoder,&self.targets.world.view);}
        if let Some(areas) = inputs.areas {
            areas.record(encoder, &self.targets.world.view);
        }
        if let Some(scattering) = inputs.scattering {
            scattering.record(encoder, &self.targets.world.view);
        }
        underwater(encoder,&self.targets.world.view);
        if let Some(mask)=inputs.ray_mask {
            let extent=((self.targets.extents.full+bevy::math::UVec2::splat(15))
                >> (inputs.settings.downsample as u32+3)).max(bevy::math::UVec2::ONE);
            self.ray_visibility.get_or_insert_with(||super::fog_filter::visibility::RayVisibilityFilter::new(device,extent))
                .record(device,queue,encoder,mask,inputs.settings.blur_radius);
        }
        // 307530 also filters when admitted local lights cast volumetric
        // shadows (+0x99); their eight-tap scattering contains the same dither.
        if !inputs.settings.skip_blur
            && (self.parameters.get().flags.x & 64 != 0
                || inputs.ray_mask.is_some()
                || inputs.shadowed_volumes
                || inputs.shadowed_local_lights)
        {
            self.filter
                .get_or_insert_with(|| FogFilter::new(device))
                .record(
                    device,
                    queue,
                    encoder,
                    inputs.settings,
                    FogFilterSources {
                        color: &self.targets.world.view,
                        scratch: &self.targets.filtered.view,
                        depth: &self.targets.depth.view,
                        rays: inputs.ray_mask,
                        extent: self.targets.extents.world,
                    },
                );
        }
        let (pipeline, bind) = if inputs.settings.nearest_upscale {
            (&self.pipelines.upscale, &bindings.upscale)
        } else {
            (&self.pipelines.upscale_slope, &bindings.upscale_slope)
        };
        FogSource {
            view: &self.targets.world.view,
            pipeline,
            binding: bind,
        }
    }
}
