//! HBAO+ 3 producer using the game's full-resolution depth and shared AO/shadow target.
//! Native sixteen coarse slices are packed in a 4x4 R8 atlas on the host; the
//! deinterleaved depth, jitter, samples and quantization remain independent.
use crate::rendering::ambient_occlusion::{
    hbao_jitters, AmbientOcclusionParameters, HbaoParameters,
};
use crate::tiled_lighting_gpu::TlfPipelines;
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

#[path = "ambient_occlusion/dwao.rs"]
mod dwao;
use dwao::DwaoPass;

struct Stage {
    pipeline: RenderPipeline,
    inputs: BindGroupLayout,
}
pub struct HbaoPass {
    extent: [u32; 2],
    quarter: [u32; 2],
    _linear: Texture,
    linear_view: TextureView,
    _normals: Texture,
    normals_view: TextureView,
    _depth: Texture,
    _depth_view: TextureView,
    depth_slices: Vec<TextureView>,
    _coarse: Texture,
    coarse_view: TextureView,
    constants: Buffer,
    constant_words: Option<[u32; 56]>,
    common: BindGroup,
    linear_stage: Stage,
    deinterleave: [Stage; 2],
    normal_stage: Stage,
    coarse_stage: Stage,
    reinterleave: Stage,
    linear_binding: Option<(TextureViewId, BindGroup)>,
    deinterleave_binding: BindGroup,
    normal_binding: BindGroup,
    coarse_binding: BindGroup,
    reinterleave_binding: BindGroup,
}
impl HbaoPass {
    pub fn new(device: &RenderDevice, extent: [u32; 2]) -> Result<Self, String> {
        if extent.contains(&0) {
            return Err("AO requires a nonempty full view".into());
        }
        let quarter = extent.map(|v| v.div_ceil(4));
        let make = |name, size: [u32; 2], layers, format| {
            let texture = device.create_texture(&TextureDescriptor {
                label: Some(name),
                size: Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::TEXTURE_BINDING
                    | TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&default());
            (texture, view)
        };
        let (linear, linear_view) = make("AO linear depth", extent, 1, TextureFormat::R32Float);
        let (normals, normals_view) = make(
            "AO reconstructed normals",
            extent,
            1,
            TextureFormat::Rgba8Unorm,
        );
        let (depth, depth_view) = make(
            "AO quarter depth slices",
            quarter,
            16,
            TextureFormat::R16Float,
        );
        let depth_slices = (0..16)
            .map(|layer| {
                depth.create_view(&TextureViewDescriptor {
                    dimension: Some(TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..default()
                })
            })
            .collect();
        let (coarse, coarse_view) = make(
            "AO coarse slice atlas",
            quarter.map(|v| v * 4),
            1,
            TextureFormat::R8Unorm,
        );
        let constants = device.create_buffer(&BufferDescriptor {
            label: Some("AO view constants"),
            size: 224,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let common_layout = device.create_bind_group_layout(
            Some("AO view constants"),
            &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                count: None,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(224),
                },
            }],
        );
        let common = device.create_bind_group(
            Some("AO view constants"),
            &common_layout,
            &[BindGroupEntry {
                binding: 0,
                resource: constants.as_entire_binding(),
            }],
        );
        let texture_entry = |binding, kind, dimension| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            count: None,
            ty: BindingType::Texture {
                sample_type: kind,
                view_dimension: dimension,
                multisampled: false,
            },
        };
        let float = TextureSampleType::Float { filterable: false };
        let d2 = TextureViewDimension::D2;
        let texture = |b| texture_entry(b, float, d2);
        let sampler = |binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            count: None,
            ty: BindingType::Sampler(SamplerBindingType::NonFiltering),
        };
        let scalar_target = |format, write_mask| {
            Some(ColorTargetState {
                format,
                blend: None,
                write_mask,
            })
        };
        let stage = |name,
                     source: &str,
                     entry,
                     entries: &[BindGroupLayoutEntry],
                     targets: &[Option<ColorTargetState>]| {
            let inputs = device.create_bind_group_layout(Some(name), entries);
            let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some(name),
                bind_group_layouts: &[Some(&common_layout), Some(&inputs)],
                immediate_size: 0,
            });
            let source = format!(
                "{}\n{source}",
                include_str!("../../../assets/shaders/dh2_ao_parameters.wgsl")
            );
            let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
                label: Some(name),
                source: ShaderSource::Wgsl(source.into()),
            });
            let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some(name),
                layout: Some(&layout),
                vertex: RawVertexState {
                    module: &module,
                    entry_point: Some("ao_vertex"),
                    compilation_options: default(),
                    buffers: &[],
                },
                fragment: Some(RawFragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: default(),
                    targets,
                }),
                primitive: default(),
                depth_stencil: None,
                multisample: default(),
                multiview_mask: None,
                cache: None,
            });
            Stage { pipeline, inputs }
        };
        let linear_stage = stage(
            "AO linearization",
            include_str!("../../../assets/shaders/dh2_ao_linear_depth.wgsl"),
            "ao_linear_depth",
            &[texture_entry(0, TextureSampleType::Depth, d2)],
            &[scalar_target(TextureFormat::R32Float, ColorWrites::RED)],
        );
        let deinterleave = ["ao_deinterleave_top", "ao_deinterleave_bottom"].map(|entry| {
            stage(
                "AO depth deinterleave",
                include_str!("../../../assets/shaders/dh2_ao_deinterleave.wgsl"),
                entry,
                &[texture(0)],
                &std::array::from_fn::<_, 8, _>(|_| {
                    scalar_target(TextureFormat::R16Float, ColorWrites::RED)
                }),
            )
        });
        let normal_stage = stage(
            "AO normal reconstruction",
            include_str!("../../../assets/shaders/dh2_ao_normals.wgsl"),
            "ao_reconstruct_normal",
            &[texture(0), sampler(1)],
            &[scalar_target(TextureFormat::Rgba8Unorm, ColorWrites::ALL)],
        );
        let coarse_stage = stage(
            "AO coarse samples",
            include_str!("../../../assets/shaders/dh2_ao_coarse.wgsl"),
            "ao_coarse",
            &[
                texture_entry(0, float, TextureViewDimension::D2Array),
                texture(1),
                sampler(2),
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::FRAGMENT,
                    count: None,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: BufferSize::new(256),
                    },
                },
            ],
            &[scalar_target(TextureFormat::R8Unorm, ColorWrites::RED)],
        );
        let reinterleave = stage(
            "AO reinterleave",
            include_str!("../../../assets/shaders/dh2_ao_reinterleave.wgsl"),
            "ao_reinterleave",
            &[texture(0)],
            &[scalar_target(TextureFormat::Rgba8Unorm, ColorWrites::RED)],
        );
        let point = device.create_sampler(&SamplerDescriptor {
            label: Some("AO point clamp"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            ..default()
        });
        let jitter: Vec<_> = hbao_jitters()
            .into_iter()
            .flatten()
            .flat_map(f32::to_le_bytes)
            .collect();
        let jitter = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("AO fixed slice jitters"),
            contents: &jitter,
            usage: BufferUsages::UNIFORM,
        });
        let bind = |name, layout, entries: &[BindGroupEntry<'_>]| {
            device.create_bind_group(Some(name), layout, entries)
        };
        let view = |binding, view| BindGroupEntry {
            binding,
            resource: BindingResource::TextureView(view),
        };
        let sample = |binding| BindGroupEntry {
            binding,
            resource: BindingResource::Sampler(&point),
        };
        let deinterleave_binding = bind(
            "AO depth source",
            &deinterleave[0].inputs,
            &[view(0, &linear_view)],
        );
        let normal_binding = bind(
            "AO normal source",
            &normal_stage.inputs,
            &[view(0, &linear_view), sample(1)],
        );
        let coarse_binding = bind(
            "AO coarse sources",
            &coarse_stage.inputs,
            &[
                view(0, &depth_view),
                view(1, &normals_view),
                sample(2),
                BindGroupEntry {
                    binding: 3,
                    resource: jitter.as_entire_binding(),
                },
            ],
        );
        let reinterleave_binding = bind(
            "AO slice source",
            &reinterleave.inputs,
            &[view(0, &coarse_view)],
        );
        Ok(Self {
            extent,
            quarter,
            _linear: linear,
            linear_view,
            _normals: normals,
            normals_view,
            _depth: depth,
            _depth_view: depth_view,
            depth_slices,
            _coarse: coarse,
            coarse_view,
            constants,
            constant_words: None,
            common,
            linear_stage,
            deinterleave,
            normal_stage,
            coarse_stage,
            reinterleave,
            linear_binding: None,
            deinterleave_binding,
            normal_binding,
            coarse_binding,
            reinterleave_binding,
        })
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        hardware_depth: &TextureView,
        output: &TextureView,
        parameters: &HbaoParameters,
    ) -> Result<(), String> {
        if parameters.extent != self.extent {
            return Err("AO constants belong to another view extent".into());
        }
        if self.constant_words.as_ref() != Some(&parameters.words) {
            queue.write_buffer(&self.constants, 0, &parameters.uniform_bytes());
            self.constant_words = Some(parameters.words);
        }
        if self
            .linear_binding
            .as_ref()
            .is_none_or(|v| v.0 != hardware_depth.id())
        {
            let group = device.create_bind_group(
                Some("AO opaque depth"),
                &self.linear_stage.inputs,
                &[BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(hardware_depth),
                }],
            );
            self.linear_binding = Some((hardware_depth.id(), group));
        }
        let clear = Operations {
            load: LoadOp::Clear(default()),
            store: StoreOp::Store,
        };
        let draw = |encoder: &mut CommandEncoder,
                    stage: &Stage,
                    inputs: &BindGroup,
                    attachments: &[Option<RenderPassColorAttachment<'_>>]| {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("HBAO view producer"),
                color_attachments: attachments,
                ..default()
            });
            pass.set_pipeline(&stage.pipeline);
            pass.set_bind_group(0, &self.common, &[]);
            pass.set_bind_group(1, inputs, &[]);
            pass.draw(0..3, 0..1);
        };
        let attachment = |view, ops| {
            Some(RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops,
            })
        };
        draw(
            encoder,
            &self.linear_stage,
            &self.linear_binding.as_ref().unwrap().1,
            &[attachment(&self.linear_view, clear)],
        );
        for i in 0..2 {
            let attachments: Vec<_> = self.depth_slices[i * 8..i * 8 + 8]
                .iter()
                .map(|v| attachment(v, clear))
                .collect();
            draw(
                encoder,
                &self.deinterleave[i],
                &self.deinterleave_binding,
                &attachments,
            );
        }
        draw(
            encoder,
            &self.normal_stage,
            &self.normal_binding,
            &[attachment(&self.normals_view, clear)],
        );
        draw(
            encoder,
            &self.coarse_stage,
            &self.coarse_binding,
            &[attachment(&self.coarse_view, clear)],
        );
        draw(
            encoder,
            &self.reinterleave,
            &self.reinterleave_binding,
            &[attachment(
                output,
                Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            )],
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "ambient_occlusion/tests.rs"]
mod tests;

pub enum AmbientOcclusionPass {
    Disabled,
    Hbao(HbaoPass),
    Dwao(DwaoPass),
}
impl AmbientOcclusionPass {
    pub fn new(
        device: &RenderDevice,
        pipelines: &TlfPipelines,
        depth: &Texture,
        parameters: &AmbientOcclusionParameters,
    ) -> Result<Self, String> {
        Ok(match parameters {
            AmbientOcclusionParameters::Disabled => Self::Disabled,
            AmbientOcclusionParameters::Hbao(p) => Self::Hbao(HbaoPass::new(device, p.extent)?),
            AmbientOcclusionParameters::Dwao { .. } => {
                Self::Dwao(DwaoPass::new(device, pipelines, depth)?)
            }
        })
    }
    pub fn mode(&self) -> u8 {
        match self {
            Self::Disabled => 0,
            Self::Hbao(_) => 1,
            Self::Dwao(_) => 2,
        }
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        pipelines: &TlfPipelines,
        depth: &TextureView,
        output: &TextureView,
        parameters: &AmbientOcclusionParameters,
    ) -> Result<(), String> {
        match (self, parameters) {
            (Self::Disabled, AmbientOcclusionParameters::Disabled) => Ok(()),
            (Self::Hbao(pass), AmbientOcclusionParameters::Hbao(p)) => {
                pass.record(device, queue, encoder, depth, output, p)
            }
            (
                Self::Dwao(pass),
                AmbientOcclusionParameters::Dwao {
                    inverse_projection,
                    render_scale,
                    ..
                },
            ) => pass.record(
                queue,
                encoder,
                pipelines,
                output,
                *inverse_projection,
                *render_scale,
            ),
            _ => Err("AO parameters belong to another producer mode".into()),
        }
    }
}
