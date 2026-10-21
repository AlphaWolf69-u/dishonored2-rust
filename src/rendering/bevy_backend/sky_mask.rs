//! Sky/cloud MRT pipeline: shared colour code plus the authored fog mask.
//! State comes from 293510 -> 2a8140 -> 2a09f0, never from the raw RPD alone.
use bevy::{
    mesh::VertexBufferLayout,
    prelude::*,
    render::{render_resource::*, renderer::RenderDevice},
};

/// Material inputs shared by sky colour and fog-mask rendering. Image slots
/// follow diffuse, opacity, density, back density, fog depth; unused cloud
/// slots on a sky material stay absent rather than requesting extra images.
#[derive(Asset, TypePath, Clone)]
pub struct SkyMaterial {
    pub sky: [Vec4; 7],
    pub depth_offset: f32,
    pub depth_uv: Vec4,
    pub clouds: bool,
    pub project_far: bool,
    pub state: crate::render_state::RenderState,
    pub images: [Option<Handle<Image>>; 5],
}
impl SkyMaterial {
    pub fn image_mut(&mut self, role: usize) -> &mut Option<Handle<Image>> {
        let slot = match role {
            0 => 0,
            29..=31 => role - 28,
            33 => 4,
            _ => panic!("invalid sky image role {role}"),
        };
        &mut self.images[slot]
    }
}

#[derive(Component, Clone)]
pub struct SkyMesh {
    pub material: Handle<SkyMaterial>,
    pub order: usize,
}
#[derive(Component)]
pub struct SkyInstance {
    pub mesh: bevy::asset::AssetId<Mesh>,
    pub material: Handle<SkyMaterial>,
    pub order: usize,
    pub world_from_local: Mat4,
}
impl bevy::render::sync_component::SyncComponent for SkyMesh {
    type Target = SkyInstance;
}
impl bevy::render::extract_component::ExtractComponent for SkyMesh {
    type QueryData = (
        &'static Mesh3d,
        &'static GlobalTransform,
        &'static ViewVisibility,
        &'static SkyMesh,
    );
    type QueryFilter = ();
    type Out = SkyInstance;
    fn extract_component(
        (mesh, transform, visible, material): bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>,
    ) -> Option<Self::Out> {
        visible.get().then(|| SkyInstance {
            mesh: mesh.0.id(),
            material: material.material.clone(),
            order: material.order,
            world_from_local: transform.to_matrix(),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct SkyView {
    pub clip_from_world: Mat4,
    pub time: f32,
    pub exposure: f32,
    pub padding: Vec2,
}

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct SkySurface {
    pub sky: [Vec4; 7],
    pub world_from_local: Mat4,
    pub fog: Vec4,
    pub depth_uv: Vec4,
    pub flags: UVec4,
}

#[derive(Resource)]
pub struct SkyProgram {
    module: ShaderModule,
    pub view_layout: BindGroupLayout,
    pub material_layout: BindGroupLayout,
}
impl SkyProgram {
    pub fn new(device: &RenderDevice) -> Self {
        // The raw pass supplies the same named inputs explicitly rather than
        // importing Bevy's forward-material/view bind groups.
        let colour = include_str!("../../../assets/shaders/dh2_sky.wgsl")
            .lines()
            .filter(|line| {
                !line.starts_with("#import ") && !line.starts_with("#define_import_path ")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("sky colour and fog density MRT"),
            source: ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    include_str!("../../../assets/shaders/dh2_sky_mask.wgsl"),
                    colour
                )
                .into(),
            ),
        });
        let view_layout = device.create_bind_group_layout(
            Some("sky view and surface"),
            &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX_FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(SkyView::min_size()),
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::VERTEX_FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: Some(SkySurface::min_size()),
                    },
                    count: None,
                },
            ],
        );
        let mut entries = (0..5)
            .map(|binding| BindGroupLayoutEntry {
                binding,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            })
            .collect::<Vec<_>>();
        entries.push(BindGroupLayoutEntry {
            binding: 5,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Sampler(SamplerBindingType::Filtering),
            count: None,
        });
        let material_layout =
            device.create_bind_group_layout(Some("sky textures and sampler"), &entries);
        Self {
            module,
            view_layout,
            material_layout,
        }
    }
}
impl FromWorld for SkyProgram {
    fn from_world(world: &mut World) -> Self {
        Self::new(world.resource::<RenderDevice>())
    }
}

pub struct SkyMaskPipeline {
    pub pipeline: RenderPipeline,
    pub view_layout: BindGroupLayout,
    pub material_layout: BindGroupLayout,
}

impl SkyMaskPipeline {
    pub fn new(
        device: &RenderDevice,
        hdr: TextureFormat,
        depth: TextureFormat,
        vertex: &VertexBufferLayout,
        state: crate::render_state::RenderState,
        indices_reversed: bool,
    ) -> Result<Self, String> {
        let program = SkyProgram::new(device);
        Self::with_program(
            device,
            hdr,
            depth,
            vertex,
            state,
            indices_reversed,
            &program,
        )
    }
    pub fn with_program(
        device: &RenderDevice,
        hdr: TextureFormat,
        depth: TextureFormat,
        vertex: &VertexBufferLayout,
        state: crate::render_state::RenderState,
        indices_reversed: bool,
        program: &SkyProgram,
    ) -> Result<Self, String> {
        let prepared = crate::render_state::sky_cloud_draw_state(state);
        let blend = crate::render_state::blend_state(&prepared);
        let (front_face, cull_mode) =
            super::shadow_pipeline::shadow_mesh_raster(&prepared, indices_reversed);
        let module = &program.module;
        let view_layout = program.view_layout.clone();
        let material_layout = program.material_layout.clone();
        let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("sky MRT"),
            bind_group_layouts: &[Some(&view_layout), Some(&material_layout)],
            immediate_size: 0,
        });
        let targets = [
            Some(ColorTargetState {
                format: hdr,
                blend: super::pipeline_state::blend_target(blend.targets[0].unwrap())?,
                write_mask: ColorWrites::from_bits_truncate(u32::from(
                    blend.targets[0].unwrap().write_mask,
                )),
            }),
            Some(ColorTargetState {
                format: TextureFormat::R8Unorm,
                blend: super::pipeline_state::blend_target(blend.targets[1].unwrap())?,
                write_mask: ColorWrites::RED,
            }),
        ];
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("sky colour and fog density MRT"),
            layout: Some(&layout),
            vertex: RawVertexState {
                module: &module,
                entry_point: Some("sky_mask_vertex"),
                compilation_options: default(),
                buffers: &[RawVertexBufferLayout {
                    array_stride: vertex.array_stride,
                    step_mode: vertex.step_mode,
                    attributes: &vertex.attributes,
                }],
            },
            fragment: Some(RawFragmentState {
                module: &module,
                entry_point: Some("sky_mask_fragment"),
                compilation_options: default(),
                targets: &targets,
            }),
            primitive: PrimitiveState {
                front_face,
                cull_mode,
                ..default()
            },
            depth_stencil: Some(super::shadow_pipeline::depth_stencil_gpu(&prepared, depth)?),
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            pipeline,
            view_layout,
            material_layout,
        })
    }
}

/// Texture bindings follow the normal image preparation and streaming lifetime.
/// No duplicate image uploads or per-frame material bind-group allocations.
pub struct GpuSkyMaterial {
    pub source: SkyMaterial,
    pub binding: BindGroup,
}
impl bevy::render::render_asset::RenderAsset for GpuSkyMaterial {
    type SourceAsset = SkyMaterial;
    type Param = (
        bevy::ecs::system::lifetimeless::SRes<RenderDevice>,
        bevy::ecs::system::lifetimeless::SRes<
            bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>,
        >,
        bevy::ecs::system::lifetimeless::SRes<SkyProgram>,
    );
    fn prepare_asset(
        source: SkyMaterial,
        _: bevy::asset::AssetId<SkyMaterial>,
        (device, images, program): &mut bevy::ecs::system::SystemParamItem<Self::Param>,
        _: Option<&Self>,
    ) -> Result<Self, bevy::render::render_asset::PrepareAssetError<SkyMaterial>> {
        use bevy::render::render_asset::PrepareAssetError;
        let Some(diffuse) = source.images[0]
            .as_ref()
            .and_then(|handle| images.get(handle))
        else {
            return Err(PrepareAssetError::RetryNextUpdate(source));
        };
        let mut views = Vec::with_capacity(5);
        for (slot, handle) in source.images.iter().enumerate() {
            let image = if !source.clouds && (1..4).contains(&slot) {
                // The sky branch does not sample cloud-only bindings.
                diffuse
            } else {
                let Some(image) = handle.as_ref().and_then(|handle| images.get(handle)) else {
                    return Err(PrepareAssetError::RetryNextUpdate(source));
                };
                image
            };
            views.push(image.texture_view.clone());
        }
        let mut entries = views
            .iter()
            .enumerate()
            .map(|(slot, view)| BindGroupEntry {
                binding: slot as u32,
                resource: BindingResource::TextureView(view),
            })
            .collect::<Vec<_>>();
        // Streamed images use the installed trilinear wrap sampler (maxLOD13).
        entries.push(BindGroupEntry {
            binding: 5,
            resource: BindingResource::Sampler(&diffuse.sampler),
        });
        let binding = device.create_bind_group(
            Some("sky material images"),
            &program.material_layout,
            &entries,
        );
        Ok(Self { source, binding })
    }
}
