//! Per-view RGBA16F luminance pyramid, shared by exposure and post-processing.
use bevy::{
    math::{UVec2, Vec4},
    prelude::{Entity, Resource},
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

#[derive(Resource)]
pub struct LuminancePipelines {
    layout: BindGroupLayout,
    programs: [RenderPipeline; 2],
    linear: Sampler,
}
impl LuminancePipelines {
    pub fn new(device: &RenderDevice) -> Self {
        use binding_types::*;
        let layout = device.create_bind_group_layout(
            Some("luminance pyramid inputs"),
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    uniform_buffer::<Vec4>(false),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        );
        let program = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("luminance downsample"),
            source: ShaderSource::Wgsl(
                include_str!("../../../assets/shaders/dh2_luminance.wgsl").into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("luminance downsample"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let programs = ["luminance_first", "luminance_standard"].map(|entry| {
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: RawVertexState {
                    module: &program,
                    entry_point: Some("luminance_vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(RawFragmentState {
                    module: &program,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(ColorTargetState {
                        format: TextureFormat::Rgba16Float,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        });
        Self {
            layout,
            programs,
            linear: device.create_sampler(&SamplerDescriptor {
                label: Some("luminance linear clamp"),
                min_filter: FilterMode::Linear,
                mag_filter: FilterMode::Linear,
                ..Default::default()
            }),
        }
    }
}
struct Level {
    extent: UVec2,
    view: TextureView,
    parameters: UniformBuffer<Vec4>,
    bindings: Vec<(TextureViewId, BindGroup)>,
}
#[derive(Resource, Default)]
pub struct LuminanceViews(std::collections::HashMap<Entity, LuminancePyramid>);
impl LuminanceViews {
    pub fn prepare(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        entity: Entity,
        full: UVec2,
        quality: i32,
    ) -> Result<&mut LuminancePyramid, String> {
        if self
            .0
            .get(&entity)
            .is_none_or(|v| !v.matches(full, quality))
        {
            self.0
                .insert(entity, LuminancePyramid::new(device, queue, full, quality)?);
        }
        Ok(self.0.get_mut(&entity).unwrap())
    }
    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut LuminancePyramid> {
        self.0.get_mut(&entity)
    }
    pub(super) fn retain(&mut self, mut active: impl FnMut(Entity) -> bool) {
        self.0.retain(|entity, _| active(*entity));
        for pyramid in self.0.values_mut() {
            pyramid.begin_frame();
        }
    }
}
pub struct LuminancePyramid {
    full: UVec2,
    quality: i32,
    texture: Texture,
    levels: Vec<Level>,
    exposure: Option<u32>,
    source: Option<TextureViewId>,
    generated: usize,
    #[cfg(test)]
    draws: u64,
}
impl LuminancePyramid {
    pub fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        full: UVec2,
        quality: i32,
    ) -> Result<Self, String> {
        let extents = crate::rendering::luminance::pyramid_extents(full.to_array(), quality)?;
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("luminance pyramid"),
            size: Extent3d {
                width: extents[0][0],
                height: extents[0][1],
                depth_or_array_layers: 1,
            },
            mip_level_count: extents.len() as u32,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba16Float,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let levels = extents
            .into_iter()
            .enumerate()
            .map(|(index, extent)| {
                let extent = UVec2::from_array(extent);
                let view = texture.create_view(&TextureViewDescriptor {
                    base_mip_level: index as u32,
                    mip_level_count: Some(1),
                    ..Default::default()
                });
                let mut parameters = UniformBuffer::from(Vec4::new(
                    1.0 / extent.x as f32,
                    1.0 / extent.y as f32,
                    1.0,
                    0.0,
                ));
                parameters.write_buffer(device, queue);
                Level {
                    extent,
                    view,
                    parameters,
                    bindings: Vec::with_capacity(2),
                }
            })
            .collect();
        Ok(Self {
            full,
            quality,
            texture,
            levels,
            exposure: None,
            source: None,
            generated: 0,
            #[cfg(test)]
            draws: 0,
        })
    }
    pub fn matches(&self, full: UVec2, quality: i32) -> bool {
        self.full == full && self.quality == quality
    }
    pub fn texture(&self) -> &Texture {
        &self.texture
    }
    pub fn view(&self, index: usize) -> Option<&TextureView> {
        self.levels.get(index).map(|v| &v.view)
    }
    pub fn extent(&self, index: usize) -> Option<UVec2> {
        self.levels.get(index).map(|v| v.extent)
    }
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }
    pub fn begin_frame(&mut self) {
        self.generated = 0;
        self.source = None;
    }
    pub fn generated_levels(&self) -> usize {
        self.generated
    }
    #[cfg(test)]
    pub fn draw_count(&self) -> u64 {
        self.draws
    }
    /// Exposure and display share this image. Generate only the missing prefix
    /// of the current source, never read last frame's remaining mip levels.
    pub fn ensure_levels(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        pipelines: &LuminancePipelines,
        source: &TextureView,
        camera_exposure: f32,
        end: usize,
    ) {
        if self.source != Some(source.id()) || self.exposure != Some(camera_exposure.to_bits()) {
            self.generated = 0;
        }
        let end = end.min(self.levels.len());
        if self.generated < end {
            self.record(
                device,
                queue,
                encoder,
                pipelines,
                source,
                camera_exposure,
                self.generated..end,
            );
        }
    }
    /// Exposure generates 0..2. Bloom can continue at 2..N using those same
    /// submitted images, without generating the first levels a second time.
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        pipelines: &LuminancePipelines,
        source: &TextureView,
        camera_exposure: f32,
        levels: std::ops::Range<usize>,
    ) {
        if levels.start == 0 {
            self.generated = 0;
            self.source = Some(source.id());
        }
        if self.exposure != Some(camera_exposure.to_bits()) {
            let first = &mut self.levels[0];
            first.parameters.set(Vec4::new(
                1.0 / first.extent.x as f32,
                1.0 / first.extent.y as f32,
                camera_exposure,
                0.0,
            ));
            first.parameters.write_buffer(device, queue);
            self.exposure = Some(camera_exposure.to_bits());
        }
        for index in levels.start..levels.end.min(self.levels.len()) {
            let input = if index == 0 {
                source.clone()
            } else {
                self.levels[index - 1].view.clone()
            };
            let level = &mut self.levels[index];
            let slot = if let Some(slot) = level.bindings.iter().position(|v| v.0 == input.id()) {
                slot
            } else {
                let group = device.create_bind_group(
                    Some("luminance level"),
                    &pipelines.layout,
                    &BindGroupEntries::sequential((
                        level.parameters.binding().unwrap(),
                        &input,
                        &pipelines.linear,
                    )),
                );
                if level.bindings.len() == 2 {
                    level.bindings.remove(0);
                }
                level.bindings.push((input.id(), group));
                level.bindings.len() - 1
            };
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("luminance downsample"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &level.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Default::default()),
                        store: StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipelines.programs[usize::from(index != 0)]);
            pass.set_bind_group(0, &level.bindings[slot].1, &[]);
            pass.draw(0..3, 0..1);
            #[cfg(test)]
            {
                self.draws += 1;
            }
            if index == self.generated {
                self.generated += 1;
            }
        }
    }
}

#[cfg(test)]
#[path = "luminance/tests.rs"]
mod tests;
