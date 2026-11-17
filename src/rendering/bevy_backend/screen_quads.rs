//! Generic19f710/31da70 screen-quad material submission. Images are borrowed
//! from the same ParticleImagesGpu residency owner as other FX programs.
use crate::{
    assets::fx::screen_quad::ScreenQuadStage,
    rendering::screen_quad::{Program, ScreenQuadMaterial},
};
use bevy::{
    prelude::default,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};
type Frame = crate::gameplay::effects::screen_quad::Frame;
struct Material {
    source: Weak<ScreenQuadMaterial>,
    views: [TextureView; 3],
    binding: Option<((TextureViewId, BufferId), BindGroup)>,
    state: crate::render_state::RenderState,
    program: Program,
}
struct Pass {
    layout: BindGroupLayout,
    uniform: Buffer,
    stride: usize,
    bytes: Vec<u8>,
    wrap: Sampler,
    clamp: Sampler,
    shader: ShaderModule,
    pipelines: HashMap<(Program, crate::render_state::RenderState, TextureFormat), RenderPipeline>,
    materials: HashMap<String, Material>,
}
#[derive(Default)]
pub(super) struct ScreenQuads {
    pass: Option<Pass>,
    prepared_source: Weak<crate::rendering::particles::world_sources::WorldFxSources>,
    prepared_formats: Option<(TextureFormat, TextureFormat)>,
}
fn draw_state(
    mut state: crate::render_state::RenderState,
    stage: ScreenQuadStage,
) -> crate::render_state::RenderState {
    //31da70 copies the material then changes only these actual packed words.
    if matches!(
        stage,
        ScreenQuadStage::MotionVectorBegin | ScreenQuadStage::MotionVectorEnd
    ) {
        state[4] = (state[4] & 0xff) | 0xe80e8000;
        state[5] = (state[5] & 0xff1010ef) | 0x10102f;
    } else if stage == ScreenQuadStage::ShadowWalkEnd {
        state[5] &= 0xffffffef;
    }
    state
}
impl ScreenQuads {
    pub fn source_pending(
        &self,
        source: &Arc<crate::rendering::particles::world_sources::WorldFxSources>,
        colour_format: TextureFormat,
        depth_format: TextureFormat,
    ) -> bool {
        self.prepared_source.as_ptr() != Arc::as_ptr(source)
            || self.prepared_formats != Some((colour_format, depth_format))
    }
    pub fn prepare_source(
        &mut self,
        images: &mut super::particle_images::ParticleImagesGpu,
        device: &RenderDevice,
        queue: &RenderQueue,
        source: &Arc<crate::rendering::particles::world_sources::WorldFxSources>,
        format: TextureFormat,
        depth_format: TextureFormat,
    ) -> Result<(), String> {
        self.prepare(images, device, queue, source.quad_material_stages(), format)?;
        self.prepared_source = Arc::downgrade(source);
        self.prepared_formats = Some((format, depth_format));
        Ok(())
    }
    pub fn retire_unused(&mut self) {
        if let Some(pass) = &mut self.pass {
            pass.materials
                .retain(|_, material| material.source.strong_count() != 0);
        }
    }
    pub fn prepare<'a>(
        &mut self,
        images: &mut super::particle_images::ParticleImagesGpu,
        device: &RenderDevice,
        queue: &RenderQueue,
        materials: impl Iterator<Item = (ScreenQuadStage, &'a Arc<ScreenQuadMaterial>)>,
        format: TextureFormat,
    ) -> Result<(), String> {
        let mut materials = materials.peekable();
        if materials.peek().is_none() {
            return Ok(());
        }
        if self.pass.is_none() {
            self.pass = Some(Pass::new(device)?);
        }
        let pass = self.pass.as_mut().unwrap();
        for (stage, source) in materials {
            let current = pass.materials.get(&source.name).is_some_and(|material| {
                material.source.as_ptr() == Arc::as_ptr(source)
                    && !source
                        .images
                        .values()
                        .any(|image| images.changed_paths.contains(&image.path))
            });
            if !current {
                let views = ["fx/disto1", "fx/disto2", "fx/opacity"]
                    .map(|name| -> Result<_, String> {
                        let image = source.images.get(name).ok_or("quad image binding absent")?;
                        images.acquire(
                            device,
                            queue,
                            image,
                            &TextureDescriptor {
                                label: Some(&source.name),
                                size: Extent3d {
                                    width: image.info.width,
                                    height: image.info.height,
                                    depth_or_array_layers: 1,
                                },
                                mip_level_count: image.info.mip_count,
                                sample_count: 1,
                                dimension: TextureDimension::D2,
                                format: super::image_formats::gpu_image_format(image.info.format),
                                usage: TextureUsages::TEXTURE_BINDING,
                                view_formats: &[],
                            },
                            TextureDataOrder::LayerMajor,
                        )
                    })
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>()?
                    .try_into()
                    .map_err(|_| "quad image count")?;
                pass.materials.insert(
                    source.name.clone(),
                    Material {
                        source: Arc::downgrade(source),
                        views,
                        binding: None,
                        state: source.state,
                        program: source.program,
                    },
                );
            }
            let target = if matches!(
                stage,
                ScreenQuadStage::MotionVectorBegin | ScreenQuadStage::MotionVectorEnd
            ) {
                TextureFormat::Rgba16Float
            } else {
                format
            };
            let state = draw_state(source.state, stage);
            let key = (source.program, state, target);
            if !pass.pipelines.contains_key(&key) {
                let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
                    label: Some("screen quad"),
                    bind_group_layouts: &[Some(&pass.layout)],
                    immediate_size: 0,
                });
                let pipeline = super::screen_effects::passes::pipeline(
                    device,
                    &pass.shader,
                    Some(&layout),
                    "screen quad",
                    "screen_distortion_vertex",
                    if source.program == Program::Blend {
                        "screen_blend_fragment"
                    } else {
                        "screen_distortion_fragment"
                    },
                    target,
                    &state,
                )?;
                pass.pipelines.insert(key, pipeline);
            }
        }
        Ok(())
    }
    pub fn prepare_frame(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        frame: &Frame,
        stage: ScreenQuadStage,
        depth: &TextureView,
        exposure: &Buffer,
        extent: [u32; 2],
        shader_time: f32,
        local_time: f32,
        inverse_z: [f32; 3],
    ) -> Result<(), String> {
        if !frame
            .iter()
            .any(|draw| draw.asset.stage == stage && draw.asset.material.is_some())
        {
            return Ok(());
        }
        let pass = self
            .pass
            .as_mut()
            .ok_or("quad materials await GPU preparation")?;
        let bytes = &mut pass.bytes;
        let stage_base = stage as usize * 32 * pass.stride;
        for (index, draw) in frame
            .iter()
            .enumerate()
            .filter(|(_, draw)| draw.asset.stage == stage)
        {
            let Some(name) = &draw.asset.material else {
                continue;
            };
            let gpu = pass
                .materials
                .get_mut(name)
                .ok_or_else(|| format!("quad GPU material absent: {name}"))?;
            let material = gpu
                .source
                .upgrade()
                .ok_or("quad material owner retired before draw")?;
            let mut offset = index * pass.stride;
            let mut word = |value: u32| {
                bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
                offset += 4;
            };
            for value in material.layers.iter().flat_map(|layer| {
                layer
                    .tiling_speed
                    .into_iter()
                    .chain(layer.strength)
                    .chain(layer.rotation_seed)
            }) {
                word(value.to_bits());
            }
            for value in material.options {
                word(value);
            }
            for value in [extent[0] as f32, extent[1] as f32, local_time, draw.blend]
                .into_iter()
                .chain(material.clamp_colour)
                .chain(material.depth)
                .chain([
                    inverse_z[0],
                    inverse_z[1],
                    inverse_z[2],
                    material.alpha_test,
                ])
                .chain(material.colour)
                .chain([
                    shader_time,
                    if stage == ScreenQuadStage::AfterToneMap {
                        1.
                    } else {
                        0.
                    },
                    0.,
                    0.,
                ])
            {
                word(value.to_bits());
            }
            let key = (depth.id(), exposure.id());
            if gpu.binding.as_ref().is_none_or(|binding| binding.0 != key) {
                gpu.binding = Some((
                    key,
                    device.create_bind_group(
                        Some("screen quad inputs"),
                        &pass.layout,
                        &[
                            BindGroupEntry {
                                binding: 0,
                                resource: BindingResource::Buffer(BufferBinding {
                                    buffer: &pass.uniform,
                                    offset: 0,
                                    size: BufferSize::new(256),
                                }),
                            },
                            BindGroupEntry {
                                binding: 1,
                                resource: BindingResource::TextureView(&gpu.views[0]),
                            },
                            BindGroupEntry {
                                binding: 2,
                                resource: BindingResource::TextureView(&gpu.views[1]),
                            },
                            BindGroupEntry {
                                binding: 3,
                                resource: BindingResource::TextureView(&gpu.views[2]),
                            },
                            BindGroupEntry {
                                binding: 4,
                                resource: BindingResource::Sampler(&pass.wrap),
                            },
                            BindGroupEntry {
                                binding: 5,
                                resource: BindingResource::TextureView(depth),
                            },
                            BindGroupEntry {
                                binding: 6,
                                resource: BindingResource::Sampler(&pass.clamp),
                            },
                            BindGroupEntry {
                                binding: 7,
                                resource: exposure.as_entire_binding(),
                            },
                        ],
                    ),
                ));
            }
        }
        queue.write_buffer(&pass.uniform, stage_base as u64, &bytes);
        Ok(())
    }
    pub fn visit_draws<'a>(
        &'a self,
        frame: &Frame,
        stage: ScreenQuadStage,
        format: TextureFormat,
        mut submit: impl FnMut(&'a RenderPipeline, &'a BindGroup, u32),
    ) -> Result<(), String> {
        if !frame
            .iter()
            .any(|draw| draw.asset.stage == stage && draw.asset.material.is_some())
        {
            return Ok(());
        }
        let pass = self.pass.as_ref().ok_or("quad GPU owner absent")?;
        let stage_base = stage as usize * 32 * pass.stride;
        for (index, draw) in frame
            .iter()
            .enumerate()
            .filter(|(_, draw)| draw.asset.stage == stage)
        {
            let Some(name) = &draw.asset.material else {
                continue;
            };
            let material = pass.materials.get(name).ok_or("quad material missing")?;
            let pipeline = pass
                .pipelines
                .get(&(material.program, draw_state(material.state, stage), format))
                .ok_or("quad pipeline awaits preparation")?;
            submit(
                pipeline,
                &material.binding.as_ref().unwrap().1,
                (stage_base + index * pass.stride) as u32,
            );
        }
        Ok(())
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        frame: &Frame,
        stage: ScreenQuadStage,
        target: &TextureView,
        format: TextureFormat,
        depth: &TextureView,
        exposure: &Buffer,
        extent: [u32; 2],
        shader_time: f32,
        local_time: f32,
        inverse_z: [f32; 3],
    ) -> Result<(), String> {
        if !frame
            .iter()
            .any(|draw| draw.asset.stage == stage && draw.asset.material.is_some())
        {
            return Ok(());
        }
        self.prepare_frame(
            device,
            queue,
            frame,
            stage,
            depth,
            exposure,
            extent,
            shader_time,
            local_time,
            inverse_z,
        )?;
        let mut drawpass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("authored screen quads"),
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
        self.visit_draws(frame, stage, format, |pipeline, binding, offset| {
            drawpass.set_pipeline(pipeline);
            drawpass.set_bind_group(0, binding, &[offset]);
            drawpass.draw(0..3, 0..1);
        })
    }
}
impl Pass {
    fn new(device: &RenderDevice) -> Result<Self, String> {
        let stride = 256u64.next_multiple_of(u64::from(
            device.limits().min_uniform_buffer_offset_alignment,
        )) as usize;
        let uniform = device.create_buffer(&BufferDescriptor {
            label: Some("screen quad parameters"),
            size: (stride * 32 * 5) as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut entries = vec![BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::VERTEX_FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: BufferSize::new(256),
            },
            count: None,
        }];
        entries.extend((1..=3).map(|binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: true },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }));
        for binding in [4, 6] {
            entries.push(BindGroupLayoutEntry {
                binding,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            });
        }
        entries.push(BindGroupLayoutEntry {
            binding: 5,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Depth,
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        });
        entries.push(BindGroupLayoutEntry {
            binding: 7,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: BufferSize::new(32),
            },
            count: None,
        });
        let layout = device.create_bind_group_layout(Some("screen quad inputs"), &entries);
        let sampler = |address_mode| {
            device.create_sampler(&SamplerDescriptor {
                address_mode_u: address_mode,
                address_mode_v: address_mode,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                mipmap_filter: MipmapFilterMode::Linear,
                ..default()
            })
        };
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("screen quad programs"),
            source: ShaderSource::Wgsl(
                include_str!("../../../assets/shaders/dh2_screen_quad.wgsl").into(),
            ),
        });
        Ok(Self {
            layout,
            uniform,
            stride,
            bytes: vec![0; stride * 32],
            wrap: sampler(AddressMode::Repeat),
            clamp: sampler(AddressMode::ClampToEdge),
            shader,
            pipelines: HashMap::new(),
            materials: HashMap::new(),
        })
    }
}
