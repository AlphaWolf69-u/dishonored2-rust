//!309da0/309b50: projected HDR sun mask, radial scattering and filter visibility.
use super::{
    coarse_depth::DepthPyramid,
    fog::{FogFrame, FogParameters, FogView},
    fog_rays::{FogRayPipelines, FogRayUniform},
    fog_sun_mask::{FogSunMaskPipelines, FogSunMaskUniform, FogSunProgram},
    fog_targets::{FogAttachment, FogExtents},
};
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
pub struct SunRaySources(pub Option<crate::render_state::RenderState>);

pub struct SunRays {
    extent: FogExtents,
    state: crate::render_state::RenderState,
    depth_source: TextureId,
    depth: DepthPyramid,
    linear: TextureView,
    stencil: TextureView,
    colour: FogAttachment,
    visibility: FogAttachment,
    masks: FogSunMaskPipelines,
    rays: FogRayPipelines,
    sun: UniformBuffer<FogSunMaskUniform>,
    world: UniformBuffer<FogParameters>,
    radial: [UniformBuffer<FogRayUniform>; 2],
    clamp: Sampler,
    bindings: Option<([TextureViewId; 4], BindGroup, [BindGroup; 2])>,
    scattering: bool,
    shadowed: bool,
    turbulence: bool,
}
impl SunRays {
    pub fn new(
        device: &RenderDevice,
        programs: &super::tiled_lighting_gpu::TlfPipelines,
        extent: FogExtents,
        state: crate::render_state::RenderState,
        alpha: &super::alpha_depth::AlphaDepthPass,
        downsample: i32,
        frame: &FogFrame,
        view: FogView,
        projection: crate::rendering::fog::rays::RayProjection,
    ) -> Result<Self, String> {
        let depth = DepthPyramid::new(device, programs, &alpha.full)?;
        let scale = [
            extent.full.x as f32 * 0.5 / depth.texture().width() as f32,
            extent.full.y as f32 * 0.5 / depth.texture().height() as f32,
        ];
        let linear = depth.texture().create_view(&TextureViewDescriptor {
            mip_level_count: Some(1),
            ..default()
        });
        let stencil = alpha.reduced.create_view(&TextureViewDescriptor {
            aspect: TextureAspect::StencilOnly,
            ..default()
        });
        let mask_extent =
            ((extent.full + UVec2::splat(15)) >> (downsample as u32 + 3)).max(UVec2::ONE);
        Ok(Self {
            extent,
            state,
            depth_source: alpha.full.id(),
            depth,
            linear,
            stencil,
            colour: FogAttachment::new(
                device,
                "sun colour mask",
                extent.world,
                TextureFormat::Rgba16Float,
            ),
            visibility: FogAttachment::new(
                device,
                "sun ray filter visibility",
                mask_extent,
                TextureFormat::R8Unorm,
            ),
            masks: FogSunMaskPipelines::with_depth_scale(
                device,
                &FogSunProgram::new(device),
                scale,
            ),
            rays: FogRayPipelines::with_sampled_stencil(device, state, scale)?,
            sun: UniformBuffer::from(FogSunMaskUniform::new(frame.rays,projection,view,
                Mat4::from_cols(Vec4::X,Vec4::Z,-Vec4::Y,Vec4::W)*view.world_from_view,extent.world)),
            world: UniformBuffer::from(frame.parameters),
            radial: [default(), default()],
            bindings: None,
            clamp: device.create_sampler(&SamplerDescriptor {
                min_filter: FilterMode::Linear,
                mag_filter: FilterMode::Linear,
                ..default()
            }),
            scattering: false,
            shadowed: false,
            turbulence: false,
        })
    }
    pub fn matches(
        &self,
        extent: FogExtents,
        state: crate::render_state::RenderState,
        alpha: &super::alpha_depth::AlphaDepthPass,
    ) -> bool {
        self.extent == extent && self.state == state && self.depth_source == alpha.full.id()
    }
    pub fn prepare(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        frame: &FogFrame,
        view: FogView,
        projection: crate::rendering::fog::rays::RayProjection,
        inverse_projection: crate::render_shadow::ShadowMatrix,
        hdr: &TextureView,
        shadow: &TextureView,
        canvas: &TextureView,
        interior: &TextureView,
        programs: &super::tiled_lighting_gpu::TlfPipelines,
    ) -> Result<(), String> {
        self.depth.record(
            queue,
            encoder,
            programs,
            self.extent.full.to_array(),
            inverse_projection,
        )?;
        let game_from_view =
            Mat4::from_cols(Vec4::X, Vec4::Z, -Vec4::Y, Vec4::W) * view.world_from_view;
        self.sun.set(FogSunMaskUniform::new(
            frame.rays,
            projection,
            view,
            game_from_view,
            self.extent.world,
        ));
        self.world.set(frame.parameters);
        self.sun.write_buffer(device, queue);
        self.world.write_buffer(device, queue);
        for (uniform, extent) in self.radial.iter_mut().zip([
            self.extent.world,
            UVec2::new(
                self.visibility.texture.width(),
                self.visibility.texture.height(),
            ),
        ]) {
            uniform.set(FogRayUniform::new(frame.rays, projection, extent));
            uniform.write_buffer(device, queue);
        }
        self.scattering = frame.rays.add_scattering;
        self.shadowed = frame.parameters.flags.x & 64 != 0;
        self.turbulence = frame.parameters.flags.x & 128 != 0;
        let ids = [hdr.id(), shadow.id(), canvas.id(), interior.id()];
        // Uniform allocations are retained. Bind only when source identities change.
        if self.bindings.as_ref().is_none_or(|v| v.0 != ids) {
            let mut entries = vec![
                BindGroupEntry {
                    binding: 0,
                    resource: self.sun.binding().unwrap(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: self.world.binding().unwrap(),
                },
            ];
            for (i, v) in [hdr, &self.linear, shadow].into_iter().enumerate() {
                entries.push(BindGroupEntry {
                    binding: 2 + i as u32,
                    resource: BindingResource::TextureView(v),
                });
            }
            entries.push(BindGroupEntry {
                binding: 5,
                resource: BindingResource::Sampler(&self.clamp),
            });
            let mask =
                device.create_bind_group(Some("sun colour inputs"), &self.masks.layout, &entries);
            let radial = std::array::from_fn(|index| {
                let mut entries = vec![
                    BindGroupEntry {
                        binding: 0,
                        resource: self.radial[index].binding().unwrap(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: self.world.binding().unwrap(),
                    },
                ];
                for (i, v) in [&self.colour.view, &self.linear, canvas, interior]
                    .into_iter()
                    .enumerate()
                {
                    entries.push(BindGroupEntry {
                        binding: 2 + i as u32,
                        resource: BindingResource::TextureView(v),
                    });
                }
                entries.push(BindGroupEntry {
                    binding: 6,
                    resource: BindingResource::Sampler(&self.clamp),
                });
                entries.push(BindGroupEntry {
                    binding: 7,
                    resource: BindingResource::TextureView(&self.stencil),
                });
                device.create_bind_group(Some("sun radial inputs"), &self.rays.layout, &entries)
            });
            self.bindings = Some((ids, mask, radial));
        }
        Ok(())
    }
    pub fn record(&self, encoder: &mut CommandEncoder, target: &TextureView) {
        let Some((_, mask, radial)) = &self.bindings else {
            return;
        };
        for (label, view, pipeline, binding, clear, count) in [
            (
                "sun colour mask",
                &self.colour.view,
                self.masks.pipeline(self.scattering, self.shadowed),
                mask,
                true,
                6,
            ),
            (
                "sun radial scattering",
                target,
                self.rays.scattering(self.turbulence),
                &radial[0],
                false,
                3,
            ),
            (
                "sun ray visibility",
                &self.visibility.view,
                &self.rays.visibility,
                &radial[1],
                true,
                3,
            ),
        ] {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: if clear {
                            LoadOp::Clear(LinearRgba::NONE.into())
                        } else {
                            LoadOp::Load
                        },
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, binding, &[]);
            pass.draw(0..count, 0..1);
        }
    }
    pub fn visibility(&self) -> &TextureView {
        &self.visibility.view
    }
}
