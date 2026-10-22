//! 2fdea0's wave-aware quarter-view water-line mask. Call before underwater
//! marking; collision height, shore distance and amplitude are real inputs.
use super::{
    ocean_surface::OceanSurfaceParameters,
    underwater_stencil::{UnderwaterStencilPass, UnderwaterStencilSource, UnderwaterStencilTarget},
};
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

#[derive(Clone, Copy, Default, ShaderType)]
pub struct WaterLineParameters {
    pub world_from_clip: Mat4,
    /// The actual quarter-view viewport, including its origin.
    pub viewport: Vec4,
    pub water_level: Vec4,
}
pub struct WaterLineInputs<'a> {
    pub amplitude_mask: &'a TextureView,
    pub shore_distance: &'a TextureView,
    pub collision_height: &'a TextureView,
}
pub struct OceanWaterLine {
    texture: Texture,
    view: TextureView,
    pipeline: RenderPipeline,
    parameters: UniformBuffer<WaterLineParameters>,
    surface: Buffer,
    clamp: Sampler,
    wrap: Sampler,
    binding: Option<([TextureViewId; 3], BindGroup)>,
}
impl OceanWaterLine {
    /// The water task supplies its actual mask extent and source program state.
    pub fn new(
        device: &RenderDevice,
        extent: UVec2,
        state: crate::render_state::RenderState,
    ) -> Result<Self, String> {
        if extent.min_element() == 0 {
            return Err("water-line target is empty".into());
        }
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("ocean water-line mask"),
            source: ShaderSource::Wgsl(
                format!(
                    "{}\n{}\n{}",
                    include_str!("../../../assets/shaders/dh2_ocean_noise.wgsl"),
                    include_str!("../../../assets/shaders/dh2_ocean_surface.wgsl"),
                    include_str!("../../../assets/shaders/dh2_ocean_water_line.wgsl")
                )
                .into(),
            ),
        });
        let target = crate::render_state::blend_state(&state).targets[0]
            .ok_or("water-line program has no colour output")?;
        let (front_face, cull_mode) = super::pipeline_state::mesh_raster(&state, false);
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("ocean water-line mask"),
            layout: None,
            vertex: RawVertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: default(),
                buffers: &[RawVertexBufferLayout {
                    array_stride: 20,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &[VertexAttribute {
                        format: VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    }],
                }],
            },
            fragment: Some(RawFragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: default(),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::R8Unorm,
                    blend: super::pipeline_state::blend_target(target)?,
                    write_mask: ColorWrites::from_bits_truncate(u32::from(target.write_mask))
                        & ColorWrites::RED,
                })],
            }),
            primitive: PrimitiveState {
                front_face,
                cull_mode,
                ..default()
            },
            depth_stencil: None,
            multisample: default(),
            multiview_mask: None,
            cache: None,
        });
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("depth water line"),
            size: Extent3d {
                width: extent.x,
                height: extent.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::R8Unorm,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&default());
        let sampler = |mode| {
            device.create_sampler(&SamplerDescriptor {
                address_mode_u: mode,
                address_mode_v: mode,
                address_mode_w: mode,
                min_filter: FilterMode::Linear,
                mag_filter: FilterMode::Linear,
                ..default()
            })
        };
        Ok(Self {
            texture,
            view,
            pipeline,
            parameters: default(),
            surface: device.create_buffer(&BufferDescriptor {
                label: Some("water-line surface parameters"),
                size: 112,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            clamp: sampler(AddressMode::ClampToEdge),
            wrap: sampler(AddressMode::Repeat),
            binding: None,
        })
    }
    pub fn texture(&self) -> &Texture {
        &self.texture
    }
    /// One publication per view/frame, before its two marking draws. The
    /// returned mask borrows this owner until that matching consumer records.
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: WaterLineParameters,
        surface: &OceanSurfaceParameters,
        inputs: WaterLineInputs,
        vertices: &Buffer,
        range: std::ops::Range<u32>,
    ) -> Result<&TextureView, String> {
        let vp = parameters.viewport;
        if !vp.is_finite()
            || vp.x < 0.
            || vp.y < 0.
            || vp.z <= 0.
            || vp.w <= 0.
            || vp.x + vp.z > self.texture.width() as f32
            || vp.y + vp.w > self.texture.height() as f32
        {
            return Err("water-line viewport exceeds its mask target".into());
        }
        self.parameters.set(parameters);
        self.parameters.write_buffer(device, queue);
        queue.write_buffer(&self.surface, 0, &surface.uniform_bytes());
        let ids = [
            inputs.amplitude_mask.id(),
            inputs.shore_distance.id(),
            inputs.collision_height.id(),
        ];
        if self.binding.as_ref().is_none_or(|old| old.0 != ids) {
            let mut entries = vec![
                BindGroupEntry {
                    binding: 0,
                    resource: self.parameters.binding().unwrap(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: self.surface.as_entire_binding(),
                },
            ];
            for (i, view) in [
                inputs.amplitude_mask,
                inputs.shore_distance,
                inputs.collision_height,
            ]
            .into_iter()
            .enumerate()
            {
                entries.push(BindGroupEntry {
                    binding: i as u32 + 2,
                    resource: BindingResource::TextureView(view),
                });
            }
            entries.extend([
                BindGroupEntry {
                    binding: 5,
                    resource: BindingResource::Sampler(&self.clamp),
                },
                BindGroupEntry {
                    binding: 6,
                    resource: BindingResource::Sampler(&self.wrap),
                },
            ]);
            self.binding = Some((
                ids,
                device.create_bind_group(
                    Some("water-line wave inputs"),
                    &self.pipeline.get_bind_group_layout(0).into(),
                    &entries,
                ),
            ));
        }
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("ocean wave-height water line"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &self.view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(bevy::color::LinearRgba::BLACK.into()),
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding.as_ref().unwrap().1, &[]);
        pass.set_viewport(vp.x, vp.y, vp.z, vp.w, 0., 1.);
        pass.set_vertex_buffer(0, *vertices.slice(..));
        pass.draw(range, 0..1);
        drop(pass);
        Ok(&self.view)
    }
}

impl OceanWaterLine {
    /// Connected ocean branch: generate the real wave-aware mask, then mark
    /// both supplied alpha-depth targets in the same command stream.
    pub fn record_underwater(
        &mut self,
        stencil: &mut UnderwaterStencilPass,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: WaterLineParameters,
        surface: &OceanSurfaceParameters,
        inputs: WaterLineInputs,
        targets: [UnderwaterStencilTarget; 2],
        vertices: &Buffer,
        range: std::ops::Range<u32>,
    ) -> Result<(), String> {
        let mask = self.record(
            device,
            queue,
            encoder,
            parameters,
            surface,
            inputs,
            vertices,
            range.clone(),
        )?;
        stencil.record(
            device,
            queue,
            encoder,
            UnderwaterStencilSource::Ocean(mask),
            targets,
            vertices,
            range,
        )
    }
}

#[cfg(test)]
#[path = "ocean_water_line/tests.rs"]
mod tests;
