//! Persistent colour-cube history from 326af0, independent of fog shadows.
use bevy::{
    math::Vec4,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

pub const SHADER_SOURCE: &str = include_str!("../../../assets/shaders/dh2_colour_cube.wgsl");

/// 36d0c0 initializes all three RGBA8 cubes with integer-divided identity RGB.
pub fn identity_pixels() -> Vec<u8> {
    (0..32)
        .flat_map(|z| {
            (0..32).flat_map(move |y| {
                (0..32).flat_map(move |x| {
                    [x * 255 / 31, y * 255 / 31, z * 255 / 31, 255].map(|v| v as u8)
                })
            })
        })
        .collect()
}

pub struct CubeOverrides<'a> {
    pub views: [&'a TextureView; 4],
    pub weights: Vec4,
}
struct Cube {
    texture: Texture,
    view: TextureView,
}
pub struct ColourCubePass {
    cubes: [Cube; 3],
    source: usize,
    output: usize,
    previous_weight: Option<f32>,
    final_index: usize,
    layouts: [BindGroupLayout; 2],
    pipelines: [ComputePipeline; 2],
    weights: [UniformBuffer<Vec4>; 2],
    sampler: Sampler,
    bindings: [Option<(Vec<TextureViewId>, BindGroup)>; 2],
}
impl ColourCubePass {
    pub fn new(device: &RenderDevice, queue: &RenderQueue) -> Self {
        let identity = identity_pixels();
        let cubes = std::array::from_fn(|_| {
            let texture = device.create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: Some("display colour-cube history"),
                    size: Extent3d {
                        width: 32,
                        height: 32,
                        depth_or_array_layers: 32,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D3,
                    format: TextureFormat::Rgba8Unorm,
                    usage: TextureUsages::TEXTURE_BINDING
                        | TextureUsages::STORAGE_BINDING
                        | TextureUsages::COPY_SRC,
                    view_formats: &[],
                },
                TextureDataOrder::MipMajor,
                &identity,
            );
            let view = texture.create_view(&Default::default());
            Cube { texture, view }
        });
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("colour-cube blending"),
            source: ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let layouts = std::array::from_fn(|mode| {
            let mut entries = vec![BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::COMPUTE,
                count: None,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(16),
                },
            }];
            entries.extend((1..=if mode == 0 { 2 } else { 5 }).map(|binding| {
                BindGroupLayoutEntry {
                    binding,
                    visibility: ShaderStages::COMPUTE,
                    count: None,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D3,
                        multisampled: false,
                    },
                }
            }));
            entries.extend([
                BindGroupLayoutEntry {
                    binding: 6,
                    visibility: ShaderStages::COMPUTE,
                    count: None,
                    ty: BindingType::StorageTexture {
                        access: StorageTextureAccess::WriteOnly,
                        format: TextureFormat::Rgba8Unorm,
                        view_dimension: TextureViewDimension::D3,
                    },
                },
                BindGroupLayoutEntry {
                    binding: 7,
                    visibility: ShaderStages::COMPUTE,
                    count: None,
                    ty: BindingType::Sampler(SamplerBindingType::NonFiltering),
                },
            ]);
            device.create_bind_group_layout(Some("colour-cube blend inputs"), &entries)
        });
        let pipelines = std::array::from_fn(|i| {
            let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("colour-cube blend program"),
                bind_group_layouts: &[Some(&layouts[i])],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&RawComputePipelineDescriptor {
                label: Some("display colour-cube blend"),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(if i == 0 {
                    "blend_cubes"
                } else {
                    "apply_cube_overrides"
                }),
                compilation_options: Default::default(),
                cache: None,
            })
        });
        Self {
            cubes,
            source: 0,
            output: 1,
            previous_weight: None,
            final_index: 1,
            layouts,
            pipelines,
            weights: std::array::from_fn(|_| UniformBuffer::default()),
            sampler: device.create_sampler(&SamplerDescriptor {
                label: Some("colour cube point clamp"),
                ..Default::default()
            }),
            bindings: std::array::from_fn(|_| None),
        }
    }
    pub fn view(&self) -> &TextureView {
        &self.cubes[self.final_index].view
    }
    fn bind(&mut self, device: &RenderDevice, index: usize, views: &[&TextureView]) {
        let ids: Vec<_> = views.iter().map(|v| v.id()).collect();
        if self.bindings[index]
            .as_ref()
            .is_some_and(|old| old.0 == ids)
        {
            return;
        }
        let mut entries = vec![BindGroupEntry {
            binding: 0,
            resource: self.weights[index].binding().unwrap(),
        }];
        let textures = views.len() - 1;
        entries.extend(
            views[..textures]
                .iter()
                .enumerate()
                .map(|(i, v)| BindGroupEntry {
                    binding: i as u32 + 1,
                    resource: BindingResource::TextureView(v),
                }),
        );
        entries.push(BindGroupEntry {
            binding: 6,
            resource: BindingResource::TextureView(views[textures]),
        });
        entries.push(BindGroupEntry {
            binding: 7,
            resource: BindingResource::Sampler(&self.sampler),
        });
        self.bindings[index] = Some((
            ids,
            device.create_bind_group(Some("colour-cube inputs"), &self.layouts[index], &entries),
        ));
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        weight: f32,
        overrides: Option<CubeOverrides<'_>>,
    ) {
        // Initial banks are identical, so their first swap is unobservable.
        // Retain the original strict ordered comparison, including NaN cases.
        if self.previous_weight.is_some_and(|old| weight < old) {
            std::mem::swap(&mut self.source, &mut self.output);
        }
        self.previous_weight = Some(weight);
        let clamped = if weight >= 0. { weight.min(1.) } else { 0. };
        self.weights[0].set(Vec4::splat(clamped));
        self.weights[0].write_buffer(device, queue);
        let source = self.cubes[self.source].view.clone();
        let output = self.cubes[self.output].view.clone();
        self.bind(device, 0, &[&source, target, &output]);
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("blend environment colour cube"),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipelines[0]);
            pass.set_bind_group(0, &self.bindings[0].as_ref().unwrap().1, &[]);
            pass.dispatch_workgroups(4, 4, 32);
        }
        self.final_index = self.output;
        if let Some(overrides) = overrides.filter(|v| {
            ((v.weights.x + 0.) + v.weights.y + v.weights.z + v.weights.w)
                > f32::from_bits(0x358637bd)
        }) {
            self.weights[1].set(overrides.weights);
            self.weights[1].write_buffer(device, queue);
            let overridden = self.cubes[2].view.clone();
            let mut views = vec![&output];
            views.extend(overrides.views);
            views.push(&overridden);
            self.bind(device, 1, &views);
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("apply colour-cube overrides"),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipelines[1]);
            pass.set_bind_group(0, &self.bindings[1].as_ref().unwrap().1, &[]);
            pass.dispatch_workgroups(4, 4, 32);
            self.final_index = 2;
        }
    }
}

#[cfg(test)]
#[path = "colour_cube/tests.rs"]
mod tests;
